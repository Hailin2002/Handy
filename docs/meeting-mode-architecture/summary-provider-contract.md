# Summary-Provider Contract — Selection, Fallback, and Cloud-Cost Guard

Status: **spec contract — no implementation yet** (Phase 7 prerequisite)
Source of truth: `docs/meeting-mode-plan.md` §5 (`MeetingSummaryService`,
plan lines 253–259), §10 (provider independence of summarization,
plan lines 1340–1347), §6.7 (Summary-Schalter „Nach dem Meeting
zusammenfassen", plan line 522), Startvalidierung (optionaler
Summary-Provider, plan line 907), Session-Zustände (plan lines 877–891),
Fehlerverhalten (plan lines 943–960), Stoppen (plan lines 936–941),
§11 erneute Zusammenfassung (plan line 1417), §12 disclosure duty
(plan lines 1454–1456) and retention protection (plan lines 1451–1452),
§13 Phase 7 (plan lines 1629–1645).
Planned implementation home: `src-tauri/src/managers/meeting_summary.rs`
(plan line 259). This document defines **how `MeetingSummaryService`
chooses between the local and the cloud summary path** — consent gating,
provider ordering, finite fallback, request-size budgeting handoff,
disclosure, and hard-failure behavior. It changes no source code and
touches no internals of `src-tauri/src/llm_client.rs`.

Sibling documents consumed read-only:

- [summary-evidence-contract.md](summary-evidence-contract.md) — pipeline
  skeleton M0–M3 (§1), idempotency keys (§2), local-mode proof (§5),
  version pinning (§6). Below abbreviated **SEC-§n**.
- [summary-pipeline-dataflow.md](summary-pipeline-dataflow.md) — windowing
  and token budgets (§3), job divergences D1–D5 (§4). Authored by
  task-0090. Below abbreviated **SPD-§n**.
- [job-scheduler-contract.md](job-scheduler-contract.md) — concurrency
  class S1 (`MAX_CONCURRENT_LLM_JOBS`), retry/backoff ownership.
  Below abbreviated **JSC-§n**.
- [stt-provider-contract.md](stt-provider-contract.md) — job state machine
  and error classes (STT-§3, STT-§6).
- [repository-boundary-contract.md](repository-boundary-contract.md) —
  store seams. Below abbreviated **RB-§n**.

Conflict rule: the plan wins over this contract, this contract wins over
catalogs, and nothing wins over actual code.

Ownership note: SPD-§7 lists "provider/model selection logic" as a
non-goal of the data-flow document and defers it. This contract assumes
that ownership for the _selection, consent, and fallback policy_. It does
not recommend concrete models or vendors — those stay product decisions
outside architecture contracts (same split as SEC-§0.2).

---

## 0. Scope

### 0.1 In scope

1. The exact conditions under which the **cloud summary path** may be
   taken (persisted consent flag, G1).
2. The **provider ordering and finite fallback chain**, including the
   terminal user-visible state and its copy-catalog keys (G2).
3. Restating the **local-mode guarantee** identically to SEC-§5 (G3).
4. The **request-size budgeting handoff**: how the chosen provider's
   context size feeds the windowing owned by SPD-§3, without duplicating
   any windowing rule here.
5. **Cost/display disclosure** duties before, during, and after cloud
   runs — and the explicit absence of cost metering (G4).
6. **Hard failure behavior** when neither path can produce a summary.

### 0.2 Explicitly out of scope (non-goals, gate G4)

| Non-goal                                                              | Reason for rejection                                                                                                                                                                                                                                                                                                                                                   |
| --------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Automatic model downloads for local summarization                     | The local path uses whatever inference mechanism the user already has on the device (Phase 7 implementation decision, §1); downloading or managing models is not a summary concern and would duplicate the model manager's mandate. If no local mechanism is available, the local kind is simply unavailable (§4.4) — it is never bootstrapped behind the user's back. |
| Cost metering, token accounting, price tables, billing estimates      | Only qualitative disclosure (which provider receives what data, plan lines 1454–1456) is contractual. Metered cost display adds a pricing-data maintenance burden with no evidence-backed v1 need; per-run token/provenance pinning (SEC-§6) already makes every run auditable after the fact.                                                                         |
| Fallback chains deeper than one cloud attempt + one local attempt     | Each extra hop multiplies disclosure states and failure combinations for negligible value; §3 fixes the chain at maximum length 2.                                                                                                                                                                                                                                     |
| Automatic model/routing selection ("pick the best model per meeting") | Non-deterministic provider choice breaks run-key reproducibility (SEC-§2.1 pins `summary_provider_id`) and hides data flows from the user. The user selects; the service never upgrades or downgrades silently except through the single disclosed fallback rule of §3.3.                                                                                              |
| Coupling summary-provider choice to STT-provider choice               | Plan lines 1340–1347 declare them independent; this contract adds no constraint in either direction.                                                                                                                                                                                                                                                                   |
| Changes to `src-tauri/src/llm_client.rs` internals                    | The cloud path consumes the existing client functions as-is (§1); no refactor, no new entry points, no behavior change in that file is required or permitted by this contract.                                                                                                                                                                                         |

