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
6. **`docs/meeting-mode-ux-copy-catalog.md`** — Textvorschläge für alle
   Randzustände; konsultieren, sobald UI-Texte oder i18n-Keys für den
   Meeting-Modus angelegt werden.

Kurzregel: **Plan → Baseline → Recorder → Daten → Provider → UX-Texte.**
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
