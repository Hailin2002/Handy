# Import Security Specification — Untrusted ZIP/TXT Validation for WhatsApp Imports (Phase 9)

Status: **executable specification — no production code exists yet** (Phase 9 prerequisite).
Source of truth: [`docs/meeting-mode-plan.md`](../../meeting-mode-plan.md) **lines 1463–1468**
(quoted verbatim below) and Phase 9 acceptance line 1688 ("Pfadtraversal, Symlinks und
Dekompressionsbomben erreichen den AssetStore nicht").
Companion artifacts: fixture generator + adversarial corpus under
[`tests/fixtures/import-security/`](../../../tests/fixtures/import-security/) with
machine-readable expected verdicts in `manifest.json`.
Receiving-side contract: [`asset-store-contract.md`](../../meeting-mode-architecture/asset-store-contract.md)
§2.2 (`StorageKey::parse`) and its `commit_import` symlink/reparse refusal.

> Plan §12, lines 1463–1468 (normative source):
> „WhatsApp-Archive und alle importierten Dateien gelten als untrusted input. Vor dem
> Entpacken werden Zielpfade normalisiert; Pfadtraversal, absolute Pfade und Symlinks
> werden abgewiesen. Begrenzungen für Dateianzahl, Einzelgröße, Gesamtgröße und
> Kompressionsverhältnis verhindern ZIP-Bombs. Beschädigte oder verschlüsselte Archive
> werden ohne Teilimport erklärt. Erst nach erfolgreicher Prüfung werden Dateien aus
> einem isolierten temporären Verzeichnis in den AssetStore übernommen."

---

## 1. Scope, threat model, and non-goals

### 1.1 What this specification governs

The **import validator**, the component that sits between an untrusted archive or text
file chosen by the user and the `AssetStore`. It decides exactly one thing per input:
`ACCEPT` (safe to stage fully, then commit) or `REJECT(code)` (nothing is imported,
explanation shown). It owns no UI, no parsing of chat semantics, and no cloud upload.

Inputs covered:

| Input | Examples | Rules |
| --- | --- | --- |
| WhatsApp "Export chat" ZIP archives | `_chat.txt`, `Media/*.jpg`, `.opus`, stickers, nested archives | §3–§7 |
| Plain-text WhatsApp exports | single `.txt` export without media | §8 |
| Any other user-chosen file routed through the meeting import | future sources (plan: "alle importierten Dateien") | §3–§7 apply to containers; §8 applies to bare text |

### 1.2 Threat model

The attacker controls every byte of the import file, including metadata (entry names,
flags, Unix mode bits, declared sizes). Attacker goals and where they die:

| Goal | Attack class | Killed by |
| --- | --- | --- |
| Write outside staging/app-data | zip-slip `../`, drive letters, UNC, absolute paths | §4 R-PATH-* |
| Plant links that later resolve elsewhere | symlink / special-file members | §4 R-MODE-* |
| Exhaust disk / RAM / CPU | zip bombs (ratio bombs, declared-size lies, entry floods) | §5 R-LIMIT-* |
| Bypass limits via recursion | nested archive bombs | §5 R-LIMIT-6 (opaque treatment) + caps |
| Sneak partial content through | encrypted members, corrupted members mid-stream | §6 R-INTEG-* (full-reject rule) |
| Feed binary garbage into the text parser | masquerading `.txt`, invalid encodings | §8 |

Out of scope (owned elsewhere, cited for boundary clarity): chat-format parsing
warnings (parser layer), hash-dedup/idempotent re-import (ingestion service),
prompt-injection defense (Phase 10), AssetStore key grammar (`StorageKey::parse`,
already normative in the asset-store contract §2.2).

### 1.3 Non-goals (complexity budget)

- No recursive extraction of nested archives — inner archives are opaque blobs (§5 R-LIMIT-6).
- No signature-based malware scanning — out of product scope for v1; the validator is a structural gate only.
- No repair attempts on damaged archives ("Beschädigte … werden ohne Teilimport erklärt" — reject-and-explain, never best-effort recovery).
- No password prompt for encrypted exports — WhatsApp exports are never encrypted; encryption means tampering or wrong file → hard reject.
- No per-entry allowlists of media MIME types — unknown media is preserved unanalysed per plan Phase 9 acceptance; the validator does not filter by type.

---

## 2. Validation pipeline overview (normative shape)

```text
file ─► Phase A: container integrity (metadata readable?)
     ─► Phase B: metadata pre-scan (ALL central-directory entries checked)
     ─► Phase C: streamed extraction into isolated staging dir
                   (caps re-enforced incrementally while decompressing)
     ─► ACCEPT: staging dir handed off to AssetStore (§7)
        REJECT: staging dir destroyed, explanation surfaced (§6.4)
```

Hard ordering rule **R-ORDER**: Phases run strictly in order; a Phase-A failure skips
B/C, a Phase-B failure skips C. No extraction byte is ever written before Phase B has
passed. Every violation found in a passed phase is collected and reported together;
the **primary code** is the highest-priority violation under this single global chain
(concretizing the per-group lists of §4.3/§5.3/§6.3; `reference_validator.py`
implements exactly this order):

```text
Phase A: E_CORRUPT_CONTAINER, E_CORRUPT_METADATA
trust:   E_ENCRYPTED
links:   E_SYMLINK, E_SPECIAL_MEMBER
paths:   E_TRAVERSAL, E_ABSOLUTE_PATH, E_RESERVED_NAME, E_INVALID_NAME
limits:  E_TOO_MANY_ENTRIES, E_SINGLE_FILE_TOO_LARGE, E_RATIO_BOMB,
         E_TOTAL_SIZE_EXCEEDED
Phase C (stream order): E_HEADER_MISMATCH, E_TRUNCATED, E_SIZE_MISMATCH,
         E_CRC_MISMATCH
```

This yields one deterministic verdict per fixture (testability requirement G3).

---

## 3. Limits constants (normative values)

All rules below reference these single-source constants; they live in one module so
tests can import them and the corpus can assert against the exact production values.

| Constant | Value | Rationale |
| --- | --- | --- |
| `MAX_ENTRIES` | 10 000 | Largest observed WhatsApp media export stays two orders of magnitude below; anything larger is hostile or pathological. |
| `MAX_FILE_UNCOMPRESSED` | 256 MiB (268 435 456 B) | WhatsApp media ceiling is ~2 GiB device-dependent but chat-shareable media is far smaller; 256 MiB comfortably covers real video notes. |
| `MAX_TOTAL_UNCOMPRESSED` | 2 GiB (2 147 483 648 B) | Matches plan §12's own scale estimate (230 MB/h audio); an import may not dwarf a session. |
| `MAX_COMPRESSION_RATIO` | 200 : 1 | Real chat text ≈ 3–5 : 1, JPEG/MP4 ≈ 1 : 1; 200 : 1 is unreachable for legitimate content. |
| `RATIO_EXEMPT_BELOW` | 4 096 B compressed | Tiny entries (e.g., 30 B → 3 KB of zeros) must not trip the ratio rule; absolute caps still apply to them. |
| `MAX_NAME_BYTES` | 1 024 | Full member name, UTF-8 bytes, after decoding. |
| `MAX_COMPONENT_BYTES` | 255 | Per path component (matches NTFS/ext/APF minimum common denominator). |

Streaming counters (Phase C) enforce the same three size caps **incrementally**: the
validator aborts at the first byte that would exceed a cap — it never buffers a bomb
to measure it.

## 4. Path normalization and rejection rules (plan line 1464)

### 4.1 Decoding (before any comparison)

R-ENC-1 — Member names are decoded from the archive bytes: if the UTF-8 flag
(general-purpose bit 11) is set, decode strict UTF-8 (failure ⇒ `E_CORRUPT_METADATA`);
otherwise decode CP437, the ZIP specification's historical default.
R-ENC-2 — The decoded name is normalized: backslashes (`\`) are treated as path
separators and converted to `/` **before** component analysis, so `..\..\x` cannot
pose as a flat name. NUL bytes and other C0 control characters (< 0x20) in the name ⇒
`E_INVALID_NAME`.

### 4.2 Component rules (applied to the `/`-split components)

| Rule | Condition (after splitting on `/`, dropping empty leading results of split mechanics) | Verdict |
| --- | --- | --- |
| R-PATH-1 | any component equals `..` | REJECT `E_TRAVERSAL` |
| R-PATH-2 | any component equals `.` | component dropped (normalization); if the remaining name is empty ⇒ REJECT `E_TRAVERSAL` |
| R-PATH-3 | name starts with `/` (i.e., first split component was empty) | REJECT `E_ABSOLUTE_PATH` |
| R-PATH-4 | any component contains a `:` — this covers drive-letter prefixes (`^[A-Za-z]:`, i.e. `C:\…` and drive-relative `C:evil`) and NTFS alternate-data-stream syntax (`file.txt:stream`) | REJECT `E_ABSOLUTE_PATH` |
| R-PATH-5 | name starts with `//` (UNC) | REJECT `E_ABSOLUTE_PATH` |
| R-PATH-6 | any component is a Windows reserved device name case-insensitively (`CON`, `PRN`, `AUX`, `NUL`, `COM1`–`COM9`, `LPT1`–`LPT9`), optionally with an extension (`COM1.txt` counts) | REJECT `E_RESERVED_NAME` |
| R-PATH-7 | any component ends with `.` or space (NTFS strips them → redirection hazard), except the bare `.` of R-PATH-2 | REJECT `E_RESERVED_NAME` |
| R-PATH-8 | name longer than `MAX_NAME_BYTES` or any component longer than `MAX_COMPONENT_BYTES` | REJECT `E_INVALID_NAME` |

Defense-in-depth note: R-PATH-1 rejects **every** `..` component even when it would
resolve inside staging. This is deliberate (the plan says traversal "wird abgewiesen",
full stop) and keeps the rule context-free and fuzz-stable.

Directory entries (names ending in `/`) pass through the identical rules; they are
never extracted as files but their names are validated.

### 4.3 Precedence within path violations (primary-code selection)

`E_SYMLINK` → `E_SPECIAL_MEMBER` (§4.4, evaluated first because mode bits decide
whether a name is even materialized) → `E_TRAVERSAL` → `E_ABSOLUTE_PATH` →
`E_RESERVED_NAME` → `E_INVALID_NAME`.

### 4.4 Unix mode rules (symlink and special-member refusal)

R-MODE-1 — For each central-directory entry, the high 16 bits of `external_attr`
are read as a Unix mode. If `(mode & 0o170000) == 0o120000` (S_IFLNK) ⇒
REJECT `E_SYMLINK`. There is no benign-symlink exception: a symlink pointing
inside the archive is rejected with the same code as one pointing out (plan:
"Symlinks werden abgewiesen", unconditionally).
R-MODE-2 — If the mode bits are present and the type is neither S_IFREG (regular
file) nor S_IFDIR ⇒ REJECT `E_SPECIAL_MEMBER` (devices, FIFOs, sockets).
R-MODE-3 — Entries whose `external_attr` high bits are 0 (created without Unix
attrs, e.g., Windows tools) are regular files by definition; no rejection.

## 5. Resource-limit rules (plan line 1465)

### 5.1 Metadata-phase checks (Phase B)

| Rule | Condition | Verdict |
| --- | --- | --- |
| R-LIMIT-1 | central-directory entry count > `MAX_ENTRIES` | REJECT `E_TOO_MANY_ENTRIES` |
| R-LIMIT-2 | any entry's declared uncompressed size > `MAX_FILE_UNCOMPRESSED` | REJECT `E_SINGLE_FILE_TOO_LARGE` |
| R-LIMIT-3 | Σ declared uncompressed sizes > `MAX_TOTAL_UNCOMPRESSED` | REJECT `E_TOTAL_SIZE_EXCEEDED` |
| R-LIMIT-4 | any entry with `uncompressed / compressed > MAX_COMPRESSION_RATIO`, unless `compressed < RATIO_EXEMPT_BELOW` | REJECT `E_RATIO_BOMB` |
| R-LIMIT-5 | declared sizes inconsistent with ZIP64 extras where present ⇒ handled as corruption (`E_CORRUPT_METADATA`, §6) | REJECT |

Declared-size checks fire **before** any decompression — a bomb is refused from its
central directory alone, costing O(entries) work, not O(output).

### 5.2 Streaming-phase re-checks (Phase C)

R-LIMIT-5a — While extracting, the running totals (current entry bytes, aggregate
bytes) are compared against the caps after every chunk. Exceeding either ⇒ abort,
REJECT `E_TOTAL_SIZE_EXCEEDED` / `E_SINGLE_FILE_TOO_LARGE` respectively (this catches
archives whose headers lied small but whose data streams long).
R-LIMIT-5b — Actual uncompressed bytes ≠ declared size for an entry ⇒
REJECT `E_SIZE_MISMATCH` (a lie in either direction is corruption, §6).

### 5.3 Nested archives (counterexample-proofing)

R-LIMIT-6 — Archives are **never recursed into**. An inner `.zip`/`.gz`/`.7z` member
is staged like any other blob. Consequently a nested bomb cannot expand: its inner
explosion would require a second, non-existent extraction pass. The outer archive
must independently satisfy R-LIMIT-1–4 or it is rejected anyway. Fixture
`benign_nested_archive_opaque.zip` pins this expectation as ACCEPT-with-note.

### 5.4 Precedence within limit violations

`E_TOO_MANY_ENTRIES` → `E_SINGLE_FILE_TOO_LARGE` → `E_RATIO_BOMB` →
`E_TOTAL_SIZE_EXCEEDED` → (Phase C) `E_SIZE_MISMATCH`.

## 6. Corrupted and encrypted archives (plan lines 1466–67)

### 6.1 Detection rules

| Rule | Condition | Verdict |
| --- | --- | --- |
| R-INTEG-1 | EOCD record missing/unparseable (truncation, junk suffix beyond tolerance of 64 KiB comment) | REJECT `E_CORRUPT_CONTAINER` |
| R-INTEG-2 | central-directory unreadable: bad signatures, offsets outside file, truncated directory | REJECT `E_CORRUPT_CONTAINER` |
| R-INTEG-3 | any member carries general-purpose bit 0 set (encrypted), regardless of method | REJECT `E_ENCRYPTED` |
| R-INTEG-4 | local header disagrees with central directory (name bytes, method, sizes, CRC) when reached in Phase C | REJECT `E_HEADER_MISMATCH` |
| R-INTEG-5 | CRC-32 of decompressed content ≠ declared CRC | REJECT `E_CRC_MISMATCH` |
| R-INTEG-6 | stream ends before declared size (premature EOF) | REJECT `E_TRUNCATED` |
| R-INTEG-7 | strict-UTF-8 decode failure of a flagged name, unparseable ZIP64 extra | REJECT `E_CORRUPT_METADATA` |

### 6.2 Full-reject rule (no partial import)

R-FULLREJECT — Validation is all-or-nothing **per import file**. One encrypted
member, one bad CRC, or one traversal name rejects the entire archive. The validator
never commits a subset and never skips a bad member to salvage the rest. Rationale:
silent partiality turns integrity signals into data-loss ambiguity; WhatsApp offers
clean re-export, so rejecting is always recoverable for the user.

### 6.3 Precedence

Phase-A codes (`E_CORRUPT_CONTAINER`, `E_CORRUPT_METADATA`) precede everything.
Then `E_ENCRYPTED` → `E_HEADER_MISMATCH` → `E_CRC_MISMATCH` → `E_TRUNCATED`
(stream-order for Phase-C codes).

### 6.4 Explanation requirement ("erklärt")

Every REJECT produces a structured report consumed by the UI:

```json
{
  "decision": "reject",
  "primary_code": "E_TRAVERSAL",
  "violations": [
    {"code": "E_TRAVERSAL", "member": "../evil.txt"}
  ],
  "stats": {"entries_scanned": 42, "bytes_declared": 1832944}
}
```

Each code maps 1:1 to an i18n key (frontend renders localized copy; the backend never
hardcodes user-facing strings):

| Code | i18n key | EN copy sketch |
| --- | --- | --- |
| E_TRAVERSAL | `meeting.import.error.eTraversal` | "An archive entry tries to escape the import folder ('{{member}}')." |
| E_ABSOLUTE_PATH | `meeting.import.error.eAbsolutePath` | "An archive entry uses an absolute path ('{{member}}')." |
| E_SYMLINK | `meeting.import.error.eSymlink` | "The archive contains a symbolic link ('{{member}}'), which imports do not follow." |
| E_SPECIAL_MEMBER | `meeting.import.error.eSpecialMember` | "The archive contains a special file ('{{member}}'), not a regular file." |
| E_RESERVED_NAME | `meeting.import.error.eReservedName` | "'{{member}}' uses a name that Windows reserves." |
| E_INVALID_NAME | `meeting.import.error.eInvalidName` | "An archive entry has an unusable name." |
| E_TOO_MANY_ENTRIES | `…eTooManyEntries` | "The archive holds more than {{limit}} items." |
| E_SINGLE_FILE_TOO_LARGE | `…eSingleFileTooLarge` | "'{{member}}' unpacks to more than {{limit}}." |
| E_TOTAL_SIZE_EXCEEDED | `…eTotalSizeExceeded` | "The archive unpacks to more than {{limit}} in total." |
| E_RATIO_BOMB | `…eRatioBomb` | "The archive expands more than {{ratio}}× its size — likely a decompression bomb." |
| E_ENCRYPTED | `…eEncrypted` | "The archive is password-protected. WhatsApp exports never are — please re-export." |
| E_CORRUPT_CONTAINER / _METADATA | `…eCorruptArchive` | "The archive is damaged and cannot be read completely." |
| E_HEADER_MISMATCH / E_CRC_MISMATCH / E_TRUNCATED / E_SIZE_MISMATCH | `…eCorruptArchive` (shared detail line per code) | "Contents failed verification." |
| E_EMPTY / E_NOT_TEXT / E_TEXT_DECODING (§8) | `…eEmptyFile` / `…eNotText` / `…eTextDecoding` | text-specific equivalents |

## 7. Isolated staging and AssetStore handoff (plan lines 1467–68)

### 7.1 Staging directory

S-STAGE-1 — Location: `<portable-aware app-data>/tmp/import-<uuid-v4>/`, resolved
through the same `portable.rs` helpers the AssetStore contract mandates (its §2.1);
never inside the `assets/` tree, never a system temp dir outside app-data (keeps
rename-based handoff same-volume and honors portable installs).
S-STAGE-2 — Files land as `<6-digit-index>_<sanitized-basename>`; the sanitized
basename is the original name's final component filtered through R-PATH-4/6/7/8 again
(colons included, so no NTFS alternate-data-stream can be smuggled onto a staged
file — defense in depth; the safe relative path is preserved only as import metadata).
Collisions get `-1`, `-2` suffixes.
S-STAGE-3 — SHA-256 is accumulated **during** the Phase-C stream (same algorithm and
format as the asset-store contract §3.2) so the handoff needs no re-read.

### 7.2 Handoff protocol (only after full validation success)

```text
1. Phase A+B+C all pass            → staging holds verified, hashed files
2. for each staged file, in index order:
   AssetStore::put(PutRequest {
       key: assets/<caller-generated asset-id>/original(.<ext>),
       kind: Blob,
       expected_hash: <sha256 from S-STAGE-3>,
       content: PutSource::File(<staging path>) })
   — the store's six-step atomic protocol (contract §5.2) performs the
     same-volume move; commit_import's symlink/reparse refusal
     (contract §2.2) is the independent second gate.
3. original relative member paths are persisted ONLY as metadata
   (assets.original_filename analogue), never as storage-key parts.
4. staging tree deleted after the last successful put AND on every
   failure/cancel path (no exceptions; crash leftovers are swept at
   startup: tmp/import-* older than 24 h are removed).
```

S-HANDOFF-1 — If any `put` fails mid-handoff, the import fails as a whole; already-
committed assets remain (they are content-addressed, unreferenced, and reclaimable)
and the error surfaces verbatim. This is the one asymmetry with R-FULLREJECT: the
handoff phase operates on already-validated content, so its failures are I/O errors,
not trust decisions.

## 8. Bare-text (.txt) export rules (ZIP/TXT parity)

Applies to a user-chosen `.txt` WhatsApp export. The `_chat.txt` extracted from an
archive is *not* subject to this section (archive rules cover it; parsing happens
downstream).

| Rule | Condition | Verdict |
| --- | --- | --- |
| R-TXT-1 | file size > `MAX_FILE_UNCOMPRESSED` | REJECT `E_SINGLE_FILE_TOO_LARGE` |
| R-TXT-2 | 0 bytes | REJECT `E_EMPTY` |
| R-TXT-3 | first 4 KiB contains a NUL byte **(files with a UTF-16 BOM are exempt — that encoding legitimately interleaves zero bytes; they proceed to R-TXT-4)**, or the file begins with a known binary magic (`PK\x03\x04`, `%PDF`, `\x89PNG`, `\xFF\xD8`, `MThd`, `RIFF`, `OggS`, `\x1f\x8b`) | REJECT `E_NOT_TEXT` |
| R-TXT-4 | BOM present (UTF-8 / UTF-16LE / UTF-16BE) → decode accordingly; else strict UTF-8 | decode failure ⇒ REJECT `E_TEXT_DECODING`; no lossy fallback (mojibake silently corrupts evidence timestamps) |
| R-TXT-5 | decoded text with zero parsable message lines | ACCEPT with warning `unparsable_lines=all` (preview shows the warning; import remains local and searchable per plan line 1689) |

## 9. Adversarial fixture corpus (implements G2/G3)

Location: `tests/fixtures/import-security/`. The committed script
`generate_fixtures.py` builds every fixture deterministically (fixed timestamps,
seeded PRNG) and emits `manifest.json` (machine-readable expected verdicts + SHA-256
per fixture) and `MANIFEST.md` (human-readable table). `reference_validator.py` is a
dependency-free Python implementation of §3–§8 used to prove the fixtures behave as
specified; `verify_corpus.py` regenerates, hashes, validates and asserts every
expected verdict, and additionally round-trips the ZipCrypto fixture to prove the
encrypted fixture is genuinely well-formed encryption (not just a flipped flag).

Attack-class coverage matrix:

| Class (gate G2 wording) | Fixtures (expected verdict) |
| --- | --- |
| Traversal | `attack_traversal_basic.zip` → E_TRAVERSAL · `attack_traversal_deep_mixed.zip` → E_TRAVERSAL · `attack_traversal_backslash.zip` → E_TRAVERSAL |
| Absolute path | `attack_absolute_unix.zip`, `attack_absolute_windows.zip`, `attack_absolute_unc.zip` → E_ABSOLUTE_PATH · `attack_ads_colon.zip` (`notes.txt:hidden`, NTFS ADS) → E_ABSOLUTE_PATH |
| Symlink | `attack_symlink_escape.zip` (target escapes), `attack_symlink_internal.zip` (benign target) → both E_SYMLINK · `attack_special_member.zip` → E_SPECIAL_MEMBER |
| Bomb ratio | `attack_ratio_entry.zip` → E_RATIO_BOMB · `attack_single_file_oversize.zip` → E_SINGLE_FILE_TOO_LARGE · `attack_total_size.zip` → E_TOTAL_SIZE_EXCEEDED · `attack_too_many_entries.zip` → E_TOO_MANY_ENTRIES · `benign_nested_archive_opaque.zip` → ACCEPT (opaque, R-LIMIT-6) |
| Encrypted | `attack_encrypted_zipcrypto.zip`, `attack_encrypted_partial.zip` → E_ENCRYPTED (partial one proves R-FULLREJECT) |
| Corrupt | `corrupt_truncated_eocd.zip` → E_CORRUPT_CONTAINER · `corrupt_central_directory.zip` → E_CORRUPT_CONTAINER · `corrupt_crc_stored.zip` → E_CRC_MISMATCH · `corrupt_local_header.zip` → E_HEADER_MISMATCH |
| Benign accept-path | `benign_minimal.zip`, `benign_media.zip`, `benign_unicode_utf8.zip`, `benign_cp437_legacy.zip`, `benign_dotfile.zip` → ACCEPT |
| Reserved/invalid names | `attack_reserved_name.zip` → E_RESERVED_NAME |
| Text rules | `benign_chat_utf8_bom.txt`, `benign_chat_utf16le.txt` → ACCEPT · `benign_chat_no_timestamps.txt` → ACCEPT+warning · `attack_txt_empty.txt` → E_EMPTY · `attack_txt_binary_masquerade.txt` → E_NOT_TEXT · `attack_txt_invalid_utf8.txt` → E_TEXT_DECODING |

Exact numeric expectations (codes, decisions, warnings) live in `manifest.json`;
that file is the single arbiter for G3. Large fixtures are built at true production
scale (§3 constants) — the ratio-bounded construction (each 8 KiB block starts with
seeded random bytes so deflate's 32 KiB window cannot collapse blocks onto each
other; sustained ratio ≈ 105:1) keeps them small on disk while ensuring only the
intended limit rule fires per fixture. `python verify_corpus.py` regenerates,
hashes, validates and asserts every expected verdict in one command and is the
corpus's continuous acceptance gate.

## 10. Gate traceability

| Gate | Where proven |
| --- | --- |
| G1 — every plan-line 1463–1468 requirement mapped to a concrete testable rule | §4 (line 1464: normalization/traversal/absolute/symlink → R-PATH-*, R-MODE-*), §5 (line 1465: four limits → R-LIMIT-1–4), §6 (lines 1466–67: corrupt/encrypted full-reject → R-INTEG-*, R-FULLREJECT), §7 (lines 1467–68: isolated staging → handoff → S-STAGE-*, S-HANDOFF-1); each rule states condition + expected verdict |
| G2 — ≥1 fixture per attack class from a committed script | §9 matrix; `generate_fixtures.py` committed; corpus generated by running it (see mission evidence) |
| G3 — exact expected validation verdict per fixture | `manifest.json` `expected.decision` + `expected.primary_code` (+warnings); asserted mechanically by `verify_corpus.py` |
| G4 — no production code modified | All artifacts under `docs/reviews/import-security-spec/` and `tests/fixtures/import-security/`; `git status` shows nothing else touched |