---

## 1. The two summary paths

Exactly two provider kinds exist in v1:

```text
MeetingSummaryProvider (conceptual seam, implemented inside
                        src-tauri/src/managers/meeting_summary.rs)

  LocalSummaryProvider    on-device inference; performs NO HTTP client
                          construction at all — the letter of SEC-§5.1
                          and the only form that passes the SEC-§5.2
                          egress assertion test (zero TCP connects,
                          loopback included). The concrete mechanism
                          (in-process engine or equivalent) is a Phase 7
                          implementation decision; if none is available,
                          this kind is simply UNAVAILABLE (§4.4) — it is
                          never bootstrapped behind the user's back.
  CloudSummaryProvider    OpenAI-compatible request against a remote
                          base_url via the EXISTING client infrastructure
                          src-tauri/src/llm_client.rs::
                          send_chat_completion_with_schema
                          (verified live; see §8 disclosure table).
```

Rules:

1. **One seam, two kinds.** Both kinds satisfy the same conceptual
   interface used by stages M1/M2 (SEC-§1.1): render input (SPD-§1.2),
   receive pinned prompt block (SEC-§6), return schema-validated output.
   Callers never branch on vendor identity, only on the two kinds above.
2. **No third kind.** A future kind (hosted private inference, hybrid)
   extends the enum/seam; it inherits every rule of this contract unless
   a successor contract supersedes it.
3. **Secrets discipline unchanged.** API keys enter as per-call arguments
   only and are never persisted into jobs, stages, summaries, or logs —
   matching plan lines 1334–1336 and the verified `llm_client.rs` pattern
   (key passed as `api_key: String` argument, never stored).

---

## 2. Cloud consent gate (gate G1)

### 2.1 Persisted setting key and default

| Property         | Value                                                                                                                                                                                                                         |
| ---------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Setting key      | `meeting_summary_cloud_consent`                                                                                                                                                                                               |
| Type             | `bool`                                                                                                                                                                                                                        |
| Default          | `false`                                                                                                                                                                                                                       |
| Home             | New field on the existing typed `AppSettings` struct (`src-tauri/src/settings/schema.rs`), persisted through the same tauri-plugin-store mechanism as every other key in that struct — no second settings store is introduced |
| Settable only by | An explicit user toggle in Settings **or** the per-session confirmation button `meeting.actions.useCloudSummary` (P5, ux-copy-catalog), which writes the same flag                                                            |

### 2.2 Consent semantics

1. **Opt-in, never inferred.** No other setting implies consent. In
   particular `post_process_enabled = true` (dictation post-processing,
   `AppSettings.post_process_enabled`) does NOT set or imply
   `meeting_summary_cloud_consent`; dictation and meeting modes keep
   separate consent surfaces, consistent with the settings separation of
   the coexistence contract. Upgrading, onboarding, and defaults never
   flip the flag; only the two user actions of §2.1 do.
2. **Gate placement — unreachable, not merely discouraged.** When the
   flag is `false`, the cloud kind is _unconstructible_ for meeting
   summary runs: `MeetingManager` validation (plan line 907, optional
   Summary-Provider) rejects a cloud selection before the session starts,
   and `MeetingSummaryService` refuses to enqueue any M1/M2 stage whose
   resolved provider kind is cloud. The refusal is a hard precondition
   check ahead of the run-key computation, so a cloud run cannot even
   enter `processing_jobs` while consent is off.
3. **Per-session pinning.** Each session snapshot records the selected
   summary choice (`none` | `local` | `cloud`) made in the start dialog
   (plan line 522, plan lines 531–533: snapshots isolate sessions from
   later global setting changes). A session started with `local` or
   `none` never becomes a cloud session mid-flight, even if the user
   flips the global flag while the meeting processes — changing the flag
   affects only future sessions and explicit re-summarize runs (plan
   line 1417), which are themselves gated again by rule 2.
4. **Consent is part of reproducibility.** The boolean consent state at
   enqueue time is carried inside `provider_settings_snapshot` of the
   run key (SEC-§2.1), so "this run was allowed to go to the cloud" is a
   persisted, hashable fact, not reconstructible guesswork.
