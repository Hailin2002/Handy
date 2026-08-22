# Meeting-Modus — Dokumentationsindex und Statusübersicht

Status: **Dokumentationsindex (dieses Dokument beschreibt nur andere Dokumente)**
Autor-Arbeitsstand: **HEAD-Commit `c5db14e8eadfe9e16d3ca2ccd78cbb452c024789`**
(zweistufig verifiziert am 22.08.2026 via `git rev-parse HEAD` in einem sauberen
Arbeitbaum; `git status --porcelain` war vor der Erstellung dieses Dokuments leer).
Plan: [docs/meeting-mode-plan.md](meeting-mode-plan.md)
Aufgenommen am: 22.08.2026

---

## 1. Verbindliche Regel: Implementiertes Verhalten nur mit Codebeleg

> **Nur Dokumente, die ihr Aussagen mit konkreten Codebelegen versehen
> (`datei.rs`/`datei.tsx` mit Fundstelle oder Commit-Hash), dürfen beschreiben,
> was implementiert ist.** Alle übrigen meeting-mode-Dokumente beschreiben
> Planung, Vorschläge oder Verträge — niemals lauffähiges Verhalten. Ein
> Statuswert „implemented behavior" ohne Codebeleg in der Tabelle unten ist
> ein Dokumentationsfehler und zu korrigieren. Aktuell (Stand HEAD
> `c5db14e8`) existiert **kein** meeting-mode-Dokument mit dem Status
> „implemented behavior": Der Meeting-Modus ist im Repository noch nicht
> implementiert (verifiziert im UX-Copy-Katalog, Abschnitt 1: kein
> Produktionscode unter `src-tauri/src`, keine Komponenten unter
> `src/components/meetings/`, keine Meeting-i18n-Keys).

## 2. Status-Enum (verbindliche Werte)

| Wert | Bedeutung |
| --- | --- |
| `plan` | Planungsdokument; legt Ziele, Umfang, Phasen und Abnahme fest. Keine Verhaltensaussage über bestehenden Code außer mit explizitem Codebeleg. |
| `proposal-only` | unverbindlicher Vorschlag (z. B. Textentwürfe); nichts davon ist implementiert oder freigegeben. |
| `spec contract` | normativer Vertrag für zukünftige Implementierung (Interface, Datenformat, Zustände); ändert keinen Quellcode. Gültig erst mit Implementierung, dann gegen den Code zu prüfen. |
| `implemented behavior` | beschreibt tatsächlich vorhandenes, im Code belegbares Verhalten. Nur zulässig mit Code-/Commit-Beleg (siehe Regel in Abschnitt 1). |

## 3. Dokumenttabelle

Verifiziert per `glob docs/**/*.md` zum Autor-Zeitpunkt: genau diese sechs
Meeting-Mode-Markdown-Dateien existierten unter `docs/`; keine fehlenden und
keine erfundenen Pfade. Jeder nachfolgende Task, der ein neues
meeting-mode-Dokument anlegt, **muss** diese Tabelle um eine Zeile ergänzen.

