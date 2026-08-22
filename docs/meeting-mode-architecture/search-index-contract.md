# Search Index Contract — SQLite FTS5 Seam (plan §9 „Suche")

Status: **spec contract — no implementation yet** (Phase 3 prerequisite)
Source of truth: `docs/meeting-mode-plan.md` §9 „Suche" (plan lines 1327–1332),
§9 preamble change envelope (plan lines 1149–1162), §5 `KnowledgeStore`
(plan lines 269–275), §6.16 project search (plan line 739), §6.13 transcript-tab
search (plan line 673), §13 Phase 3 acceptance (plan line 1548: „stabile IDs,
Revisionen, Löschmarker, `change_log` und FTS5-Suche"; plan line 1658:
„projektbezogene FTS5-Suche und Sprung zum Quellenbeleg"), §17 DoD / test
matrix M20 (plan line 1902: „Suche springt zur richtigen Chatnachricht oder
Meeting-Zeitposition").

This document defines the full-text index seam below the domain services. It
changes no source code.

Sibling contracts consumed by this document (read-only):

- [repository-boundary-contract.md](repository-boundary-contract.md) — the
  repository seam, units of work (§4), change-envelope semantics (§5),
  SQL confinement (§7.1). Below abbreviated **RBC**.
- [asset-store-contract.md](asset-store-contract.md) §9 — the three monotone
  meeting-delete levels and their row effects. Below abbreviated **ASC**.
- [ingestion-pipeline-contract.md](ingestion-pipeline-contract.md) §7 —
  transport firewall; FTS indexes `NormalizedItem` projections only. Below
  abbreviated **IPC**.
- [export-project-dataflow.md](export-project-dataflow.md) §3 — canonical
  `MeetingPositionRef`. Below abbreviated **EPD**.
- [module-placement-map.md](module-placement-map.md) — places the search seam
  on `KnowledgeStore::search` itself (its §"AzureMeetingClient/SearchService"
  note); no separate `SearchService` component exists in v1.

Conflict rule (identical to the documentation index): the plan wins over this
contract, this contract wins over catalogs, and nothing wins over actual code.

---

## 0. Scope, non-goals, and schema authority

### 0.1 In scope

1. The **population mechanism** decision: explicit change-envelope-driven
   indexer stage vs. SQL triggers — decided in §2 (gate G1).
2. The **logical index shape**: one FTS5 virtual table over the six indexed
   corpora, with identity and metadata columns (§3, §4).
3. The **German tokenization/diacritics approach** and the exact user-query
   translation rules (§5).
4. The **query interface**, exposed exclusively through the repository seam
   (`KnowledgeRepository::search`), returning stable entity references plus
   highlighted fragments — never raw row internals (§6, gate G2).
5. The **deletion cascade behavior** for soft deletes, hard retention deletes,
   and parent-entity cascade (§7, gate G3).
6. The **rebuild-from-source guarantee** and drift detection (§8).

### 0.2 Explicitly out of scope (non-goals and complexity budget)

These exclusions are binding for v1. Each names the reason it is rejected;
re-opening any of them requires a documented decision against this budget
(gate G4).

| # | Excluded | Reason |
| --- | --- | --- |
| NG-1 | Ranking ML, learning-to-rank, tuned relevance models, user/personalized ranking | BM25 (`ORDER BY rank`) is sufficient for a local library of meeting transcripts and chats (plan lines 1329–1332 deliberately defer anything beyond FTS5). |
| NG-2 | Fuzzy matching beyond the mechanisms stated in §5.2 (tokenizer-inherent case/umlaut folding, automatic prefix matching per §5.3 Q4, deterministic `ß`↔`ss` term expansion per §5.3 Q5) | No edit distance, no Levenshtein, no trigram index, no `spellfix1`, no synonym tables. Every "did you mean" feature is deferred; the included mechanisms are deterministic, bounded, and unit-testable. |
| NG-3 | Cross-device search, server-side search, search over sync | Single-device product in v1 (plan line 85: „zentrale Cloud-Synchronisation oder Mehrgerätebetrieb" is explicitly out; plan lines 102–103 defer server DB and multi-device access); search reads local SQLite only. |
| NG-4 | Embedding/vector search | Plan lines 1329–1332 keep it behind a future `SearchService`; roadmap item (plan line 1753). Nothing in v1 may add vector columns, model downloads, or embedding jobs. |
| NG-5 | Stemming (porter/snowball), German compound splitting/decompounding, lemmatization | Not available in the bundled SQLite FTS5 build beyond the porter stemmer (English-oriented); decompounding is an unsolved research problem. Prefix matching (§5.3 Q4) is the stated mitigation for trailing compounds. |
| NG-6 | Boolean query syntax (AND/OR/NOT/NEAR/column filters) exposed to users | Users get plain word search; the store builds the FTS5 expression itself (§5.3). Raw FTS5 syntax from callers never reaches `MATCH`. |
| NG-7 | Deep pagination of results | Fixed page: `limit` clamped to `[1, 200]`, default 50. Refining the query replaces paging. |
| NG-8 | Indexing of media content (OCR, audio fingerprinting), asset filenames, `author_label`, meeting descriptions, snapshot evidence excerpts | Not named by plan line 1329. The corpus table (§3) is versioned; adding a field later bumps `index_version` and triggers one rebuild (§8.3). |
| NG-9 | FTS over `meeting_sessions.title` | Plan line 1329 does not list meeting titles. The Meetings-page search box (plan line 469) filters via the ordinary `list_sessions` read path (SQL `LIKE` on title inside `MeetingStore`) — it is list filtering, not full-text search. This keeps meeting-list behavior independent of index freshness. |
| NG-10 | Other-language tokenizers (ICU tokenizer, per-language index selection) | `unicode61` handles the plan's target language German acceptably (§5) and degrades gracefully for mixed German/English; ICU is not part of the bundled SQLite build. |
| NG-11 | Saved searches, search history, analytics | No v1 requirement anywhere in the plan; pure scope creep. |

### 0.3 Schema authority

Identical to RBC §0.1 item 1: this document fixes the **logical** index shape
(corpus, columns-as-roles, identity, metadata). The concrete DDL — including
the `CREATE VIRTUAL TABLE … USING fts5(…)` statement and the index-meta table
— belongs to **task-0020 / `docs/meeting-mode-architecture/db-schema.md`**,
which must implement the shape specified here. If they disagree, `db-schema.md`
wins for DDL questions and this contract wins for population, query, deletion,
and rebuild semantics.

### 0.4 Locally verifiable crates

The contract requires **no new dependencies**. Everything specified here is
implementable with the already-pinned `rusqlite = { version = "0.37",
features = ["bundled"] }` (`src-tauri/Cargo.toml` line 73). Verified
empirically in this worktree (probe program compiled offline against the
Cargo.lock-pinned `libsqlite3-sys 0.35.0` bundled build):

```text
sqlite_version=3.50.2          — bundled build
fts5_table_create=ok           — SQLITE_ENABLE_FTS5 is set
                                 (libsqlite3-sys-0.35.0/build.rs line 132)
query='munchen'      hits=1    — umlaut folding ä/ö/ü works (unicode61 remove_diacritics 1)
query='grusse'       hits=0    — 'ß' is NOT equivalent to 'ss' (see §5.2)
prefix_query='"projekt" *' hits=1 — prefix matches compound "Projektmaßnahme"
after_delete hits=0            — DELETE on the index table removes hits immediately
snippet="Grüße aus \u{e000}München\u{e001} wieder" — snippet() with custom markers
```

Probe source and run: temporary cargo project outside the workspace,
22.08.2026, `cargo run --offline` with the workspace's pinned versions.

---

## 1. Position in the architecture

Plan lines 205–214 fix the layers; the FTS index lives entirely below the
repository seam:

```text
React-Oberfläche
  -> Tauri-Commands und Events        (search command handlers)
    -> Domain-Services                (ProjectManager / MeetingManager read paths)
      -> Repository-Schnittstellen    (KnowledgeRepository::search)   <-- QUERY SEAM
        -> SQLite FTS5 search_index                                   <-- THIS CONTRACT (tier D, RBC §3 row 17)
```

Rules inherited unchanged from RBC:

- **S1 — seam exclusivity.** Only the two stores execute SQL touching the
  index; services and commands see typed values (RBC §7.1 R1/R3); the frontend
  only `invoke()`s commands (RBC §7.1 R5). There is no second query path.
- **S2 — no separate `SearchService` in v1.** Plan line 1331 names
  `SearchService` only as the *future* seam for vector search. In v1 the
  search surface is `KnowledgeRepository::search` (RBC §2 sketches it);
  module-placement-map confirms the seam sits on `KnowledgeStore` itself.
  A future `SearchService` wraps this method set without replacing it.
- **S3 — tier D.** The index is derived data (RBC §3 row 17, RBC §4.1 W5):
  never independently writable, always maintained alongside its source
  mutation, always rebuildable (§8).

## 2. Population mechanism — DECIDED: explicit change-envelope-driven indexer stage (gate G1)

### 2.1 Decision

Index population happens through an **explicit indexer stage**: a crate-internal
module `src-tauri/src/managers/search_index.rs` exposing functions that receive
`&rusqlite::Transaction` plus the change envelope of the just-mutated record
(`id`, `revision`, `deleted_at`, and the content to index) and apply the index
row inserts/updates/removals **inside the caller's unit-of-work transaction**.
Every store unit of work that mutates an indexed corpus (§3) calls the stage
as its last step, implementing RBC rule W5 ("FTS updates ride along") —
same transaction, same commit, no background worker, no queue, no eventual
consistency.

SQL triggers (`CREATE TRIGGER … AFTER INSERT ON meeting_transcript_segments …`)
are **rejected**.

### 2.2 Consistency argument against RBC §5 (change-envelope semantics)

1. **Single authority for the envelope.** RBC §5 fixes `id`/`revision`/
   `updated_at`/`deleted_at` semantics once, in Rust, enforced by the stores'
   unit-of-work enumeration (W1–W7). Triggers would duplicate the tombstone
   and revision logic as SQL trigger bodies — a second, divergent
   implementation of exactly the semantics §5 declares authoritative. With the
   explicit stage there is exactly one consumer of the envelope, and it is the
   same code path the tests exercise.
2. **Tombstones are data, not absence.** RBC §5.4: "consumers never infer
   deletion from absence." The FTS index is precisely such a consumer. The
   stage reacts to `deleted_at = Some` on the committed envelope by removing
   index rows (rule D1, §7) — deletion is *propagated*, not discovered. A
   trigger on the base table would have to re-derive the transition
   (`OLD.deleted_at IS NULL AND NEW.deleted_at IS NOT NULL`) instead of being
   handed the fact.
3. **Crash consistency inherits W7 for free.** Because the stage writes in the
   caller's transaction (W1/W5), a commit is atomic across base rows,
   `change_log` rows, and index rows. Restart recovery therefore needs no
   index-specific logic: any visible index row belongs to a committed,
   live base row (orphan invariant D5, §7.1); a crashed unit of work leaves
   neither. A trigger-based design has the same atomicity but none of the
   enumerability — see next point.
4. **Auditability of units of work.** RBC §4.2 enumerates every unit of work
   with its transaction contents. Hidden trigger writes are invisible in that
   enumeration and cannot be exercised through the store seam that RBC §7.3
   tests (in-memory SQLite, injected fakes). The explicit stage keeps every
   tier-D write attributable to a named unit (U8, U9, K1–K9) and directly
   unit-testable.
5. **Hard-delete levels exist.** ASC §9 defines retention-driven **hard**
   `DELETE`s of `meeting_transcript_segments`, `meeting_summaries`,
   `sources`/`source_items` (levels 2–3) — including child cascades across
   tables. Expressing multi-table cascade as chained triggers spreads
   coupling through DDL owned by a different authority (task-0020); the
   explicit stage states the cascade once, in reviewed Rust (§7.2).
6. **Rebuild/drift machinery needs the revision anyway.** Drift detection
   (§8.2) compares the envelope `revision` stored per index row against the
   base row. The stage is the natural writer of that pairing; a trigger
   design would bolt the revision bookkeeping on afterwards.

### 2.3 Consequence for RBC §7.1 rule R1 (disclosed, read-only sibling)

`managers/search_index.rs` contains `rusqlite` code and therefore must be
added to the R1 SQL-confinement allow-list as item 6:

> `src-tauri/src/managers/search_index.rs` (tier-D indexer stage, called only
> inside U8/U9/K1–K9 transactions per search-index-contract §2)

This is a **required one-line amendment to
`repository-boundary-contract.md` §7.1 R1**; that file is read-only in this
task's scope, so the amendment is recorded here and flagged to the supervisor.
Fallback if the amendment is rejected: implement the stage as two private
modules inside the already-allow-listed `meeting_store.rs` /
`knowledge_store.rs` sharing only pure (non-SQL) helpers. Functionally
identical; only the file layout changes.

Related ruling (**N1 — ownership-matrix row 17 interpretation**): RBC §3 row
17 assigns the FTS index to `KnowledgeStore`. Read together with W5 (ride-along
maintenance inside the mutating store's transaction), this contract interprets
row 17 as: **KnowledgeStore owns the search surface** — the `search` query
method, index lifecycle (creation via migration, meta/versioning, rebuild,
§8) — while **row-level maintenance rides along in whichever store owns the
mutated corpus row** (`MeetingStore` for U8/U9, `KnowledgeStore` for
K1–K9). Neither store gains SQL rights over the partner's base tables; both
only ever write their own tier-D index rows.

## 3. Indexed corpus (what is searched)

Plan line 1329 is the authority: „SQLite FTS5 indexiert Projektnamen,
Source-Items, Transkripte, Aufgaben und Snapshots." The assigned leaf's shorter
list (segments, source_items, summaries, task titles) is subsumed — the plan
additionally requires project names and snapshots, so the corpus is:

| # | `entity_kind` | Base table (owner) | Indexed field(s) | Written by units of work |
| --- | --- | --- | --- | --- |
| 1 | `project` | `projects` (KnowledgeStore) | `name` | K1, K2/K2', K3 |
| 2 | `source_item` | `source_items` (KnowledgeStore) | `text` | K4, K5 (+ hard deletes per ASC §9) |
| 3 | `transcript_segment` | `meeting_transcript_segments` (MeetingStore) | `text` | U8, U3, hard deletes per ASC §9 |
| 4 | `meeting_summary` | `meeting_summaries` (MeetingStore) | `summary_markdown` | U9, U3, hard deletes per ASC §9 |
| 5 | `task` | `project_tasks` (KnowledgeStore) | `title`; `description` (indexed only when non-empty) | K7, task-level tombstones |
| 6 | `project_snapshot` | `project_snapshots` (KnowledgeStore) | `summary_markdown` (current snapshots only, see below) | K9 |

Corpus rulings:

- **C1 — snapshots.** Only the `current` snapshot of each project is indexed
  (plan line 1311: older snapshots are kept for traceability, not discovery);
  `publish_snapshot` (K9) replaces the previous snapshot's index rows in the
  same transaction that demotes it.
- **C2 — summaries include structure.** `summary_markdown` is indexed as one
  field; the structured JSON columns (`decisions`, `action_items`,
  `open_questions`) are not separately indexed in v1 (NG-8 rationale: they are
  derivations of the markdown content; adding them later is a version bump).
- **C3 — transport firewall.** `source_item.text` is indexed exactly as
  normalized by IPC `normalize` — no transport-format remnants (IPC §7
  INV-4). The indexer never parses original payloads.
- **C4 — one row per (entity, field).** Each indexed field is its own index
  row so that snippets never span concatenated fields and per-field deletion
  is trivial.
- **C5 — meeting titles** are not indexed (NG-9).

## 4. Logical index shape

One FTS5 virtual table for all corpora (unified querying, unified rebuild),
contentful (it stores its own copy of the indexed text):

```sql
-- DDL authority: task-0020/db-schema.md; logical shape normative here.
CREATE VIRTUAL TABLE search_index USING fts5(
    text,                       -- indexed content copy (field value verbatim)
    entity_kind   UNINDEXED,    -- §3 corpus discriminator
    entity_id     UNINDEXED,    -- stable UUID of the base entity (RBC §5.1)
    field         UNINDEXED,    -- e.g. 'name', 'text', 'title', 'description',
                                  -- 'summary_markdown'
    revision      UNINDEXED,    -- base-row envelope revision at index time (§8.2)
    tokenize = 'unicode61 remove_diacritics 1'
);

CREATE TABLE search_index_meta (          -- plain table, task-0020-owned DDL
    key   TEXT PRIMARY KEY,               -- 'tokenizer_spec', 'index_version'
    value TEXT NOT NULL
);
-- SEARCH_INDEX_VERSION = 1; bumped when corpus (§3), fields, or tokenizer
-- spec change. Mismatch with search_index_meta at startup => rebuild (§8.3).
```

Design decisions, with the alternatives rejected:

- **Contentful (self-storing) chosen** over *external-content*
  (`content=`base tables) and *contentless*. External-content saves disk but
  requires reproducing the exact original values on every delete/update —
  the classic drift source, and hostile to the soft-delete-first model where
  "removal" is driven by envelopes rather than row churn. Contentless tables
  complicate deletion (needs SQLite ≥ 3.43 `contentless_delete`) and support
  no auxiliary retrieval. Storage duplication is acceptable on a local
  single-user machine; text of even hundreds of hours of meetings is a few
  hundred MB worst case, and the AssetStore, not SQLite, holds the large
  binaries (plan lines 1274–1275).
- **Unindexed metadata columns** (`entity_kind`, `entity_id`, `field`,
  `revision`) ride on the same table so that scoped queries (§6.3) and
  cascade deletions (§7) are single-table statements and `DELETE FROM
  search_index WHERE entity_id = ?` is exact — verified in the §0.4 probe.
  They are `UNINDEXED` so they never pollute matching or ranking.
- **No rowids cross the seam.** FTS5 rowids are private to the index
  (identity is `(entity_kind, entity_id, field)`); RBC §5.1 forbids
  autoincrement-derived identity, and gate G2 forbids raw internals in
  results.
- **Tokenizer string is pinned** in `search_index_meta.tokenizer_spec`;
  a mismatch against the compiled-in expectation at startup forces a rebuild
  (§8.3), protecting against SQLite default changes across upgrades.

## 5. German tokenization, diacritics, and query translation

### 5.1 Index-time decisions

- **Tokenizer:** `unicode61 remove_diacritics 1`, written explicitly (it is
  the FTS5 default; pinning guards against silent default changes and makes
  `tokenizer_spec` self-describing). Case folding is inherent to `unicode61`.
- **Effect (probe-verified, §0.4):** umlaut diacritics fold (`München` is
  found by `munchen`); **`ß` is its own character and is *not* equivalent to
  `ss`** (the probe shows `grusse` does not match `Grüße`, `strase` does not
  match `Straße`). Accepted at index time; compensated at
  query time (Q5) rather than by storing a folded text duplicate, because a
  duplicated column would break snippet fidelity against the original text
  and double storage.
- **No stemming, no compound splitting** (NG-5). Trailing-compound recall is
  handled by automatic prefix matching (Q4).

### 5.2 Stated fuzzy bound (gate G4 wording)

The only non-exact matching mechanisms in v1 are:

1. case folding and umlaut-diacritic folding (`ä/ö/ü` → `a/o/u`) — inherent
   to the tokenizer;
2. right-truncation: every user term is a prefix term (Q4);
3. the deterministic `ß`↔`ss` term expansion (Q5).

Everything else (edit distance, phonetics, synonyms, spell correction,
stemming) is excluded per NG-2.

### 5.3 Query translation rules (normative, exhaustive)

The store translates the raw user string into the FTS5 `MATCH` expression.
Callers pass plain text; raw FTS5 syntax from callers is impossible by
construction (NG-6):

- **Q1 — empty handling.** Trim. An empty or whitespace-only query yields
  `Ok(vec![])` — not an error.
- **Q2 — term extraction.** Cap the query at **256 characters** (truncate,
  do not reject), split on Unicode whitespace, keep at most **8 terms**
  (excess dropped), and truncate each term to **64 scalar values**.
- **Q3 — sanitization.** Remove all `"` (U+0022) characters from each term.
  Terms are then wrapped as quoted phrases, so every remaining character
  (including `-`, `(`, `*`, `NEAR`, `OR`) is literal text. This is the
  grammar-injection guard; test target T9.
- **Q4 — prefix.** Each term becomes `"term" *` (quoted phrase + prefix), so
  `projekt` finds `Projektmaßnahme` (probe-verified).
- **Q5 — `ß` expansion.** For each term: if it contains `ss`, add an OR
  alternative with the first `ss` replaced by `ß`; if it contains `ß`, add an
  OR alternative with `ß` replaced by `ss`. At most one alternative per term;
  alternatives are OR-grouped inside the term's parentheses. Example:
  `strasse` → `("strasse" * OR "straße" *)`.
- **Q6 — combination.** Terms are joined by single spaces (= FTS5 implicit
  AND): `münchen strasse` → `("münchen" *) ("strasse" * OR "straße" *)`.

### 5.4 Query execution

- Matching restricted by scope via the `UNINDEXED` columns (§6.3); ranking is
  `ORDER BY rank` (BM25, built-in weights); execution is a single statement
  over `search_index` plus, where scope demands it, joins against base tables
  on stable UUIDs — executed inside `KnowledgeStore`, read-only.
- Reads of tier-D data are ordinary read operations (RBC §4.2 read-only
  list); they open no write transaction and append no log rows.

## 6. Query interface — repository seam only (gate G2)

### 6.1 Surface

The only v1 search entry point is the already-sketched
`KnowledgeRepository::search` (RBC §2/§2.1). Refined signatures (sketch,
compilable-shape; final homes per RBC §2):

```rust
// src-tauri/src/managers/knowledge_store.rs (sketch — not yet implemented)

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchEntityKind {
    Project, SourceItem, TranscriptSegment, MeetingSummary, Task, ProjectSnapshot,
}

#[derive(Debug, Clone)]
pub enum SearchScope {
    All,
    /// Hits restricted to entities reachable from these projects via
    /// `project_sources` and the meeting↔source link (RBC §3 N1/N2).
    Projects(Vec<Uuid>),
    /// Hits restricted to these corpora (§3).
    Kinds(Vec<SearchEntityKind>),
}

#[derive(Debug, Clone)]
pub struct SearchHit {
    pub kind: SearchEntityKind,
    pub entity_id: Uuid,          // stable reference, RBC §5.1
    pub position: HitPosition,    // Belegposition, plan line 1330
    pub field: &'static str,      // which indexed field matched
    /// Highlighted fragment: snippet() output with exactly balanced
    /// HIGHLIGHT_START (U+E000) / HIGHLIGHT_END (U+E001) markers around each
    /// match, '…' where content was elided. Plain text otherwise.
    pub fragment: String,
}

/// Canonical jump target for transcript hits (EPD §3.1 — same type, three
/// consumers; produced here, resolved by the UI/player boundary).
pub enum HitPosition {
    MeetingPosition(MeetingPositionRef),                 // segment/summary hits
    SourceItem  { source_id: Uuid, item_id: Uuid },      // chat/note/file items
    Project     { project_id: Uuid },                    // project name hits
    Task        { project_id: Uuid, task_id: Uuid },     // task title/description
    Snapshot    { project_id: Uuid, snapshot_id: Uuid },
}

impl KnowledgeStore /* and trait KnowledgeRepository */ {
    /// Returns at most `min(limit.clamp(1,200), 200)` hits in BM25-relevance
    /// order. Never errors on odd input (Q1/Q2/Q3); errors only on StoreError
    /// conditions.
    pub fn search(&self, q: &str, scope: SearchScope, limit: u32)
        -> Result<Vec<SearchHit>, StoreError>;
}
```

### 6.2 Result-model guarantees (G2)

- **Stable references only.** Every hit carries `entity_kind` + UUID +
  `HitPosition`. No SQLite rowids, no FTS internals (`offsets()` bytes, match
  indices), no `Connection`/statement handles, no filesystem paths — mirroring
  RBC §2 ("never row handles, never `Connection`"). Consumers resolve details
  through their ordinary typed read paths.
- **Fragments, not documents.** `fragment` is produced by
  `snippet(search_index, 0, U+E000, U+E001, '…', 16)` — at most ~16 tokens
  around the first match, original text preserved verbatim (probe-verified).
  Markers are constants exported from the store module; the frontend renders
  them (e.g., `<mark>`) and strips them before copying text.
- **Score opacity.** BM25 ranks hits internally (`ORDER BY rank`); the numeric
  score is deliberately **not** part of `SearchHit` — the UI displays order,
  never numbers (NG-1).
- **Jump-target convergence.** Transcript-segment hits carry
  `HitPosition::MeetingPosition(MeetingPositionRef)` per EPD §3.2, so search,
  task evidence, and the audio player share the single canonical position
  type; M20 ("Suche springt zur richtigen … Meeting-Zeitposition", plan line
  1902) is satisfiable without format conversion.
- **Command-layer shape.** Any Tauri command exposing search receives/returns
  only these typed values (RBC §7.1 R3); tauri-specta bindings therefore
  cannot leak row internals (RBC §7.3 command-surface test applies verbatim).
  Plan line 1777's future server/MCP verb `search_project_sources` maps onto
  this same method with `SearchScope::Projects`.

### 6.3 Scope semantics

- `All` — no restriction.
- `Projects(ids)` — hit kept iff its entity is reachable from one of `ids`:
  `project` (own id), `task`/`project_snapshot` (their `project_id`),
  `source_item` (via its `source_id` appearing in `project_sources`),
  `transcript_segment`/`meeting_summary` (via their `meeting_id`'s
  one-to-one `sources` row, RBC §3 N1, appearing in `project_sources`).
  Implemented as joins on UUIDs inside the store; no denormalized project id
  is added to the index (denormalization would break on relinking, plan
  line 796: Neu-Zuordnen ändert nur die Verknüpfung).
- `Kinds(kinds)` — filter on `entity_kind`.

## 7. Deletion cascade (gate G3)

### 7.1 Cascade rules

- **D1 — tombstone removal.** When a unit of work commits an envelope with
  `deleted_at = Some` for an indexed entity, the stage removes **all** index
  rows of that `entity_id` in the same transaction (U3, K3, K5 item-level
  tombstones, task tombstones via K7).
- **D2 — hard-delete removal.** When ASC §9 levels 2–3 physically delete
  indexed base rows (`meeting_transcript_segments`, `meeting_summaries`,
  `source_items`, whole `sources`), the performing unit of work removes the
  corresponding index rows in the same transaction. Level 1 („nur Rohaufnahme
  löschen") removes **nothing**: chunks are not indexed and segments remain
  searchable (ASC §9 row „FTS5 index": unchanged).
- **D3 — ancestor cascade (within one store's domain).** Removal propagates
  to dependents of the deleted entity, because index visibility follows
  *reachability*:

  ```text
  live(entity) = envelope alive (deleted_at = None)
                 ∧ every same-store ancestor alive
  same-store ancestors:
    source_item → source;                                   (KnowledgeStore, K5)
    task, current project_snapshot → project;               (KnowledgeStore, K3)
    transcript_segment, meeting_summary → meeting_session;  (MeetingStore, U3)
  ```

  Concretely: `soft_delete_source` (K5) removes its `source_items`' rows;
  `soft_delete_project` (K3) removes that project's task and
  current-snapshot rows; `soft_delete_session` (U3) removes that meeting's
  segment and summary rows. Restoration paths do not exist in v1 (tombstones
  are terminal); a restored entity re-enters the index only via rebuild
  (§8), which indexes exactly the `live` set.
- **D4 — relink is not deletion.** `unlink_project_source` /
  reassignment (K6, plan line 796) changes linkage only; index rows are
  untouched, and scope filtering (§6.3) reflects the new linkage immediately
  because it joins at query time.
- **D5 — no orphan rows survive a commit** (mechanical form of G3, evaluated
  per store over the corpora that store owns):

  > After every committed unit of work: an index row exists **iff** its
  > `(entity_kind, entity_id, field)` denotes an indexed field whose base row
  > is live per D3, and its `text` equals the base field value, and its
  > `revision` equals the base envelope revision.

  Cross-store residue from interrupted D6 flows is *not* a D5 violation at
  any single commit point; it is caught by the startup scan (§8.4).
- **D6 — cross-store deletion flows.** Deleting a whole meeting spans both
  stores (ASC §9 level ≥2 plus U3/K5). Because each unit of work owns exactly
  one transaction (RBC W1), the orchestration flow executes U3 and K5 as two
  commits; each removes the index rows of the corpora **its** store owns.
  A crash between the two commits cannot produce an orphan *within* either
  store's invariant, but can leave e.g. a meeting's `source_items` indexed
  while their `source` is already tombstoned. Therefore: every startup runs
  the cheap §8.2 drift/orphan scan before the first `search`, and detected
  residue is remedied by rebuild (§8.4). No incremental repair heuristics.

### 7.2 Proof obligations (how G3 is "provable")

Each deletion path maps to a named unit of work + a named behavioral test
(in-memory SQLite, RBC §7.3 pattern):

| Deletion event | Index effect | Test |
| --- | --- | --- |
| U3 `soft_delete_session` | segments' + summary's rows removed (D1/D3) | T3 |
| K3 `soft_delete_project` | project's, tasks', snapshot's rows removed (D1/D3) | T5a |
| K5 `soft_delete_source` | its `source_items`' rows removed (D1/D3); meeting-side rows are U3's business (D6) | T5b |
| ASC §9 level 2 hard delete | segment+summary+item rows removed (D2) | T4a |
| ASC §9 level 3 hard delete | all meeting-text rows removed (D2) | T4b |
| K7 task tombstone | task rows removed (D1) | T5c |
| K9 publish_snapshot | previous current-snapshot rows replaced (C1) | T6b |
| K6 unlink/relink | **no** index change; scope result changes (D4) | T10b |

Additionally, invariant D5 is checked as a property test (T7 runs an
arbitrary interleaving of mutations and compares incremental state against a
fresh rebuild, §8.1 — equality is exactly D5).

## 8. Rebuild-from-source guarantee

### 8.1 Rebuild function

`pub(crate) fn rebuild_search_index(tx: &rusqlite::Transaction<'_>) ->
Result<u64, StoreError>` (home: `knowledge_store.rs`, per ruling N1):

1. `DELETE FROM search_index` (whole table);
2. re-insert one row per live indexed field, from a single deterministic
   `SELECT` per corpus joined down from base tables, ordered by
   `(entity_kind, entity_id)` — the select expresses `live(·)` (§7.1 D3/D6)
   purely in SQL;
3. update `search_index_meta` (`index_version`, `tokenizer_spec`);
4. return the number of rows written.

The rebuild is idempotent: running it twice yields byte-identical index
content. It runs inside one transaction, so it either fully replaces the
index or leaves it untouched (W1 analog for tier-D bulk writes). It is
invoked from `lib.rs` setup (crate-internal, R2-style confinement), not from
services.

### 8.2 Incremental/drift correspondence

Because the stage stamps each row with the base envelope `revision` (§4),
drift is detectable: a maintenance read comparing
`count(*)/sum(revision)` pairs per `entity_kind` between `search_index` and
the live base projections identifies mismatch. Drift handling is always the
same remedy: rebuild. No incremental repair heuristics (simplicity budget).

### 8.3 When rebuild runs

- at startup, iff `search_index_meta.index_version != SEARCH_INDEX_VERSION`
  or `tokenizer_spec` mismatches (covers upgrades and corpus-definition
  changes, NG-8 escape hatch);
- when the §8.4 startup scan detects cross-store flow residue;
- never automatically on a timer, and never during normal operation —
  correctness comes from §2's same-transaction ride-along, not from periodic
  reconciliation.

### 8.4 Startup orphan/drift scan

Before serving the first `search` after process start, the store runs one
read-only check per corpus comparing index presence/revisions against the
live base projections (the §8.2 comparison). Outcome:

- clean → normal operation, no write occurs;
- residue from an interrupted D6 cross-store flow, or drift from a defect →
  one rebuild transaction (§8.1), then re-check.

The scan is O(index size) counting aggregates only — no per-row diffing on
the happy path.

## 9. Verification appendix (test targets, all in-memory SQLite per RBC §7.3)

| ID | Asserts |
| --- | --- |
| T1 | Tokenizer folding: `München` found by `munchen`; `Projektmaßnahme` found by `projekt` (mirrors §0.4 probe) |
| T2 | Sanitized query translation: terms containing `"`, `(`, `*`, `NEAR`, `OR` are matched literally; no `StoreError` raised (Q1–Q3, Q6) |
| T3 | U3 tombstone removes segment+summary hits (D1/D3) |
| T4a/b | ASC level-2/level-3 hard deletes remove exactly their rows (D2) |
| T5a/b/c | K3/K5/K7 cascade removals within their store's domain (D3) |
| T5d | Interrupted D6 flow (U3 committed, K5 not): startup orphan scan detects indexed `source_items` under a tombstoned source; rebuild clears them (D6, §8.4) |
| T6a | Forced failure mid-K4 leaves neither `source_items` nor index rows (atomicity, W5) |
| T6b | K9 swaps snapshot index rows atomically with demotion (C1) |
| T7 | Property: after arbitrary mutation sequences, incremental index == fresh rebuild (D5, §8.1) |
| T8 | Tampered index row (revision/text changed out-of-band) is reported by the §8.2 drift check |
| T9 | Q3 grammar-injection guard: `"` stripped; crafted terms never alter expression structure |
| T10a | Scope filters: `Projects`, `Kinds` return only reachable/correct-kind hits |
| T10b | Relink (K6) changes scope results without touching index rows (D4) |
| T11 | Fragments carry balanced U+E000/U+E001 marker pairs; elision uses `…` |
| T12 | Q5 `ß` expansion fires in both directions (`strasse`→`straße`, `straße`→`strasse`) |
| Static | RBC §7.3 allow-list gate extended with `fts5|search_index` awareness: only allow-listed files may name the virtual table |

## 10. Gate verification checklist

| Gate | Where satisfied |
| --- | --- |
| G1 population mechanism decided with consistency argument vs. RBC §5 | §2.1 decision (explicit envelope-driven indexer stage, triggers rejected); §2.2 six-point argument; §2.3 disclosed R1 consequence |
| G2 stable references + highlighted fragments, never raw internals | §6.1 types; §6.2 guarantees (no rowids/internals/scores; snippet markers; `MeetingPositionRef` convergence) |
| G3 deletion provably removes index entries (cascade rule stated) | §7.1 D1–D6 (incl. `live()` reachability and cross-store flows); §7.2 event→effect→test table |
| G4 non-goals exclude ranking ML, bounded fuzziness, cross-device; doc index updated | §0.2 NG-1/NG-2/NG-3 (plus NG-4–NG-11); `docs/meeting-mode-documentation-index.md` §3 row added in the same change |