5. **Withdrawal.** Turning the flag off takes effect at the next gate
   evaluation. It never aborts a currently `running` cloud job
   retroactively (the disclosure was given and data already sent);
   cloud stages of the same run that have not yet started a cloud
   request are re-resolved against the withdrawn consent: per §3.2
   rule 3 this counts as a permanent cloud failure, so a selected
   `cloud` chain falls back to local, and a chain whose local element
   is also unavailable ends in §6 hard failure — no further byte is
   sent after withdrawal.

---

## 3. Provider ordering and fallback (gate G2)

### 3.1 Ordering source

There is no implicit preference algorithm. The order is:

1. **User selection wins.** The session snapshot's summary choice is the
   first and normally only entry of the attempt chain.
2. If the selection is `cloud`, the chain is `[cloud, local]`.
3. If the selection is `local`, the chain is `[local]` — **the cloud kind
   is never appended** (rule §3.4.1: fallback may only move toward fewer
   disclosures, never more).
4. If the selection is `none`, summarization is skipped entirely
   (session transitions per SSC T11 analog; nothing in this contract
   resurrects it automatically).

### 3.2 When fallback triggers

Fallback from `cloud` to `local` happens only when **all** of:

1. the cloud job reached terminal `failed` under the normal retry
   machinery (transient errors exhaust `max_attempts` and backoff exactly
   as specified in SPD-§4.2 D4; the scheduler's S1 slot rules of JSC-§n
   apply unchanged during every attempt), or
2. the failure was classified **permanent** by the provider-side error
   classes (auth rejected, model unknown, quota exhausted — same
   classification ownership as STT-§6: classification lives with the
   provider, clocks live with the scheduler), or
3. consent was withdrawn while the run was queued (§2.2 rule 5), which
   counts as a permanent cloud failure for chain purposes.

A `cancelled` cloud job never falls back: cancellation is a user decision,
not a provider defect (SPD-§4.2 D5 semantics).

### 3.3 Fallback execution

1. The fallback starts a **new run**, not a retry of the old one: the new
   run's `summary_provider_id` differs, therefore its idempotency key
   (SEC-§2.1) differs and it gets its own `processing_jobs` row. The old
   run stays `failed` with its original error class — audit trail intact.
2. **Cross-provider intermediate reuse is forbidden in v1.** Stage rows
   pin provider/model/prompt (SEC-§1.3, SEC-§6); reusing cloud-produced
   map results inside a local reduce would mix provenance inside one
   artifact for a marginal saving. The local run recomputes M0→M3 from
   the same immutable segment set (deterministic per SPD-§1, so windows
   are identical; only model calls are redone). Complexity-budget
   rationale: provenance-mixing rules, per-stage provider affinity and a
   second dedup surface cost more than the saved calls.
3. The chain has **maximum length 2** and is evaluated once: after the
   local attempt also reaches terminal `failed`, the run ends in the
   hard-failure state of §6. There is no loop, no return to cloud, no
   third element.

### 3.4 Direction invariant (testable)

> **INV-P1:** Every automatic transition between provider kinds in a
> summary attempt chain replaces a cloud kind with the local kind, never
> the reverse. Equivalently: no code path may enqueue a cloud-kind stage
> whose session did not select `cloud` with consent set (§2.2 rule 2).

This is what keeps "fallback" privacy-safe: the local destination sends
nothing off-device (§5), so falling back needs no fresh consent and no
fresh disclosure beyond the post-hoc labeling of §5.4.

---

## 4. Request-size budgeting (handoff to SPD/task-0090)

1. **Windowing is owned elsewhere.** How a long transcript is split into
   segment-aligned windows, the token estimator, budget arithmetic, and
   the map-reduce vs. single-pass rule are normatively defined in
   SPD-§1.3 and SPD-§3 and are **not restated or modified here**. This
   contract only supplies inputs to and takes outputs from that machinery.
2. **Context size per attempt.** The provider kind selected for the
   current attempt contributes its context size `C` to M0:
   - `cloud`: `C` from the pinned provider configuration (a non-secret
     per-provider setting; unknown or unparsable values fall back to the
     most conservative row of SPD-§3.4, `C = 8192`);
   - `local`: `C` likewise from the local inference configuration, same
     conservative fallback.
     `C` is recorded in the persisted window-plan stage row (SEC-§1.3), so
     every run's splitting is reconstructible.
3. **Budgets bind both kinds equally.** No provider kind may receive a
   request exceeding `WINDOW_INPUT_BUDGET(C)` (SPD-§3.3); oversize input
   is a defect of the caller, never solved by truncating transcript text
   (SPD-§7: no mid-segment splits).