| Dokumentpfad | Abgedeckte Plan-Abschnitte (meeting-mode-plan.md) | Status | Authority Statement |
| --- | --- | --- | --- |
| `docs/meeting-mode-plan.md` | gesamtes Dokument (§1–§17): Ziel, Umfang, Architektur, UX, Audio, STT, Datenmodell, Phasen 0–11, Testmatrix, Definition of Done | `plan` | Finale Planung („noch keine Implementierung", Kopfzeile des Dokuments); einzige normative Quelle für alle meeting-mode-Verträge; beschreibt Produktverhalten ausschließlich als Ziel, nicht als Ist-Zustand. |
| `docs/meeting-mode-phase0-baseline.md` | §13 Phase 0, §14 Testmatrix (M1–M20 als pending angelegt); Abschnitt 3 dokumentiert den bestehenden Diktierpfad aus §4 „Verifizierter Ausgangszustand" | `implemented behavior` (nur für den Diktier-/Build-Bestand) | Zeichnet die tatsächlich ausgeführte Phase-0-Basis auf (Checkpoint-Commits `2355da6c`, `b677107`, `a7226b6`, `8fd9dee`, `beb286e`; Messprotokolle unter `src-tauri/target/baseline-tools/logs/`) und zitiert Code (`speech_pipeline.rs`, `settings/types.rs`) — gilt als implemented-behavior-Autorität **ausschließlich für den Diktierpfad und die Build-Basis**, nicht für irgendeinen Meeting-Feature-Umfang; alle M1–M20-Ergebnisse sind dort ausdrücklich `pending`. |
| `docs/meeting-mode-phase2-contracts.md` | §7 („Persistenzformat", „Gemeinsame Zeitachse"), §8 („Gemeinsames Segmentmodell"), §9 (`meeting_audio_chunks`), §13 Phase 2 & 5, §14 (drei benannte Testmatrix-Zeilen) | `spec contract` | Normativer Datenvertrag (On-disk-Layout, Manifest-Schema, Finalisierung, Recovery, Konsumregeln) für Phase 2/5; Eigenangabe im Kopf: „contract draft — no implementation yet … changes no source code". |
| `docs/meeting-mode-ux-copy-catalog.md` | §6.2 (Regel 8), §6.8, §6.9, §6.11, §6.12, Session-Zustände/Fehlerverhalten, §11 (Löschen), §12 (Speicher/Datenschutz) | `proposal-only` | Vorschau-Katalog englischer i18n-Textvorschläge; Eigenangabe: „Nichts davon ist implementiert oder freigegeben"; enthält selbst eine ehrliche Verifizierungstabelle gegen den aktuellen Code (Abschnitt 1). |
| `docs/meeting-mode-architecture/recorder-contract.md` | §5 (`MeetingRecorder`), §6.x (Session-Zustände, Startvalidierung, Pause, Stoppen, Fehlerverhalten), §7 (Loopback, Gemeinsame Zeitachse, Persistenzformat), §14 (drei benannte Testmatrix-Zeilen) | `spec contract` | Interface-/Zustandsmaschinen-Vertrag für `MeetingRecorder` inkl. Reuse-Pflichten (verweist code-cited auf bestehendes `audio_toolkit::audio::device` und `FrameResampler` in `src-tauri/src/audio_toolkit/audio/resampler.rs`); Eigenangabe: „contract draft — no implementation yet … changes no source code". |
| `docs/meeting-mode-architecture/stt-provider-contract.md` | §8 (Transkriptionspipeline), §9 (`meeting_stt_jobs`) | `spec contract` | Normatives Einzel-Trait-Konzept `MeetingSttProvider` (Local/Azure Fast Transcription/Custom Multipart) inkl. Jobzustandsmaschine und Retry-Idempotenz; Eigenangabe: „Architekturvertrag, Phase 5 nicht implementiert"; der bestehende Short-Audio-Pfad bleibt von diesem Vertrag unberührt. |
| `docs/meeting-mode-architecture/coexistence-contract.md` | §6.10 (Gleichzeitige Nutzung beider Werkzeuge), §6 (Startvalidierung: `selected_output_device`), §6.9 (Shortcut-Konzept), §8 (Ressourcenlogik zwischen Meeting und Diktat, Lokale Verarbeitung), §13 Phase 4/5, §14 (Zeile „Diktat wird während lokaler Meetingverarbeitung ausgelöst") | `spec contract` | Normativer Koexistenzvertrag Diktat ↔ Meeting: Engine-Lease-Erweiterung mit Pause-an-Segmentgrenze/Auto-Resume/Timeouts und Strukturbeweis „Capture wartet nie auf Inferenz", Mikrofon-Tap mit Ein-Gerät-offen-Invariante (Fälle Meeting-aktiv/-pausiert/ohne Meeting), Start-Mutual-Exclusion mit Nutzerentscheidung, Settings-Trennung inkl. expliziter Erhaltung von `selected_output_device` für Feedback-Sounds und Session-Snapshot-Isolation, Shortcut-Namespace (`meeting_*`) mit Verfügbarkeits-Precheck und Blockierung identischer Bindings; Touch-Point-Inventar mit Codebelegen markiert jede Stelle als Zukunftsphase, null Codeänderungen durch den Vertrag selbst. |
| `docs/meeting-mode-architecture/shutdown-dataflow.md` | §6.2 Regel 10 (Zeilen 396–398), §6.12 (Zeile 654), „Stoppen" (Zeilen 936–941), Session-Zustände/Fehlerverhalten (Zeilen 877–896, 959–961), §13 Phase 4 (Zeile 1571), §17 DoD Punkt 2 | `spec contract` | Normative Shutdown-/Quit-Sequenz: zwei Teardown-Varianten T (Capture aktiv) und P (nur Verarbeitung) über acht Schritten D0–D7 mit Test-Timeouts pro Schritt, Zustandsresiduum je Timeout und Watchdog-Fallback (8 s interaktiv / 4 s End-Session); vier Exit-Eintrittspunkte (Tray-Quit, Fenster schließen, OS-Abmeldung/-Herunterfahren, Crash mitten im Teardown) jeweils mit Beleg der bestehenden Code-Nähte (`tray.rs` Quit-Handler, `lib.rs` CloseRequested/RunEvent::Exit, signal_handle SIGUSR1/2); Falltafel Q0–Q8 weist jedes Unterbrechungs-Residuum der bestehenden Startup-Recovery-Sequenz (startup-recovery-dataflow §3 S0–S7) zu — keine neue Recovery-Regel; dokumentiert die `stopping`-Kandidatenmengen-Abweichung zu startup-recovery-dataflow S0 zugunsten von session-state-contract R1; explizite Non-Goals/Komplexitätsbudget; Eigenangabe: „spec contract — no implementation yet … changes no source code". |
| `docs/meeting-mode-architecture/module-placement-map.md` | §5 („Zielarchitektur": alle Backend-Komponenten, Plan-Zeilen 219–308; Frontend-Komponentenliste, Plan-Zeilen 310–333) | `spec contract` | Normative Modulplatzierung: genau ein kanonischer Pfad je Plan-§5-Komponente (B1–B11, F1–F22) plus Re-export-/Sichtbarkeitsregeln (E1–E5, F1), adjudiziert den Pfadkonflikt `MeetingRecorder` (Plan §5 `audio_toolkit/audio/meeting_recorder.rs` vs. Ledger task-0011 `src-tauri/src/meeting_recorder`) zugunsten des Plan-Pfads gemäß Autoritätsordnung „Plan schlägt Verträge", und cross-checkt jede Platzierung gegen die Scope-Abschnitte von recorder-, asset-store- und repository-boundary-contract; Eigenangabe: „spec contract — no implementation yet … changes no source code". |
| `docs/meeting-mode-architecture/job-scheduler-contract.md` | §5 (`MeetingTranscriptionQueue`, Plan-Zeilen 245–252), §8 (Lokale Verarbeitung Zeilen 1059–1067, Ressourcenlogik Zeilen 1089–1099, Azure-Jobs Zeilen 1121–1128), §9 (`meeting_stt_jobs` Zeilen 1190–1199, `processing_jobs` Zeilen 1313–1318), Fehlerverhalten/Stoppen (Zeilen 936–961) | `spec contract` | Normativer Cross-Subsystem-Scheduler-Vertrag über `MeetingSttProvider` und `MeetingSummaryService`: genau drei Konkurrenzklassen (L1 = genau ein lokaler Engine-Lease gemäß Coexistence-Vertrag, C1 = begrenzte Cloud-STT-Slots, S1 = begrenzte LLM-Slots), Prioritäts-/Fairnessregeln (Timeline-Reihenfolge, Mic/System-Alteration, Per-Meeting-Rotation), Cancellation-Propagierung für queued/leased/running mit idempotenten Ergebnissen, Lease-Timeouts per Deadline-Sweep, Retry-Backoff-Ownership (Klassifikation an den Providern, Uhren ausschließlich im Scheduler) und Invariante P1 „Job-Zeilen sind der einzige Persistenzpunkt" inkl. Strukturbeweis CAP1 „Capture wartet nie auf Scheduling"; konsistent mit stt-provider-contract (Zustandsmaschine STT-§3) und summary-evidence-contract (Idempotency-Keys SEC-§2); explizite Non-Goals/Komplexitätsbudget; Eigenangabe: „spec contract — no implementation yet … changes no source code". |
| `docs/meeting-mode-architecture/summary-provider-contract.md` | §5 (`MeetingSummaryService`, Plan-Zeilen 253–259), §6.7 (Summary-Schalter, Plan-Zeile 522), Startvalidierung (optionaler Summary-Provider, Plan-Zeile 907), Session-Zustände/Fehlerverhalten (Plan-Zeilen 877–891, 943–960), §10 (Provider-Unabhängigkeit Zeilen 1340–1347), §11 (erneute Zusammenfassung, Plan-Zeile 1417), §12 (Offenlegungspflicht Zeilen 1454–1456, Aufbewahrungsschutz Zeilen 1451–1452), §13 Phase 7 (Zeilen 1629–1645) | `spec contract` | Normativer Provider-Auswahl-/Fallback-/Cloud-Kostenwächter-Vertrag für `MeetingSummaryService`: Cloud-Pfad unerreichbar ohne persistierten Consent-Flag `meeting_summary_cloud_consent` (Default `false`, typisiertes AppSettings-Schema, nie aus Diktat-Einstellungen abgeleitet), endliche Fallback-Kette maximaler Länge 2 mit Richtungs-Invariante INV-P1 (nur Cloud→lokal, nie zurück), Terminalzustand `summary_no_path_available` mit vorgeschlagenen Copy-Katalog-Keys und Aufbewahrungsschutz, Request-Größen-Budgetierung als Hand-off an summary-pipeline-dataflow (SPD-§3, task-0090), Disclosure vor/während/nach Cloud-Läufen bei explizit abgelehnter Kosten-Messung, Local-Mode-Garantie restated konsistent zu SEC-§5; Non-Goals: keine automatischen Modell-Downloads, kein Cost Metering über Disclosure hinaus; Eigenangabe: „spec contract — no implementation yet … changes no source code". |
| `docs/meeting-mode-architecture/frontend-state-contract.md` | §5 (Schichtarchitektur Zeilen 205–217, Frontend-Komponenten Zeilen 310–333), §6 (UX-Prinzipien Zeilen 394–397, „kein zweiter Recording-State" Zeilen 924–927), §9 (Change-Envelope Zeilen 1149–1159), §13 Phase-3-/Phase-4-Abnahme (Zeilen 1556, 1577) | `spec contract` | Normativer Frontend-State-Vertrag: drei neue Domain-Zustands-Stores (`meetingStore`, `projectStore`, `dictationStore`) mit einer totalen, injektiven Ownership-Matrix als 1:1-Spiegel der Entity-Ownership-Matrix des repository-boundary-contract (Zeilen 16–17 bewusst ohne Frontend-Spiegel); exklusiver `listen()`-Besitz pro Event-Topic über Registrar-Module mit symmetrischem Unlisten-Lebenszyklus, Revisions-Guard (SSC-O3) und Subscribe-then-Snapshot-Handshake (SSC-O5/O6) inkl. Defect-Class-/Test-Tabelle; Change-Envelope-Konsum (Tombstones, Listen-Invalidierung, Tier-B-Koaleszenz); Command-Aufrufe nur in Store-Actions (Erweiterung von RBC-R5, Legacy-Carve-Out); Optimistic-Update-Whitelist O-1 bis O-4 mit Rollback-Mechanik, alles Übrige await-authoritativ; Non-Goals: keine zweite State-Library, kein Client-Cache-Framework, kein Offline-Mutation-Queue, kein persistierter Frontend-State; Eigenangabe: „spec contract — no implementation yet … changes no source code". |
| `docs/meeting-mode-architecture/search-index-contract.md` | §9 „Suche" (Zeilen 1327–1332), §9 Change-Envelope-Vorspann (Zeilen 1149–1162), §5 (`KnowledgeStore`, Zeilen 269–275), §6.13 (Zeile 673), §6.16 (Zeile 739), §13 Phase-3-Abnahme (Zeilen 1548, 1658), Testmatrix M20 (Zeile 1902) | `spec contract` | Normativer FTS5-Suchindex-Vertrag: Population entschieden als explizite, Change-Envelope-getriebene Indexer-Stufe innerhalb derselben Transaktion je Unit of Work (Konkretisierung von RBC-W5; SQL-Triggers mit sechspunktiger Konsistenzargumentation gegen RBC §5 abgelehnt; offengelegte einzeilige R1-Allowlist-Amendment-Forderung für `managers/search_index.rs` plus Ownership-Auslegung zu Matrixzeile 17); logische FTS5-Indextabelle über sechs Korpora (Projektnamen, Source-Items, Transkriptsegmente, Meeting-Summaries, Aufgaben-Titel/-Beschreibungen, aktuelle Snapshots) mit UNINDEXED-Metadatenspalten und gepinntem Tokenizer `unicode61 remove_diacritics 1`; deutsche Tokenisierung mit Umlaut-Faltung, dokumentierter ß-Limitation und deterministischer ß↔ss-Anfrage-Expansion, probe-verifiziert offline gegen die gepinnte bundled-SQLite-Version (3.50.2 via rusqlite 0.37/libsqlite3-sys 0.35.0, null neue Crates); Abfrage ausschließlich über `KnowledgeRepository::search` mit stabilen Entity-Referenzen, Snippet-Fragmenten (U+E000/U+E001-Marker, opake BM25-Ordnung ohne Score-Exposition) und kanonischem `MeetingPositionRef` gemäß Export-Dataflow; Lösch-Kaskade D1–D5 inklusive Hard-Delete-Leveln 2/3 aus dem Asset-Store-Vertrag und Reachability-Definition `live()`; Rebuild-from-Source-Garantie mit Revisions-Drift-Erkennung; Non-Goals/Komplexitätsbudget NG-1–NG-11 (kein Ranking-ML, Fuzzy-Suche nur Prefix + ß↔ss-Expansion, keine geräteübergreifende Suche, keine Vektorsuche); Eigenangabe: „spec contract — no implementation yet … changes no source code". |

| `docs/meeting-mode-architecture/retention-quota-contract.md` | §12 (Speicher- und Datenschutzlogik, Zeilen 1436–1476), §11 (Löschen, Zeilen 1405–1434), §9 (`change_log`, Zeilen 1320–1326), §2 (Nicht-Ziele) | `spec contract` | Normativer Retention-/Quota-/Change-Log-Pruning-Policy-Engine-Vertrag als reine Naht über Asset-Store-§8/§9: drei abgeleitete Ledgers (Audio-Bytes, Asset-Bytes, Change-Log-Zeilen; nie gespeicherte Zähler), vier Auswertetrigger T1–T4 (Startup nach AS-§7-Scan, Post-Finalize, Post-Import, Daily-Timer laut AS-§8.2), festes dreistufiges Sequenzmodell A→B→C mit reinem `evaluate(PolicyInput) -> Vec<PolicyAction>` (kein I/O, keine Uhr im Modul, injizierter Zeitstempel, totale Ordnung per UUID-Tiebreak), genau drei v1-Regeln ohne DSL (Age; Quota Hard = 100 Dual-Track-Stunden ≙ 23 040 000 000 B / Soft = 80 h auf projiziertem Post-Pass-Total, mit normativer Früh-Lösch-Klarstellung §4.4: Quotadruck trifft nur beantwortete Zeitfenster-Policies, nie `Unanswered`/`KeepPermanent`/schutz-flagged; Change-Log-Prune 180 Tage, max. 1000 Zeilen je Pass mit Marker-Zeile), zwei Exemption-Dimensionen (`Unanswered`/`KeepPermanent`, `projects.protect_meeting_audio`) plus adoptiertem `RetentionBlockReason::SessionEndedDiskFull` (schließt DG-§7 offenen Punkt 1), Doppel-Gate-Invariante G3 (Snapshot-Filter plus RetentionGuard unmittelbar vor dem ersten Unlink, exhaustive Unlink-Pfad-Enumeration U-P1 bis U-P5), Auto-Löschung ausschließlich Delete-Level 1 (Level 2/3 typsystemisch unerreichbar), N1–N6-Notice-Key-Tabelle mit drei Proposed-New-Keys und zwei begründet keylosen Pfaden; Sketch via verify-sketch.ps1 kompiliert (rustc 1.98.0) und durch sieben Rust-Tests verifiziert (Konstanten, Determinismus, Block-Schutz, Quota-Projektion, Soft-Floor, Stuck-Warning, Prune-Cap/Cutoff; 7 passed, 0 failed); Non-Goals NG-1–NG-8 (namentlich keine konfigurierbare Rule-DSL); Eigenangabe: „spec contract — no implementation yet … changes no source code“. |
| `docs/meeting-mode-architecture/project-snapshot-contract.md` | §9 (`project_snapshots`/`snapshot_evidence`, Zeilen 1303–1311; `processing_jobs` Zeilen 1313–1318; `change_log` Zeilen 1320–1325; Envelope-Regeln Zeilen 1149–1159; manuelle Statusnotiz Zeilen 1224–1225), §10 „Projektintelligenz“ (Zeilen 1380–1403), §5 (`ProjectIntelligenceService` Zeilen 293–299, `ProjectManager` Zeilen 261–265), §13 Phase 10 (Zeilen 1694–1712), §14 (Snapshot-Testmatrix-Zeile, Zeile 1879) | `spec contract` | Normativer Projekt-Snapshot-Lebenszyklusvertrag: idempotenter Neuschöpfungsschutz je `(project, source-revision-set)` durch Instantiierung der SEC-§2-Schlüsselalgebra mit `UNIQUE(project_id, idempotency_key)` plus Coalescing-Disziplin (höchstens ein nicht-terminaler Intelligence-Job je Projekt, P0-Cutoff erst zur Ausführungszeit, keine Timer-Re-Analysis), lineare Parent-Chain ohne Verzweigung via atomarer K9-Demotion, Write-once-Unveränderlichkeit veröffentlichter Snapshots inklusive gepinnter Belegrevisionen und Read-time-Aging der Pins (`live`/`stale_quote`/`locator_gone`/`source_tombstoned`), struktureller Vorrangsnachweis für Nutzeränderungen über disjunkte AI-Schreibflächen gegen die SEC-§4-Schutzspalten plus `manual_change_guard` (RB-§6-ProjektManager-Pflichten zitiert), Entity-Level-Diffmodell (`StatementKind`/`DiffOp`/`DiffEntry`/`SnapshotDiff`) mit deterministischer Lineage-Zuweisung über Fingerprints statt Text-Patches (Sketch via verify-sketch.ps1 kompiliert, rustc 1.98.0), Keep-all-Retention alter Versionen mit an Retention-Quota-Hooknamen gebundenem reserviertem Erweiterungspfad (`PolicyAction::PruneSupersededSnapshots`, Trigger T4, Stage-B-Position nach R3, Marker-Row-first, Gate-2-Analog im Pruning-Transaction), Prompt-/Modellversionierung bewusst nur als SEC-§6-Zeiger ohne Norm-Restatement; Non-Goals NG-1–NG-8 (namentlich keine Text-Patches, keine Snapshot-Verzweigung, keine automatische Löschung in v1); Eigenangabe: „spec contract — no implementation yet … changes no source code“. |

### Verwandte, aber keine .md-Dokumente (keine Tabellenzeilen)

Die Spike-Artefakte unter `docs/phase1-spike/` (CSV-Rohdaten der vier Läufe
sowie Messwerkzeuge wie `analyze-spike-csv.ps1`) sind **Evidenz, keine
Dokumentation**: Sie werden von `docs/meeting-mode-phase2-contracts.md`
(§4 Feld-Mapping) zitiert, besitzen aber keinen eigenen beschreibenden Text
und erscheinen deshalb nicht in der Dokumenttabelle. Das Werkzeug
`docs/meeting-mode-architecture/verify-sketch.ps1` ist ebenfalls Skript,
kein Dokument.

## 4. Lesereihenfolge für neue Contributor

1. **`docs/meeting-mode-plan.md`** — vollständig lesen. Erzeugt das
   Produktverständnis (§1–§3), den verifizierten Ausgangszustand (§4) und die
   Zielarchitektur (§5). Ohne diesen Kontext ist jedes Folgedokument
   missverständlich.
2. **`docs/meeting-mode-phase0-baseline.md`** — aktueller, messverifizierter
   Stand der Build-/Testbasis und des unangetasteten Diktierpfads; erklärt,
   worauf alle weiteren Phasen aufbauen.
3. **`docs/meeting-mode-architecture/recorder-contract.md`** — der erste
   Schnittstellenvertrag (Capture → Chunks → Events); Grundlage für jeden
   Beitrag an Phase 2.
4. **`docs/meeting-mode-phase2-contracts.md`** — der Datenvertrag auf dem
   Recorder-Vertrag (Chunk-Naming, Manifest, Finalisierung, Recovery,
   STT-Konsum); Pflichtlektüre vor jeder Änderung an Persistenz oder Queue.
5. **`docs/meeting-mode-architecture/stt-provider-contract.md`** — der
   Providervertrag (Queue ↔ Local/Azure/Custom); Pflichtlektüre vor Arbeit an
   Phase 5/6.
6. **`docs/meeting-mode-architecture/job-scheduler-contract.md`** — der
   Scheduler-/Cancellation-Vertrag über Provider- und Summary-Jobs
   (Konkurrenzklassen, Fairness, Leases, Backoff-Ownership, Persistenzpunkt);
   Pflichtlektüre vor jeder Änderung an `meeting_transcription.rs`,
   `processing_jobs` oder Cancellation-Flows.
7. **`docs/meeting-mode-architecture/summary-provider-contract.md`** — der
   Summary-Provider-Vertrag (Consent-Gate, Fallback-Kette,
   Kosten-/Disclosure-Wächter, Hard-Failure); Pflichtlektüre vor jeder Arbeit
   an `meeting_summary.rs` oder Meeting-Summary-Einstellungen.
8. **`docs/meeting-mode-architecture/frontend-state-contract.md`** — der
   Frontend-State-Vertrag (Store-Grenzen, Listener-Ownership,
   Envelope-Konsum, Optimistic-Regeln); Pflichtlektüre vor jeder Arbeit an
   `src/stores/` oder den Komponenten F1–F22.
9. **`docs/meeting-mode-architecture/search-index-contract.md`** — der
   Suchindex-Vertrag (FTS5-Population, Tokenisierung, Abfrage-/Lösch-/
   Rebuild-Semantik); Pflichtlektüre vor jeder Arbeit an
   `knowledge_store.rs`, `search_index.rs` oder Suchkommandos.
10. **`docs/meeting-mode-architecture/project-snapshot-contract.md`** — der
    Projekt-Snapshot-Vertrag (Snapshot-Identität/Parent-Chain, Pinning,
    Entity-Level-Diff, Retentions-Hooks); Pflichtlektüre vor jeder Arbeit an
    `project_intelligence.rs` oder `publish_snapshot`.
11. **`docs/meeting-mode-ux-copy-catalog.md`** — Textvorschläge für alle
   Randzustände; konsultieren, sobald UI-Texte oder i18n-Keys für den
   Meeting-Modus angelegt werden.

Kurzregel: **Plan → Baseline → Recorder → Daten → Provider → Scheduler →
Summary-Provider → Frontend-State → Search-Index → Snapshots → UX-Texte.**
Jedes Vertragdokument nennt im Kopf seine Plan-Anker; bei Konflikten gilt die
Reihenfolge: Plan schlägt Verträge, Verträge schlagen Kataloge, und nichts
schlägt den tatsächlichen Code (der seinerseits nur durch code-zitierte
Dokumente wie die Phase-0-Baseline abgedeckt wird).

## 5. Pflegeregeln

- Neues meeting-mode-Dokument → sofortige Ergänzung der Tabelle in Abschnitt 3
  (Pfad, Plan-Abdeckung, Statuswert aus Abschnitt 2, einzeiliges Authority
  Statement).
- Statuswechsel (z. B. `spec contract` → `implemented behavior`) erfordert einen
  Codebeleg im Authority Statement gemäß Regel in Abschnitt 1.
- Dieses Indexdokument selbst hat immer Status „Dokumentationsindex" und
  beschreibt nie eigenständig implementiertes Verhalten.
