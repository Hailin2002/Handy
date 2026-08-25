# Secrets Exposure & Keychain-Fallback Durability Audit

- **Task:** task-0069 (Security Engineering leaf) — read-only audit
- **Scope:** `src-tauri/src/secret_store.rs`, `src-tauri/src/settings`, `src-tauri/src/commands`, IPC/webview boundary for API-key material, Linux keychain-fallback durability
- **Explicitly out of scope:** speech_pipeline / llm_client logging of key material (owned by task-0039). Where those files interact with the boundary (e.g. `get_azure_speech_auth`) they are assessed only as IPC exposure paths, not as logging paths.
- **Source modifications:** none. This document is the sole artifact added by this task.

All `file:line` references are against this worktree as of the audit date.

---

## 1. Plaintext key-flow trace: `hydrate_secret_maps` → webview (Gate G1)

### 1.1 The flow

| Step | Location | What happens |
|---|---|---|
| 1 | `secret_store.rs:200-230` (`hydrate_secret_maps`) | Reads every configured key from the OS keychain / fallback (`imp::get_secret`, `secret_store.rs:208,225`) and inserts the **plaintext** values into the in-memory maps `AppSettings.speech_pipeline_api_keys` and `AppSettings.post_process_api_keys`. |
| 2 | `settings/store.rs:90` | `get_settings_unlocked` calls `hydrate_secret_maps` at the end of **every** settings load — so any caller of `get_settings` receives an `AppSettings` whose maps hold live plaintext keys. |
| 3 | `commands/mod.rs:34-36` (`get_app_settings`) | The Tauri command returns `Ok(get_settings(&app))` — i.e. the hydrated `AppSettings` — directly to the webview. This is the only command whose return value carries the hydrated maps. |
| 4 | `settings/types.rs:323-337` (`impl Serialize for SecretMap`) | **The choke point:** regardless of contents, `SecretMap` serializes as a zero-entry map (`serializer.serialize_map(Some(0))`). The wire format sent to the webview therefore contains `"speech_pipeline_api_keys": {}` and `"post_process_api_keys": {}`. |
| 5 | Events | Verified against the full emit inventory (`grep '\.emit' src-tauri/src`, 76 hits reviewed): no event payload contains `AppSettings`, either `SecretMap`, or any raw `HashMap<String,String>` of keys. Event payloads are transcription text, audio levels, model-download progress, history ids, theme, secure-input status, log records. |

### 1.2 Verdict on webview exposure

**Plaintext API keys are NOT transmitted to, and NOT retained by, the frontend through any current command or event path.**

- The hydrated maps exist in backend memory only; serialization strips them (`types.rs:329-336`), so `get_app_settings` delivers `{}`.
- Corroborating frontend evidence: the UI reads `settings.speech_pipeline_api_keys?.azure ?? ""` (`src/components/settings/general/SpeechPipelineSettings.tsx:56,69,329,361`) and `settings?.post_process_api_keys?.[providerId] ?? ""` (`src/components/settings/PostProcessingSettingsApi/usePostProcessProviderState.ts:65,101`) and always observes `""`. Keys are **write-only** from the UI's perspective: they enter via `change_speech_pipeline_api_key_setting` (`shortcut/mod.rs:696-722`) and `change_post_process_api_key_setting` (`shortcut/mod.rs:1148-1168`), which forward directly into `secret_store::set_secret` and never touch the settings store.
- Re-persistence of hydrated values is impossible through the normal write path: `update_settings`/`try_update_settings` persist via `serde_json::to_value(settings)` (`store.rs:215`), which routes both maps through the same empty-map `Serialize` impl. Disk stores (`settings_store.json`) contain `{}` for both maps after migration.

### 1.3 Debug dumps and store snapshots

