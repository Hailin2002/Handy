# Schema-Versioning & Forward-Compatibility Policy (Meeting Mode)

Status: **spec contract — no implementation yet** (Phase 2 prerequisite)
Source of truth: `docs/meeting-mode-plan.md` §9 line 1154 ("Schema- und
Parser-Versionen für reproduzierbare Migrationen"), §9 lines 1149–1162
(v1 record rules), §7/§9 persistence sections as cited per artifact below.
Authority order: plan beats contracts, contracts beat catalogs, nothing beats
actual code — identical to the documentation index §4.

This document defines **one policy across all versioned meeting-mode
artifacts**: the session `manifest.json`
([phase2-contracts](../meeting-mode-phase2-contracts.md) §2,
[asset-store-contract.md](asset-store-contract.md) §6), export bundles
([export-project-dataflow.md](export-project-dataflow.md) §4/§5), future
import formats ([import-pipeline-dataflow.md](import-pipeline-dataflow.md),
[ingestion-pipeline-contract.md](ingestion-pipeline-contract.md)), and the
derived internal artifacts that already pin versions
([search-index-contract.md](search-index-contract.md) `search_index_meta`,
SQLite `user_version` migrations per
[data-migration-operator-guide](../meeting-mode-data-migration-operator-guide.md)).
It changes no source code and does not rewrite the defining contracts; where a
defining contract already pins a rule (e.g. phase2-contracts §2 rule 1
"Unknown fields MUST be ignored"), this policy adopts it verbatim and forbids
deviating elsewhere.

Sibling contracts consulted (read-only for this policy):
`session-state-contract.md` (SSC), `coexistence-contract.md`,
`summary-evidence-contract.md` (SEC), `project-snapshot-contract.md` (PSC),
`identifier-registry.md`.

---

## 1. Scope: what is a versioned artifact

An artifact is **versioned** under this policy iff bytes written by one build
of the app may later be read by another build (same machine after update,
another machine, a restored backup, or a future sync/import target). The
versioned set:

| Abbrev. | Artifact                    | Location                              | Defining contract                                        | Version field(s) today                                  |
| ------- | --------------------------- | ------------------------------------- | -------------------------------------------------------- | ------------------------------------------------------- |
| SM      | Session `manifest.json`     | `meetings/<meeting-id>/manifest.json` | phase2-contracts §2; asset-store-contract §6             | `manifest_version` (+ `parser_min_version`)             |
| EJ      | Export JSON document        | `<export-dir>/*.json`                 | export-project-dataflow §4.2                             | `export_format` type tag + `export_version`             |
| EM      | Export bundle manifest      | `<export-dir>/export-manifest.json`   | export-project-dataflow §4.4                             | `export_manifest_version`                               |
| ETX     | Text exports (Markdown/TXT) | `<export-dir>/*.md`, `*.txt`          | export-project-dataflow §4.1–4.2                         | `Export-Version` header line / provenance envelope      |
| IMP     | Import source formats       | user-provided files                   | ingestion-pipeline-contract; import-pipeline-dataflow §3 | `parser_version` (semver, in-process); none on disk yet |
| SI      | Search-index meta           | `search_index_meta` table             | search-index-contract §5                                 | `index_version` key                                     |
| DB      | SQLite schemas              | app database file                     | plan §9; operator guide §"migrations"                    | `user_version` pragma                                   |

Out of scope (governed elsewhere, listed to prevent scope creep): Tauri event
payloads (`live-view-streaming-contract.md` — in-process, never persisted),
settings store shape (`session-state-contract.md`, existing
`tauri-plugin-store`), WAV chunk payloads (fixed PCM16LE framing, phase2 §1),
asset blob content (content-addressed by hash, asset-store §4).

---

## 2. Required version fields (uniform rule VFR)

**VFR-1 — Exactly one integer schema version per serialized artifact.**
Every versioned JSON artifact carries exactly one top-level member whose name
ends in `_version` with a **positive non-zero unsigned integer** value
(`manifest_version`, `export_version`, `export_manifest_version`). Future
import formats MUST embed `schema_version` with the same constraints **before
the first release that writes them** (plan line 1154). Booleans, strings
(`"1"`), floats and `0` are invalid version values.

**VFR-2 — Type tag accompanies the version.** Every self-describing file also
carries a stable type-tag member read _before_ the version
(SM: `"schema": "handy.meeting-manifest"` per AS §6.2; EJ:
`"export_format": "speak-meeting-export"` per EPD §4.2). A file whose type tag
does not match the reader's expectation is rejected regardless of its version
value — versions are only comparable within one type.

**VFR-3 — Floor member.** SM additionally persists `parser_min_version`
(phase2 §2 line 95: "lowest version a reader accepts"): the minimum
`manifest_version` that any reader of this artifact generation is required to
support. It is a writer-declared requirement statement that travels with the
file; it never raises a reader's own capability. New multi-release mutable
artifacts adopt the same pattern (`<field>_min_version`); single-release
immutable artifacts (EJ, EM, ETX) do not need it because their version never
moves retroactively.

**VFR-4 — Version constants are named, not inlined.** Writers emit named
constants (planned home beside the serializer, e.g.
`SESSION_MANIFEST_VERSION: u32 = 1`), never literals scattered through
serialization code. A test asserts each emitted constant equals the value
documented in §7 of this policy.

**VFR-5 — Version is read first, enforced always.** A reader parses the
version member before interpreting any other content and dispatches per the
§4 matrix. Reading content "anyway" and checking the version afterwards is a
contract violation (it silently normalizes forward-incompatible data).

---

## 3. Change classes: minor vs. major

The integer encodes the **major** class only. Minor changes do not bump the
integer; they must be recorded in the owning contract's version-history note
(§6, drift ban).

### 3.1 Minor class (no version bump)

- Adding an **optional** object member at any nesting level (absent ⇒ default).
- Adding provenance/metadata that consumers may ignore (EPD §5 envelope
  extensions, additional display-only timestamps).
- Documentation clarifications that do not move any byte on disk.
- Tightening validation of values that were previously documented as
  unspecified, **provided** no in-the-wild artifact can violate the tightened
  rule (if one can, it is major).

### 3.2 Major class (version bump N → N+1)

- Removing, renaming, retyping or repurposing any existing member.
- Changing field semantics, units, coordinate systems or ranges
  (e.g. timeline ms → µs; half-open → closed ranges).
- Adding or removing a value of a **closed enumeration**
  (`state`, `kind`, `source`, `reason`, …) — closed enums are contracts;
  see UFR-2 for why additions are not free.
- Changing the write protocol or atomicity guarantees (SM §6.3 tmp/rename
  pattern, EM member hashing rules).
- Splitting one artifact into several, or merging several into one.
- Dropping reader support for an older major (raises the effective floor;
  see §5).

### 3.3 Uniform unknown-\* rule (decided once)

- **UFR-1 (unknown object members — tolerate).** All readers of all versioned
  artifacts MUST ignore object members (at any depth) they do not know.
  Struct deserialization for versioned types therefore never uses
  deny-unknown-fields semantics. This adopts phase2-contracts §2 rule 1
  verbatim and extends it to every other artifact in scope.
- **UFR-2 (unknown closed-enum values — reject).** A value outside a
  documented closed enumeration is not an "unknown field"; it is a broken or
  future-major assumption. Readers reject the containing element with a named
  error; for SM a chunk row with unknown `state` is treated like `damaged`
  pending recovery classification (fail-safe, never consumed silently —
  phase2 §6 readiness rule stays intact). Guessing or defaulting unknown enum
  values is forbidden everywhere.
- **UFR-3 (preserve-on-rewrite).** Writers of _mutable_ artifacts (SM is the
  only one today) MUST carry unknown members through atomic rewrites instead
  of dropping them, so an upgrade/downgrade cycle cannot destroy data a newer
  build stored. Implementation shape: retain the parsed unknown members of the
  previous manifest generation alongside the DB-projected fields. Writers of
  immutable artifacts (EJ, EM, ETX) have no rewrite step and thus no
  preservation duty.
- **UFR-4 (no downgrade writes).** No build writes an artifact version lower
  than its own constant. Older builds simply refuse newer artifacts (§4);
  they never "convert down".

---

## 4. Reader compatibility matrix (G1)

Reader capabilities are the compile-time pair `(min_supported, max_known)` of
the artifact version. Defaults per §5: `max_known = current`, and
`min_supported` covers exactly the predecessor major while a migration exists.

| Case                       | Condition                                                  | Verdict                        | Behavior                                                                                                                                                                                 |
| -------------------------- | ---------------------------------------------------------- | ------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| R1 exact match             | `artifact == max_known`                                    | **accept**                     | normal path                                                                                                                                                                              |
| R2 older in-window         | `min_supported ≤ artifact < max_known`                     | **accept (warn)**              | run migration-on-read (§5) once, then proceed; log one warning naming both versions                                                                                                      |
| R3 newer than reader       | `artifact > max_known`                                     | **reject**                     | named error (`schema_version_too_new` class); never guess, never partially read; UI offers update-path copy, not data loss wording                                                       |
| R4 older than floor        | `artifact < min_supported`                                 | **reject**                     | named error (`schema_version_unsupported`); migration not shipped for this span                                                                                                          |
| R5 missing/garbage version | unparseable, zero, or wrong type tag (VFR-2)               | **reject**                     | treat as foreign/corrupt file; recovery flows of the owning contract take over (SM: AS §7 crash-residue handling)                                                                        |
| R6 unknown members only    | any accepted case + unknown object members present         | **accept (warn)**              | ignore per UFR-1, preserve per UFR-3; warn-level log once per file                                                                                                                       |
| R7 unknown enum value      | closed-enum violation inside an otherwise accepted version | **reject element / fail safe** | per-artifact rule: SM chunk row → `damaged`-classification; EM member → bundle rejected before consumer trust; EJ segment → segment dropped with named warning, rest of document renders |

Concrete older/newer-build enumerations (gate G1):

- **Older build reads newer artifact:** SM v2 file, build knows ≤ v1 → R3
  reject; startup scan leaves the session untouched except AS-§7 residue
  rules and surfaces `schema_version_too_new`. Same for EM/EJ: bundle is not
  offered for import/consumption.
- **Newer build reads older artifact:** SM v1 file, build knows ≤ v2 → R2:
  migrate-on-read, warn, continue; rewritten manifest (AS §6.3 update points)
  is written as v2 **with unknown v1-era members preserved** (UFR-3) and
  `parser_min_version` still ≥ the oldest shipped reader (VFR-3).
- **Same version, newer minor content:** v1 file containing members added in
  a later minor → R6 accept-with-warning.
- **Foreign file:** wrong type tag or no version → R5 reject.

---

## 5. Migration-on-read limits

- **MIG-1 — Window.** Each release supports reading the current version and,
  where a migration is implemented, exactly one predecessor major. Two-step
  jumps (v1 file, current v3) are rejected per R4. Rationale: migrations are
  tested pairs; unbounded chains turn every release into a format archaeology
  tool (plan simplicity principle, §16).
- **MIG-2 — Idempotent, pure, all-or-nothing.** A migration is a pure
  function `old bytes → new in-memory representation`; it either fully
  succeeds or yields a named error. It never writes a partially converted
  artifact; a persistent rewrite happens only through the artifact's own
  atomic write protocol (SM: §6.3 tmp→fsync→rename).
- **MIG-3 — DB stays authoritative.** Where SQLite and a versioned file both
  hold a fact, migration repairs only the derived artifact, never the
  authoritative rows (SSC §6.1: manifest is a cross-check artifact only).
  A migration failure therefore degrades to the owning contract's recovery
  flow, not to data loss.
- **MIG-4 — Immutable artifacts are never migrated.** EJ/EM/ETX are written
  once by one build; a reader outside their version rejects (R3/R4) rather
  than converting. Import sources (IMP) are likewise never rewritten — the
  importer records `parser_version` on the produced rows (plan §9:
  `ingestion_state`, `parser_version`, `content_hash`) so a re-parse under a
  newer parser creates new revisions instead of mutating history
  (ingestion-pipeline K3/K4 pattern).
- **MIG-5 — Migrations are named and logged.** Each shipped migration has an
  entry in the owning contract's version history (§6) and emits one log line
  per invocation. Silent conversions are format drift (§6).

SQLite note: the existing `user_version`-tracked migration chain
(`src-tauri/src/managers/history.rs:99-136`, cited in the operator guide)
already satisfies MIG-2/MIG-5 for DB schemas; meeting-mode tables reuse the
same mechanism per plan §9 rather than inventing a second one.

---

## 6. Ban on implicit format drift

- **DRIFT-1 — No serialization change without a decision record.** Any change
  to a serialized shape (field set, enum set, units, protocol) requires, in
  the same PR: (a) the version-class decision (minor/major, §3), (b) an entry
  in the owning contract's version-history note, (c) an update of the §7
  registry row in this policy when the current value moves.
- **DRIFT-2 — Constants over literals** (restates VFR-4): a serializer that
  conditionally emits different shapes per feature flag without a version
  decision is a defect even if every individual output parses.
- **DRIFT-3 — Tests pin the emitted version.** At least one test per writer
  asserts the emitted version/type-tag pair against the §7 registry values;
  a failing test blocks release, not a documentation edit after the fact.
- **DRIFT-4 — Reader strictness is code-reviewed.** Deserializers for
  versioned types are reviewed against UFR-1/UFR-2: tolerate unknown members,
  reject unknown enum values, check version first (VFR-5).
- **DRIFT-5 — No undocumented third writers.** Only the owning contract's
  designated writer produces an artifact; debug tools or one-off scripts that
  emit meeting-mode artifacts must go through the same serializer and version
  constants.

### 6.1 Version-history notes (living subsections)

Owning contracts append entries in this style when bumping or extending:
`<artifact> v<N> (<date>, <PR>): <change>, <class>`.

- SM v1 (2026-08-22, phase2-contracts §2 + asset-store-contract §6.2):
  initial draft; `manifest_version = 1`, `parser_min_version = 1`.
- EJ v1 / EM v1 / ETX v1 (2026-08-22, export-project-dataflow §4/§5):
  initial draft.
- IMP: no on-disk bundle format defined yet; `wa1-`/`man1-` key-grammar
  prefixes act as the grammar version per ingestion-pipeline K4.

**Adjudication note (registered observation, owners own the fix):**
phase2-contracts §2 and asset-store-contract §6.2 both specify the _same_
session `manifest.json` as v1 with differing field supersets (e.g. AS adds
`schema` type tag, `session_snapshot`, `integrity.self_hash_sha256`;
phase2 adds `parser_min_version`). This policy pins only the version-field
semantics — both drafts describe **version 1**, and their union must be
reconciled into one normative schema before Phase 2 implementation. Until
reconciled, implementers must treat AS §6.2 (the richer, later, AssetStore-
owned draft) as the byte-format authority and phase2 §2 as the semantic
source for timeline/chunk rules, consistent with the documentation index §4
authority ordering.

---

## 7. Registry: current version values and change-class examples (G3)

| Artifact                         | Field                                         | Current value             | Writer constant (planned)      | Minor-change example                                                       | Major-change example                                         |
| -------------------------------- | --------------------------------------------- | ------------------------- | ------------------------------ | -------------------------------------------------------------------------- | ------------------------------------------------------------ |
| SM `meetings/<id>/manifest.json` | `manifest_version` (+`parser_min_version`)    | `1` (`1`)                 | `SESSION_MANIFEST_VERSION`     | add optional `drift.notes` string member                                   | rename `timeline_start_ms`; add `paused` state to chunk enum |
| EJ export JSON                   | `export_version` (+type tag `export_format`)  | `1`                       | `EXPORT_JSON_VERSION`          | add optional `summary.rerun_generation`                                    | remove per-segment `id`; ms → µs in `start_ms/end_ms`        |
| EM `export-manifest.json`        | `export_manifest_version`                     | `1`                       | `EXPORT_MANIFEST_VERSION`      | add optional `created_by_build` member                                     | change `members[]` hashing rule (add salted hash)            |
| ETX Markdown/TXT                 | `Export-Version` header / provenance envelope | `1`                       | `EXPORT_TEXT_VERSION`          | extra header metadata line                                                 | reorder mandatory header lines; drop `Quelle-ID`             |
| IMP import bundles (future)      | `schema_version` (mandated pre-release)       | n/a yet                   | `IMPORT_BUNDLE_SCHEMA_VERSION` | optional new member                                                        | new container layout; new closed-enum reason values          |
| SI `search_index_meta`           | `index_version` key                           | `1`                       | `SEARCH_INDEX_VERSION`         | — (per search-index NG-8/§8.3 every corpus/tokenizer change bumps ⇒ major) | added corpus field or tokenizer-spec change ⇒ one rebuild    |
| DB                               | `user_version` pragma                         | chain per migration guide | migration modules              | —                                                                          | every schema migration step                                  |

Change-class examples above are illustrative applications of §3, not new
norms; the owning contracts remain authoritative for their fields.

---

## 8. Non-goals and complexity budget (G4)

- **NG-1 — No generic schema/serialization framework.** No schema registry
  service, no runtime JSON-Schema validation layer, no reflection-based
  codec DSL. Typed structs + serde + the §2 rules are the whole mechanism.
- **NG-2 — No semver inside file version fields.** Existing artifacts pin
  single integers; introducing `major.minor.patch` strings in files would
  create a second comparability regime. In-process `parser_version` semver
  (IMP) stays as-is and is never serialized into meeting-mode artifacts
  beyond provenance metadata.
- **NG-3 — No automatic upgrade pipelines across many majors.** MIG-1's
  one-major window is deliberate; no chained auto-migrator, no migration
  marketplace.
- **NG-4 — No downgrade/export-to-old-version feature.** UFR-4; users who
  need portability get exports (EJ/EM), which are versioned independently of
  the live store.
- **NG-5 — No cross-app or cloud format negotiation.** No capability
  handshake protocols, no format-negotiation headers; the reader matrix is
  total and local.
- **NG-6 — No backfilling version fields onto legacy dictation artifacts.**
  `transcription_history` predates meeting mode and keeps its own
  `user_version` chain; this policy does not retrofit it.
- **NG-7 — No preservation guarantee for unknown enum values.** UFR-3
  preserves unknown _members_, not unrecognized _values_; rejecting those
  fail-safe is intended, not a gap.

---

## 9. Acceptance-gate mapping

| Gate                                                    | Where satisfied                                                                                          |
| ------------------------------------------------------- | -------------------------------------------------------------------------------------------------------- |
| G1 reader compatibility matrix                          | §4 matrix R1–R7 incl. older/newer-build enumerations                                                     |
| G2 unknown-field rule decided once, applied uniformly   | §3.3 UFR-1–UFR-4 (tolerate members, reject closed-enum values, preserve on rewrite, no downgrade writes) |
| G3 current version + change-class examples per artifact | §7 registry; §6.1 history notes; adjudication note for the two SM drafts                                 |
| G4 non-goals + documentation index updated              | §8 NG-1–NG-7; row in `docs/meeting-mode-documentation-index.md` §3 and reading-order entry               |