4. **Unavailability is not an error of the budget.** If no local
   inference mechanism is available on the device, the local kind is
   _unavailable_ (not failing): the attempt chain of §3 collapses to its
   first element only, and a cloud failure lands directly in §6 hard
   failure. Missing local capability is detected by the start-validation
   probe (plan line 907) and surfaced there, not discovered mid-run.

---

## 5. Disclosure duties and cost guard (part of G4)

### 5.1 Before a cloud run

Every cloud-bound run — initial summary and explicit re-summarize alike —
is preceded by the P5 disclosure flow (ux-copy-catalog, proposal-only):
title/body keys `meeting.cloud.notice.llm.title` /
`meeting.cloud.notice.llm.body` naming the provider and stating that only
transcript text (for long meetings: the Map-Reduce text windows, no
audio) leaves the device, with actions `useCloudSummary` /
`summarizeLocally` / `disableSummary`. Normative basis: plan lines
1454–1456 ("zeigt die UI vor dem Start und während der Verarbeitung
klar, welche Daten an welchen Provider gesendet werden").

### 5.2 During a cloud run

While any cloud-kind stage is `queued`/`running`, the surfaces show the
S7 active line `meeting.cloud.active.llm` (proposal-only catalog key),
again naming the provider. The line is absent for pure-local runs — its
absence is itself the honest signal required by plan line 1644.

### 5.3 After fallback

When INV-P1 moved a run from cloud to local, the published summary's
provenance (SEC-§6.1 fields) shows the provider that actually produced
it (`local`), and the detail view's provenance line reflects that. No
additional dialog is required — the run stayed on-device end-to-end.

### 5.4 What is deliberately NOT built (cost guard)

- No token counters, price lookups, spend accumulation, quotas, or
  warnings ("you have spent …") anywhere in the meeting flow.
- No per-run cost estimate in the disclosure dialogs; they name the
  recipient and the data types, per plan lines 1454–1456, and stop there.
- The only quantitative trace is the retrospective one already mandated
  by SEC-§6: provider, model, prompt version per run/stage.

Rationale (documented complexity budget): metering requires maintained
price data per vendor/model, currency/locale handling, and a trust model
for self-reported usage numbers — high complexity, low evidenced user
value for v1, and it duplicates what provider dashboards already show.

---

## 6. Hard failure when neither path is available (gate G2 terminal)

When the attempt chain of §3 is exhausted (cloud failed terminally and
local unavailable-or-failed, or local selected and failed):

1. **State.** The last `processing_jobs` row is terminal `failed` with a
   distinct, explainable error class:

   | Error class                 | Meaning                                                                                                    |
   | --------------------------- | ---------------------------------------------------------------------------------------------------------- |
   | `summary_no_path_available` | chain exhausted: every candidate kind failed or was unavailable (incl. consent withdrawal per §3.2 rule 3) |

   The session follows SSC T13: `recoverable_error` with `last_error`
   carrying the class. Nothing is silently swallowed (plan lines 888–890:
   no silent data loss).

2. **Copy-catalog keys** (proposals to be integrated by the
   ux-copy-catalog owner; catalog remains proposal-only authority):

   | Key (proposal)                                   | English text (proposal)                                                               |
   | ------------------------------------------------ | ------------------------------------------------------------------------------------- |
   | `meeting.summary.failed.title`                   | `Summary failed`                                                                      |
   | `meeting.summary.failed.body`                    | `The summary could not be created. Your recording, transcript and all data are safe.` |
   | `meeting.summary.failed.reason.noPath`           | `Neither the local nor the cloud summary path was available.`                         |
   | `meeting.summary.failed.reason.consentWithdrawn` | `Cloud summarization was turned off before the summary ran.`                          |

   Reused action keys from the catalog's shared table: `tryAgain`
   (re-enqueue per §3.3 rule 1 mechanics), `openSettings`
   (fix provider/key or enable local inference), `disableSummary`
   (dismiss: session proceeds to `completed` without a summary — the
   SSC T14 dismissal path).

3. **Retention protection applies.** While the failed job exists, it
   blocks automatic audio deletion exactly like failed STT jobs
   (plan lines 1451–1452); the user's recording and transcript are never
   lost because summarization failed.
4. **Retry identity.** A user-initiated "Try again" creates a new run
   generation (new idempotency key, SEC-§2.1); it re-evaluates the whole
   chain from the top under current consent/configuration, so fixing the
   root cause (key entered, local inference enabled) is sufficient — no
   stale attempt-chain memory.
5. **Finite by construction.** Between two user actions the system
   performs at most: (cloud retries up to `max_attempts`) + (at most one
   local attempt). Every path — success, fallback-success, hard failure,
   cancellation — ends in a terminal job state with a user-visible
   derivation (SPD-§4.3 derived view). There is no state in which a
   summary is neither running nor finished-and-explainable.

---

## 7. Local-mode guarantee restatement (gate G3)

Identical in meaning to SEC-§5; repeated here so this contract stands
alone, with the same normative force:

1. **Type-level separation.** The local summary kind performs no HTTP
   client construction at all; the cloud kind exclusively uses the
   existing `send_chat_completion_with_schema` infrastructure of
   `src-tauri/src/llm_client.rs`. This is the letter of SEC-§5.1,
   deliberately stronger than "loopback only": the SEC-§5.2 egress test
   fails on any TCP connect, so an HTTP-to-localhost design could not
   carry the "nachweislich" property.
2. **Egress assertion test.** A mandatory integration test runs a full
   local-mode summary against a fixture corpus while a test-installed
   socket guard fails the process on any outbound TCP connect; the test
   passes only with zero connects (executable form of plan line 1644
   "nachweislich").
3. **UI symmetry.** Cloud selections show the §5.1/§5.2 disclosures; the
   absence of those disclosures on local runs is part of the guarantee.
4. **Minimization.** Cloud runs receive only the current stage's windows
   (SPD-§3 sizing), never the whole archive or unrelated meetings'
   segments (plan lines 1402–1403).

With §2 (consent gate) and INV-P1 (fallback direction) this yields the
closed property: **without an explicit, persisted, user-set consent flag,
no transcript byte of a meeting summary ever leaves the device —
including through fallback, retry, or recovery paths.**

---

## 8. External-citation disclosure

| Citation                                                    | Status at authoring time                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| ----------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `src-tauri/src/llm_client.rs` (§1 rule 3, §5/§7 cloud path) | **LIVE-VERIFIED** — file read in this workspace: `send_chat_completion(provider, api_key, model, prompt, disable_reasoning)` (lines 300–317) delegates to `send_chat_completion_with_schema(provider, api_key, model, user_content, system_prompt, json_schema, disable_reasoning)` (lines 329–337); API key enters as an argument, is never persisted, and the module contains no settings write path.                                                                                                    |
| `src-tauri/src/settings/schema.rs` (§2.1 settings home)     | **LIVE-VERIFIED** — file read in this workspace: typed `AppSettings` struct (line 21 ff.) with snake_case persisted fields (e.g. `post_process_enabled`, line 118) is the established pattern the new key `meeting_summary_cloud_consent` follows.                                                                                                                                                                                                                                                         |
| ux-copy-catalog P5/S7/shared-table keys (§5, §6.2)          | **LIVE-VERIFIED as proposal-only** — keys `meeting.actions.useCloudSummary` / `summarizeLocally` / `disableSummary` (lines 484–486), `meeting.cloud.notice.llm.*` (lines 479–483), `meeting.cloud.active.llm` (lines 182–185), shared actions `tryAgain`/`openSettings` (lines 64–66) exist in the catalog; the catalog self-declares "nothing of this is implemented". The four new `meeting.summary.failed.*` keys of §6.2 are NEW proposals introduced by this contract and are NOT yet in the catalog. |
| All other claims                                            | Derived solely from `docs/meeting-mode-plan.md` (line-numbered quotes) and sibling architecture contracts in this repository. No new external (web/vendor) claims are made.                                                                                                                                                                                                                                                                                                                                |

---

## 9. Gate verification checklist

| Gate | Requirement                                                                                              | Where satisfied                                                                                                                                                                     |
| ---- | -------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| G1   | Cloud path unreachable without persisted consent flag; key and default stated                            | §2.1 (key `meeting_summary_cloud_consent`, default `false`, typed-settings home), §2.2 (unconstructibility gate, opt-in-only writers, session pinning, withdrawal)                  |
| G2   | Finite fallback chain terminating in user-visible state with copy-catalog key                            | §3 (ordering, trigger, execution, max length 2, INV-P1), §6 (terminal `failed` + `summary_no_path_available`, proposed keys, retention protection, finite-by-construction argument) |
| G3   | Local-mode guarantee restated consistently with summary-evidence-contract §5                             | §7 (four-point restatement mirroring SEC-§5 items 1–4, plus closure property)                                                                                                       |
| G4   | Non-goals: no automatic model downloads, no cost metering beyond disclosure; documentation index updated | §0.2 (both named rejections with reasons, plus three more), §5.4 (explicit no-metering clause); index row added to `docs/meeting-mode-documentation-index.md` §3 in the same change |