- `debug!("Loaded settings: {:?}", settings)` fires after hydration (`store.rs:34`, called from `load_or_create_app_settings:32-36`) — safe: `SecretMap`'s `Debug` impl redacts non-empty values to `[REDACTED]` (`types.rs:339-348`; tested in `settings/tests.rs:392-417`).
- Salvage-path logging logs field *names* only, with an explicit comment that values may hold secrets (`store.rs:115-116`).
- No code path serializes the hydrated maps anywhere except through `SecretMap::Serialize` → `{}`. A grep for `to_value`/`to_string` over settings confirms all persistence and command-output serialization goes through the redacting impl (`store.rs:74,80,85,106,215`; `commands/mod.rs:34-41`).
- The webview holds the `store:default` plugin permission plus `fs:read-files` scoped to `$APPDATA` (`capabilities/default.json:9,18-23`), i.e. it *could* open `settings_store.json` directly — but post-migration that file contains no key material, so this is defense-in-depth surface only (see §4.5).

### 1.4 Adjacent IPC finding

| ID | Finding | Severity |
|---|---|---|
| F-1 | `get_azure_speech_auth` (`speech_pipeline.rs:460-503`) fetches a **short-lived Azure issueToken result** and returns it in `AzureSpeechAuth { token, region, … }` to the webview (`speech_pipeline.rs:495-503`). The long-lived API key itself stays backend-side (`speech_pipeline.rs:466-468`), so this does not violate the no-key-material rule — but the token is a bearer credential exposed to webview JS (and thereby to any XSS in the frontend bundle). It never touches disk or logs. | **Low** |

**Remediation F-1:** acceptable as-is for desktop trust model; if hardening, keep the token in backend state and proxy Azure websocket auth through a command, or shorten token TTL handling. Document the token as intentional webview-visible credential in the meeting-mode redaction contract's non-goals.

### 1.5 Contract gap observed in the frontend (informational)

Because keys are write-only, `usePostProcessProviderState.ts:103` computes `hasApiKey` from a value that is always `""`, so its auto-fetch heuristic at line 105 under-fetches for providers whose key exists only in the backend. Not a security defect; listed because future meeting-mode work will copy this pattern and should not infer key presence from the (always-empty) frontend map. Severity: **Info**.

---

## 2. Linux keychain-fallback durability (Gate G2)

Scope: the `#[cfg(not(windows|macos))] imp` module, `secret_store.rs:72-115`, backing file resolved by `fallback_file()` (`secret_store.rs:117-129`: portable data dir, else `$XDG_DATA_HOME/speak/secrets.json`, else `~/.local/share/speak/secrets.json`).

### 2.1 Non-atomic read-modify-write can destroy ALL secrets

| ID | Finding | Severity |
|---|---|---|
| F-2 | Every mutation is a whole-file read-modify-write: `set_secret` = `read_all()` → `insert` → `write_all` (`secret_store.rs:103-107`); `delete_secret` likewise (`109-114`). `write_all` uses plain `std::fs::write(&path, json)` (`secret_store.rs:96`) — truncate-then-write, no temp file, no rename, no fsync. `read_all` treats *any* read error **and** any JSON parse failure as "no secrets" (`Err(_) => HashMap::new()`, `unwrap_or_default()`, `secret_store.rs:81-84`). | **High** |

**Concrete loss interleavings (derived from the actual code):**

*Torn write / crash (total loss):*
1. Secrets file contains `{"speak:speech-pipeline/azure":"KEY_A","speak:post-process/p1":"KEY_B"}`.
2. User saves a DeepL key → `set_secret` → `std::fs::write` **truncates** `secrets.json` (`secret_store.rs:96`) and begins writing the new 3-entry JSON.
3. Process crash / power loss mid-write → file holds a partial JSON document.
4. Next launch: `read_all` → `from_str(...).unwrap_or_default()` fails silently → `{}` (`secret_store.rs:82`).
5. The next successful `set_secret` writes a fresh file containing **only** the newly set key. KEY_A and KEY_B are permanently destroyed; there is no backup, no quarantine, no error ever surfaced.

*Concurrent lost update (silent partial loss):*
1. Two Tauri commands execute concurrently on different runtime threads (Tauri 2 runs commands off the main thread; nothing in the fallback `imp` takes a lock — contrast `SETTINGS_TRANSACTION_LOCK` which guards the settings store, `store.rs:21-27`).
2. Thread 1 (`change_speech_pipeline_api_key_setting`) and thread 2 (`delete_secret` from `change_post_process_api_key_setting`) both `read_all()` → same snapshot.
3. Thread 1 writes `{azure, p1}`; thread 2 then writes back its own stale-based map without azure → azure's new key is silently gone. Last writer wins; no conflict detection.

**Remediation F-2:** write to `secrets.json.tmp` with `OpenOptions`, `fsync`, then `atomic rename` onto the target; treat a parse failure of the primary file as fatal (refuse mutations and warn loudly instead of `unwrap_or_default`); guard all fallback `imp` functions with a process-wide `Mutex` (mirroring `store.rs:21-27`); optionally keep one rotating `.bak` written before each successful replace.

### 2.2 No restrictive permissions on `secrets.json`

| ID | Finding | Severity |
|---|---|---|
| F-3 | `write_all` creates the file with default umask-derived mode (typically `0644` on Linux) — world/group readable plaintext API keys. No `PermissionsExt::set_mode(0o600)`, no pre-create chmod before content lands, no check on existing files. The parent dir is created with `create_dir_all` defaults (`secret_store.rs:91-94`). Portable-mode deployments (`fallback_file()` prefers `portable::data_dir()`, `secret_store.rs:119`) put the file next to the executable, frequently on FAT/exFAT USB media where POSIX modes don't exist at all. | **High (multi-user Linux), Medium (single-user)** |

**Remediation F-3:** create/open the temp file with mode `0600` before writing content (and chmod existing files to `0600` on first touch); verify after rename; in portable mode, warn that the fallback file inherits media semantics and recommend the OS keychain where available.

### 2.3 Silently ignored delete errors

| ID | Finding | Severity |
|---|---|---|
| F-4 | `delete_secret` discards the `write_all` result: `let _ = write_all(&map);` (`secret_store.rs:112`), and the public wrapper returns `()` (`secret_store.rs:142-144`). Callers (`shortcut/mod.rs:710-714,1159`) then report success to the UI. On a full disk or permission error, the "deleted" key remains in the plaintext file indefinitely while the user believes it is gone. Note the same pattern exists in the keychain imp (`let _ = entry.delete_credential();`, `secret_store.rs:67`), where a failed delete also surfaces as success. | **Medium** |

**Remediation F-4:** make `delete_secret` return `Result<(), String>` like `set_secret`, propagate through `change_*_api_key_setting`, and surface a toast/error in the UI. At minimum, `error!`-log the failure (currently fully silent).

### 2.4 Read errors conflated with "unset"

Covered inside F-2, worth stating separately because it converts any transient failure (EACCES after an ACL change, EIO, corrupted file) into an empty credential universe: subsequent logic such as `fetch_post_process_models` (`shortcut/mod.rs:1288-1298`) concludes "API key is required", and a later `set_secret` persists a map missing every other entry. Severity folded into F-2 (**High**).

---

## 3. Migration edge cases in `migrate_legacy_settings_secrets` (part c)

Function: `secret_store.rs:152-194`; invoked once per startup at `lib.rs:512-515` inside `settings::update_settings`.

### 3.1 Partial-failure migration erases legacy plaintext keys (data loss)

| ID | Finding | Severity |
|---|---|---|
| F-5 | If `imp::set_secret` fails for provider X (locked keychain, Credential Manager error, F-2 torn file), the loop `continue`s **with X still present in the in-memory map** (`secret_store.rs:179-184`) — the comment says this is so "nothing is lost" and migration retries next startup. But `lib.rs:512` uses `update_settings`, whose write path is unconditional (`store.rs:219-225` → `write_settings_unlocked` → `serde_json::to_value(settings)`), and `SecretMap::Serialize` emits `{}` (`types.rs:329-336`). Result: the settings store is rewritten with `"speech_pipeline_api_keys": {}`, destroying the legacy plaintext of provider X on disk while X was **never** written to the keychain. On next startup there is nothing left to migrate. The retry-safety comment is false. | **High** |

**Concrete sequence:** legacy store has `azure=K1, deepl=K2` → azure migrates OK, `migrated_any=true` → deepl `set_secret` fails → `continue` keeps K2 in memory → `update_settings` persists → disk now `{}` for both maps → K2 lost forever.

**Remediation F-5:** fail the whole transaction when any `set_secret` fails: have `migrate_legacy_settings_secrets` return `Result<bool, String>`, use `try_update_settings` (`store.rs:228-237`) so persistence is skipped on error, and only remove entries whose keychain write succeeded. Alternative: stage all keychain writes first, and remove legacy entries only after every write succeeded.

### 3.2 Headless CLI path can persist settings without ever migrating

| ID | Finding | Severity |
|---|---|---|
| F-6 | In headless mode (`--transcribe-file` etc.), setup returns early at `lib.rs:488`, so the migration at `lib.rs:512` never runs. But `apply_accelerator_settings` (`lib.rs:466` → `managers/transcription.rs:2027-2030`) calls `get_settings`, and `get_settings_unlocked` persists whenever a migration/bindings-merge marks the store dirty (`store.rs:73-75,84-86`). For a user whose store still holds legacy plaintext keys (e.g. GUI never launched since upgrade), a CLI-only invocation can rewrite the store with `{}` maps — destroying unmigrated keys without migrating them. Normal GUI startup orders correctly (migration at `lib.rs:512` precedes the first other settings consumer, `initialize_core_logic` at `lib.rs:541`), so this needs the specific headless-first condition. | **Medium** |

**Remediation F-6:** run `migrate_legacy_settings_secrets` (or at minimum refuse to persist settings) in the headless setup branch before any `get_settings` call; simplest is hoisting the migration above the headless split.

### 3.3 Edge cases checked and found sound (for the record)

- Empty-string entries mean "not configured" and are skipped by the filter (`secret_store.rs:163-167`); covered by `secret_store.rs:238-243`.
- Stale-copy drop when the keychain already holds an entry is deliberate and logged (`secret_store.rs:170-177`).
- Hydrate-before-migrate ordering inside `update_settings` (`store.rs:90` runs before the closure at `lib.rs:512-513`) cannot clobber legacy values: `hydrate_secret_maps` only fills empty/missing entries (`secret_store.rs:202-212,219-228`), so non-empty legacy values survive until migration examines them.
- Migration iterates the key maps, not the provider lists (`secret_store.rs:162-190`), so keys belonging to removed/unrecognized post-process provider ids are still migrated, not dropped.
- Failure of one id does not skip removal of successfully migrated ids — correct except for the F-5 erase side effect.

---

## 4. Redaction contract for future meeting-mode commands/events (Gate G3)

### 4.1 Contract statement

**No meeting-mode Tauri command return value, event payload, or debug/diagnostic dump may contain API-key material (values of `speech_pipeline_api_keys`, `post_process_api_keys`, or any future key map) in any form: plaintext, masked-but-reversible, or derived bearer material other than explicitly approved short-lived tokens (see F-1).**

### 4.2 Exact enforcement choke point

**Primary choke point (existing, structural): `crate::settings::types::SecretMap::serialize` (`settings/types.rs:323-337`).** Any payload that carries settings-derived state must type its key collections as `SecretMap` (never a raw `HashMap<String,String>`/`Record<string,string>`); the strip then happens unconditionally at serialization, identically for commands, events, and store persistence. This is the single function through which every current safe path already flows (see §1.1 step 4).

**Secondary choke point (for payloads built ad hoc): `commands::get_app_settings`-style construction** — meeting-mode commands must source settings exclusively via `crate::settings::get_settings`/`update_settings` and must not re-plumb hydrated maps into bespoke response structs. Any new meeting-module response/event type lives next to `commands/` and is registered in `collect_commands!`/`collect_events!` (`lib.rs:200-336`), which is the review checkpoint where the type audit below is applied.

### 4.3 How the contract is enforced/tested

1. **Serialization canary unit test** (missing today — noted gap, §4.4): in `settings/tests.rs`, populate both maps with sentinel values (`sk-canary-…`), assert `serde_json::to_value(&settings)["speech_pipeline_api_keys"] == {}` and same for `post_process_api_keys`, and assert the full serialized JSON does not contain the sentinel. This pins the choke point itself.
2. **Type-level gate:** every meeting-mode command/event type that embeds settings must reference `SecretMap`. Enforced in review via `bindings.ts` diff inspection (generated at `lib.rs:338-344`): no meeting-mode payload may declare a key-map field typed as anything other than `SecretMap`. A lightweight CI grep can automate: flag `HashMap<String, String>` fields named `*api_key*`/`*key*` outside `secret_store.rs`/`types.rs`.
3. **Runtime canary sweep test:** integration-style test that sets a sentinel key via `secret_store::set_secret`, invokes each meeting-mode command with representative args, captures every emitted event during a scripted session, and asserts the sentinel substring appears in none of the serialized outputs. This catches bespoke response structs even if someone bypasses `SecretMap`.

### 4.4 Current-state gap feeding the contract

There are tests for `Debug` redaction (`settings/tests.rs:392-417`) but **no test pins the `Serialize`-as-`{}` behavior** — the property the entire §1 verdict rests on. If someone weakens `SecretMap::serialize` (e.g. to support a "show key" UI feature), `get_app_settings` would start leaking all keys with no failing test. Adding test (1) above is the highest-value follow-up and is a prerequisite for trusting the contract.

---

## 5. Findings summary

| ID | Area | Severity | One-line remediation |
|---|---|---|---|
| F-1 | IPC (`speech_pipeline.rs:460-503`) | Low | Accept short-lived Azure token exposure; document as intentional in the redaction contract. |
| F-2 | Fallback durability (`secret_store.rs:81-84,96,103-114`) | High | Temp-file + fsync + atomic rename; fail closed on parse errors; add a process-wide mutex. |
| F-3 | Fallback durability (`secret_store.rs:91-96,119`) | High/Medium | Create temp file mode `0600` before content; chmod existing files; warn in portable mode. |
| F-4 | Fallback/keychain delete (`secret_store.rs:67,112`) | Medium | Return `Result` from `delete_secret`; propagate to callers/UI; stop swallowing errors. |
| F-5 | Migration (`secret_store.rs:179-189` + `store.rs:219-225` + `types.rs:329-336`) | High | Use `try_update_settings`; abort persistence if any keychain write fails; remove entries only after success. |
| F-6 | Migration ordering (`lib.rs:454-488` vs `lib.rs:512`) | Medium | Migrate before the headless early-return, or suppress persist-on-read in headless mode. |
| — | Test gap (§4.4) | Medium (contract risk) | Add serialize-canary test pinning `SecretMap` → `{}`. |

No findings require immediate code change within this read-only task; F-2/F-3/F-5 are the priority repairs for a follow-up implementation task.

## 6. Verification performed

- Full reads: `secret_store.rs`, `settings/{mod,store,types,schema,tests}.rs`, `commands/mod.rs`, targeted reads of `shortcut/mod.rs`, `speech_pipeline.rs`, `actions.rs`, `managers/transcription.rs`, `portable.rs`, `lib.rs`, `capabilities/default.json`, frontend consumers (`SpeechPipelineSettings.tsx`, `usePostProcessProviderState.ts`, `bindings.ts`).
- Emit inventory: `rg '\.emit'` across `src-tauri/src` (76 matches reviewed) — none carry key maps.
- Command inventory: `collect_commands!` (`lib.rs:200-336`) cross-checked against return types; only `get_app_settings` returns settings, sanitized at `types.rs:329`.
- Serialization-path sweep: all `serde_json::to_value/to_string` uses over settings route through the redacting `Serialize` impl.
- No tests were executed for this audit (no source modified); cited tests are quoted from source at the referenced lines.
