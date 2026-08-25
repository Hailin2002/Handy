# Meeting-STT-Provider-Vertrag

Status: **Architekturvertrag, Phase 5 nicht implementiert**
Plan: [docs/meeting-mode-plan.md](../meeting-mode-plan.md), Abschnitt 8
(Transkriptionspipeline) und Abschnitt 9 (`meeting_stt_jobs`)
Geltungsbereich: das **einzige** gemeinsame Interface, hinter dem Local,
Azure (Fast Transcription) und Custom (Multipart) als Meeting-STT-Provider
stehen. Kein Aufrufer außerhalb der Meeting-Transkriptionskomponente kennt
providerspezifische Details.

---

## 1. Das Meeting-SttProvider-Interface

Es gibt genau ein Trait-Konzept `MeetingSttProvider`. Die drei Implementierungen
sind `LocalMeetingProvider`, `AzureMeetingClient` und `CustomMultipartProvider`.
`AzureMeetingClient` existiert **neben** dem bestehenden Short-Audio-Pfad
(`speech_pipeline.rs`); der Short-Audio-Pfad bleibt unverändert für Diktate.

### 1.1 Interface-Operationen

```text
MeetingSttProvider
  validate(config_snapshot) -> Result<ProviderReadiness, ProviderError>
      prüft Konfiguration VOR Jobstart (Endpoint, Modell, Sprache,
      Secret-Vorhandenheit als Boolescher Befund — niemals der Wert selbst).
  transcribe(job: SttSegmentJob) -> Result<ProviderResult, ProviderError>
      verarbeitet genau ein Segment/Paket und liefert relative Segmente
      zurück; blockiert höchstens für die vereinbarte Timeout-Dauer.
  capabilities() -> ProviderCapabilities
      max_audio_duration, max_payload_bytes, returns_word_timestamps,
      returns_confidence, supports_channel_count, streaming: false (v1).
```

Der Aufrufer (`MeetingTranscriptionQueue`) kennt nur dieses Interface. Er
kennt keine URLs, keine HTTP-Statuscodes, keine Multipart-Feldnamen und keine
Kanalreihenfolge. Providerbesonderheiten sind ausschließlich in den
Implementierungs-Unterkapiteln von Abschnitt 4 verortet.

### 1.2 Ergebnisvertrag

`ProviderResult` enthält ausschließlich **relativ zum übergebenen Audio**
gemessene Segmente:

```text
segments: Vec<ProviderSegment>
  start_offset_ms, end_offset_ms   // relativ zum Anfang des übergebenen Audios
  text
  word_timestamps: Option<Vec<...>>  // falls capabilities().returns_word_timestamps
  confidence: Option<f32>
detected_language: Option<String>
```

Der Provider liefert **keine** Meeting-Timeline-Zeiten. Die Korrektur auf die
Meeting-Timeline macht ausschließlich die Queue (Abschnitt 5).

---

## 2. Gemeinsames Segmentmodell (Eingabe)

Jeder Job trägt mindestens die Felder aus Plan Abschnitt 8
(„Gemeinsames Segmentmodell“):

| Feld                         | Bedeutung                                                                                |
| ---------------------------- | ---------------------------------------------------------------------------------------- |
| `meeting_id`                 | stabile UUID der Session                                                                 |
| `source`                     | `microphone` oder `system` (bei Azure-Paketen die logische Quelle des Pakets, siehe 4.2) |
| `timeline_start_ms`          | Segmentbeginn auf der gemeinsamen Meeting-Timeline                                       |
| `timeline_end_ms`            | Segmentende auf der gemeinsamen Meeting-Timeline                                         |
| `audio_chunk_ids`            | referenzierte `meeting_audio_chunks`-IDs (Beweiskette Audio → Text)                      |
| `language`                   | zu verwendende Sprache aus dem Session-Snapshot                                          |
| `provider`                   | `local` \| `azure` \| `custom`                                                           |
| `provider_settings_snapshot` | **nicht geheimer** Konfigurationssnapshot zum Jobstart (Abschnitt 7)                     |
| `attempt`                    | Zähler der Versuche, beginnt bei 0 und wird bei jedem `running`-Eintritt erhöht          |
| `state`                      | Zustand aus der Zustandsmaschine in Abschnitt 3                                          |

Zusätzlich persistiert die Queue (Tabelle `meeting_stt_jobs`, Plan Abschnitt 9):
`id`, `attempt_count`, `next_attempt_at`, `last_error` und den
**Idempotency-Key** (Abschnitt 3.3). `audio_chunk_ids` bleibt bis zum
bestätigten Ergebnis erhalten; Audiodaten werden erst nach erfolgreichem,
transaktional abgeschlossenem Job gemäß Aufbewahrungsregel freigegeben
(Plan Abschnitt 12: fehlgeschlagene Jobs verhindern Audiolöschung).

---

## 3. Persistente Job-Zustandsmaschine

Zustände: `queued`, `running`, `retry_wait`, `succeeded`, `failed`,
`cancelled` (identisch zu Plan Abschnitt 9 und zur generischen
`processing_jobs`-Semantik). Jeder Übergang wird **transaktional zusammen mit
seinen Nebenwirkungen** (Segment-Inserts, Fehlerzeile, Zähler) in SQLite
geschrieben.

### 3.1 Zustandsdiagramm

```text
                     +---------------------------------+
                     |  App-Neustart / Crash-Recovery  |
                     |  running (unterbrochen) -> queued
                     |  queued bleibt queued           |
                     +---------------------------------+
                                   |
                                   v
   Nutzer bricht ab            +---------+   Worker nimmt Job,
   (Meeting-Abbruch oder  ---> | queued  |--> next_attempt_at
   einzelne Wiederholung)      +---------+   erreicht, Engine/
       |                       |        ^    Lease frei
       | cancelled             |        |        |
       v                       |        |        v
  +-----------+   Nutzer       |   retry_wait <---+  transitorischer
  | cancelled |   wiederholt   |   (Backoff-      |  Fehler bei attempt
  +-----------+   / Re-Queue   |    Timer)        |  < max_attempts
       |                       |        ^         |
       | Nutzer wiederholt     |        |         v
       | (Re-Queue)            |        +---- retry_wait
       v                       |        |    attempt >= max_attempts
  (neuer Job, s. u.)           |        |
                               v        |
                        +-----------+   |
                        | running   |---+
                        +-----------+
                         |         |
        permanenter      |         |      Erfolg: Segmente +
        Fehler ODER      |         |      state-Flip in EINER
        max_attempts     |         |      Transaktion
        erreicht         |         |
                         v         v
                   +---------+  +-----------+
                   | failed  |  | succeeded |
                   +---------+  +-----------+
                        |             |
     Nutzer wiederholt  |  TERMINAL:  |  kein Übergang heraus.
     (Re-Queue) --------+  failed und succeeded sind Terminalzustände
     (neuer Job, s. u.)
```

### 3.2 Zulässige Übergänge — vollständige Liste

| Von          | Nach         | Bedingung / Auslöser                                                                                                                                                                                                                                                  |
| ------------ | ------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `queued`     | `running`    | Worker übernimmt Job (`attempt_count += 1`, `next_attempt_at` wird geleert); nur wenn Engine-Lease bzw. Cloud-Kontingent verfügbar                                                                                                                                    |
| `queued`     | `cancelled`  | Nutzer bricht Meeting-Verarbeitung oder einzelnen Job ab                                                                                                                                                                                                              |
| `running`    | `succeeded`  | Provider-Ergebnis validiert und Segmente in **derselben** SQLite-Transaktion eingefügt (3.3)                                                                                                                                                                          |
| `running`    | `retry_wait` | transitorischer Fehler (Klassifikation Abschnitt 6) **und** `attempt_count < max_attempts`; `next_attempt_at = now + backoff(attempt)`                                                                                                                                |
| `running`    | `failed`     | permanenter Fehler (Abschnitt 6) **oder** transitorischer Fehler bei `attempt_count >= max_attempts`; `last_error` wird gesetzt                                                                                                                                       |
| `running`    | `cancelled`  | Nutzerabbruch; laufender Provider-Aufruf wird best-effort abgebrochen, Teilergebnisse werden verworfen                                                                                                                                                                |
| `retry_wait` | `running`    | `next_attempt_at` erreicht und Worker frei                                                                                                                                                                                                                            |
| `retry_wait` | `cancelled`  | Nutzerabbruch während Backoff                                                                                                                                                                                                                                         |
| `failed`     | `queued`     | **explizite Nutzeraktion** („Erneut versuchen“); setzt `attempt_count = 0` und erzeugt einen frischen Idempotency-Key (3.3)                                                                                                                                           |
| `cancelled`  | `queued`     | **explizite Nutzeraktion** (erneute Verarbeitung); wie `failed → queued`                                                                                                                                                                                              |
| `running`    | `queued`     | **nur beim App-Neustart**: Crash-Recovery stuft in der DB persistierte `running`-Jobs ohne abgeschlossenes Ergebnis zurück (Abschnitt 3.4)                                                                                                                            |
| `succeeded`  | —            | **Terminal.** Kein Übergang heraus. Erneute Transkription mit anderem Modell/Provider erzeugt einen **neuen Job** mit neuem Idempotency-Key und ersetzt Segmente nur über den dokumentierten Re-Transkriptionspfad (Plan Abschnitt 11), niemals durch Zustandswechsel |

Nicht aufgeführte Übergänge sind verboten (insbesondere
`retry_wait → succeeded`, `failed → running`,
`succeeded → running/failed`).

### 3.3 Idempotency-Key und Doppelt-Einfüge-Schutz

Regeln (G3):

1. **Bildung:** `idempotency_key = SHA-256(meeting_id, provider, source,
timeline_start_ms, timeline_end_ms, sortierte audio_chunk_ids,
provider_settings_snapshot, requeue_generation)`.
   Der Key ist **unabhängig von `attempt`**: alle Wiederholungen desselben
   logischen Auftrags tragen denselben Key. `requeue_generation` wird nur bei
   `failed/cancelled → queued` erhöht — eine bewusste Wiederholung ist ein
   neuer logischer Auftrag.
2. **Eintragung:** Der Erfolg wird nur als Atomarer Schritt gebucht:
   `INSERT` aller `meeting_transcript_segments` (mit `job_id` und pro Segment
   `sequence`) **und** `UPDATE … state = 'succeeded'` in **einer** SQLite-
   Transaktion mit `UNIQUE(job_id, sequence)` und
   `UNIQUE(job_id, idempotency_key)` auf Jobebene. Schlägt ein Teil fehl,
   rollt die Transaktion zurück — es gibt keinen Zustand „succeeded ohne
   Segmente“ oder „Segmente ohne succeeded“.
3. **Wiederholung nach Crash zwischen Insert und Commit:** unmöglich, weil
   Insert und Commit dieselbe Transaktion sind.
4. **Wiederholung nach Crash nach Commit:** Der Job ist bereits `succeeded`;
   die Recovery (3.4) findet ihn so vor und startet ihn nicht erneut.
5. **Replay einer Provider-Antwort (z. B. doppelt zugestellte HTTP-Antwort,
   doppelt ausgeführter Worker):** Vor dem Insert prüft die Queue, ob für
   `(job_id, idempotency_key)` bereits Segmente existieren; ein Treffer führt
   zum No-op mit Ergebnis „bereits gebucht“, nicht zu einem zweiten Insert.
6. **Doppelte Jobs:** Ein `UNIQUE(meeting_id, idempotency_key)`-Constraint
   verhindert, dass Enqueue-Logik (z. B. VAD, das einen Chunk zweimal
   segmentiert) denselben logischen Auftrag zweimal anlegt.
7. **Wortdubletten an Segmentgrenzen** sind ein separates, im Plan
   verortetes Merge-Problem (Plan Abschnitt 8) und nicht Teil der
   Job-Idempotenz.

Damit kann ein wiederholter oder wiederholt ausgeführter Job keine
duplizierten Transkriptsegmente erzeugen.

### 3.4 Neustart-Wiederaufnahme (Plan Abschnitt 8 „Lokale Verarbeitung“)

Beim Programmstart gleicht die Queue persistierten Zustand und Realität ab:

- `queued`-Jobs bleiben `queued` und werden normal verarbeitet.
- `running`-Jobs werden als **unterbrochen** erkannt (es gibt keinen
  lebenden Worker mehr): Übergang `running → queued` **ohne** Erhöhung von
  `attempt_count` und ohne Backoff-Strafe — der Absturz war kein
  Provider-Fehler. Der Idempotency-Key bleibt unverändert, sodass ein
  teilweise (nie commitete) verarbeiteter Auftrag sauber neu läuft.
- `retry_wait`-Jobs behalten `next_attempt_at`; liegt dieser in der
  Vergangenheit, werden sie sofort fällig.
- `succeeded`, `failed`, `cancelled` bleiben unangetastet.
- Der Abgleich wird in derselben Migration/Startup-Transaktion wie der
  Session-Zustandsabgleich (Plan Abschnitt „Session-Zustände“) ausgeführt.

### 3.5 Retry-, Timeout- und Backoff-Parameter

Startwerte (konfigurierbare interne Konstanten, nicht Nutzereinstellungen;
Feinjustierung erfolgt in Phase 5 anhand gemessener Werte):

| Parameter             | Azure                                                       | Custom                    | Local                                       |
| --------------------- | ----------------------------------------------------------- | ------------------------- | ------------------------------------------- |
| Timeout pro Versuch   | 120 s + 0,5 s pro Audiominute des Pakets, max. 600 s        | 120 s                     | 10 × Echtzeitfaktor des Modells, min. 120 s |
| `max_attempts`        | 5                                                           | 5                         | 3                                           |
| Backoff-Basis         | 2 s                                                         | 2 s                       | 5 s                                         |
| Backoff-Faktor        | exponentiell, `base * 2^(attempt-1)`                        | wie Azure                 | wie Azure                                   |
| Jitter                | ±20 % deterministisch aus Job-ID (kein Wandern bei Replays) | wie Azure                 | wie Azure                                   |
| Maximalverzögerung    | 5 min pro Versuch                                           | 5 min                     | 2 min                                       |
| HTTP-Statuscode-Retry | 408, 429, 5xx                                               | wie Azure                 | —                                           |
| `Retry-After`-Header  | wird respektiert, gekappt auf Maximalverzögerung            | wird respektiert, gekappt | —                                           |

`next_attempt_at` wird persistiert, damit Backoff einen Neustart überlebt.
Während `retry_wait` läuft die Aufnahme unbehindert weiter; es gehen keine
Abschnitte verloren (Plan Abschnitt „Fehlerverhalten“).

---

## 4. Provider-Zuordnung (G2)

Jeder Provider bildet auf dasselbe Interface ab. Keines der folgenden
Details ist außerhalb der jeweiligen Implementierung sichtbar.

### 4.1 `LocalMeetingProvider`

- Führt Inferenz über die **vorhandene `TranscriptionManager`-Engine** aus
  (Plan: „Local STT must be handled by TranscriptionManager“); der Provider
  lädt keine zweite Modellinstanz.
- **Single-Engine-Lease:** Genau **ein** Meeting-STT-Worker hält zu jedem
  Zeitpunkt höchstens einen Lease auf die Engine. Der Lease wird zentral im
  `TranscriptionManager` koordiniert (dort existiert bereits die gemeinsam
  geschützte geladene Engine); der Meeting-Worker **fordert** den Lease an
  und **wartet an Segmentgrenzen**, nie innerhalb eines Segments.
- **Diktat-Priorität (Plan Abschnitt „Ressourcenlogik“):** Braucht ein
  interaktives Diktat die Engine, pausiert der Meeting-Worker an der nächsten
  sicheren Segmentgrenze, gibt den Lease ab und setzt danach automatisch
  fort. Meeting- und Diktiermodellwechsel werden ausschließlich im
  `TranscriptionManager` koordiniert; zwei große Modelle werden nie ungeprüft
  parallel geladen.
- Mic- und Systemsegmente werden nach Timeline und Fairness serialisiert
  (kein zweiter Worker, keine zweite Queue).
- Ergebnisse werden unmittelbar nach jedem Segment in SQLite geschrieben;
  das chronologische Transkript wird aus persistierten Segmenten abgeleitet.
- Fehler: Modell fehlt/nicht ladbar = **permanent** (Konfiguration);
  Inferenz-Panics werden abgefangen und als transitorischer Fehler des
  einzelnen Segments klassifiziert, der Engine-Zustand wird vor dem nächsten
  Versuch validiert.
- Kein Netzwerk, kein Secret-Bedarf; `validate()` prüft Modell-ID und
  Modellverfügbarkeit.

### 4.2 `AzureMeetingClient` (Fast Transcription)

- Nutzt ausschließlich die Fast-Transcription-API, **nicht** den
  Short-Audio-Endpunkt (60-s-Grenze). Region/Endpoint-Auflösung erfolgt wie
  im bestehenden Pfad aus `azure_speech_endpoint`, aber gegen den
  Fast-Transcription-Endpunkt.
- **Stereo-Paket-Grenze:** Aus den synchronisierten Mic-/System-Chunks wird
  ein temporäres Stereo-PCM16-WAV (16 kHz) gebaut: **links = Mikrofon,
  rechts = Systemaudio**, fest codiert und im Paket-Manifest vermerkt. Azure
  transkribiert beide Kanäle unabhängig; die Queue ordnet die zurückgegebenen
  Kanäle über diese feste Zuordnung wieder `microphone`/`system` zu. Lokale
  Verarbeitung bekommt die Quellen dagegen getrennt (Plan Abschnitt 7).
- **Bündelungs-Limits:** Abgeschlossene Capture-Chunks werden zu
  Azure-Auftragspaketen gebündelt. Jedes Paket bleibt **klar unter** den
  offiziellen Grenzen von 500 MB und 5 Stunden: harte interne Deckel von
  **400 MB** und **4 h** pro Paket plus konfigurierbare Zielpaketdauer
  (Startwert 10 min). Vor jedem Upload prüft der Client beide Deckel gegen
  die tatsächlich gepackte Datei und weist die Paketbildung sonst mit
  **permanenter** Konfigurations-/Segmentierungsfehlermeldung zurück
  (Nachbesserung durch kleinere Pakete, nicht durch Upload). Paketdauer und
  Parallelität sind konfigurierbare interne Werte (Plan Abschnitt 8).
- **Idempotenz:** Jedes Paket trägt die lokale, deterministische Job-ID als
  Idempotency-Referenz; Azure-Jobs besitzen Timeout, begrenzten
  Exponential-Backoff, persistierten Versuchszähler und letzte Fehlermeldung
  (Plan Abschnitt 8). Lokale Audiodaten bleiben bis zum bestätigten,
  gespeicherten Ergebnis erhalten.
- **Auth:** API-Key ausschließlich zur Requestzeit aus dem Secret Store
  gelesen (Abschnitt 7); niemals in Job, Manifest, Event oder Log.
- Fehlerklassifikation konkret: 401/403 (Key ungültig/entzogen),
  400/404/415 (Endpoint/Region/Format falsch), fehlender Key,
  Überschreiten der Paket-Limits, nicht unterstützte Sprache = **permanent**;
  408, 429, 5xx, DNS-/Verbindungs-/TLS-Fehler, Timeouts = **transitorisch**
  (Abschnitt 6).

### 4.3 `CustomMultipartProvider`

- Sendet begrenzte Audiosegmente als Multipart-`POST` an den konfigurierten
  Endpoint gemäß dem dokumentierten vorhandenen Vertrag
  (`speech_pipeline.rs::transcribe_custom`): `Content-Type: audio/wav` als
  Audio-Part, Header `X-Speech-Language`, optional
  `Authorization: Bearer <token>` aus dem Secret Store (Scope
  `speech-pipeline`, Id `custom`). Der Meeting-Pfad kapselt diese Details
  vollständig; die Queue sieht nur `MeetingSttProvider`.
- Der vorhandene Endpunkt wird **nicht automatisch als langzeitfähig
  betrachtet**: `capabilities()` meldet die konfigurierbaren, vom
  Betreiber bestätigten Limits; Standardannahme ist die gleiche Paketbildung
  wie bei Azure (begrenzte Segmentgröße), damit ein nicht langzeitfähiger
  Endpunkt nicht mit Stundenpaketen überfordert wird.
- Timeout, Retry, Authentifizierung und Antwortvalidierung verwenden
  **dieselbe Joblogik** wie Azure (Abschnitt 3.5, Abschnitt 6).
- Antwortvalidierung: Antwort muss ein vom bestehenden Parser
  (`extract_transcript`-Familie) erkennbares Transkript enthalten; ein
  strukturiertes Segmentformat mit relativen Offsets ist bevorzugt, fehlen
  Offsets, deckt das Ergebnis das gesamte Job-Fenster ab
  (`start_offset_ms = 0`, `end_offset_ms = Fensterlänge`) — niemals erfindet
  der Adapter Zwischenzeiten.
- Fehlerklassifikation konkret: 401/403 (Token falsch), 404/400 (Endpoint
  falsch), fehlender Endpoint, nicht parsebare Antwort bei Status 2xx nach
  Validierungsregeln = **permanent**; Verbindungs-/Timeout-/5xx-Fehler =
  **transitorisch**.

---

## 5. Zeitstempelkorrektur auf die Meeting-Timeline

- Provider liefern Offsets **relativ zum übergebenen Audio** (Abschnitt 1.2).
- Für Azure-Pakete sind die von Fast Transcription gelieferten Offsets (in
  100-ns-Ticks) pro Kanal in Millisekunden umgerechnet und gelten relativ zum
  Paketanfang.
- Korrektur: `timeline_start_ms = job.timeline_start_ms + start_offset_ms`,
  entsprechend für das Ende. Bei Azure wird zusätzlich die Kanalquelle
  (`microphone`/`system`) über die feste Stereo-Zuordnung (4.2) gesetzt.
- Das Ergebnis wird als `meeting_transcript_segments` mit `start_ms`/`end_ms`
  auf der gemeinsamen Timeline persistiert (Plan Abschnitt 9); das
  chronologische Transkript wird aus diesen persistierten Zeiten abgeleitet.
- Überlappungen durch providerabhängige Bündelung/Kontextüberlappung werden
  beim chronologischen Merge dedupliziert (doppelte Grenzwörter entfernen,
  Plan Abschnitt 8); die Korrektur selbst erfindet keine Zeiten und verschiebt
  nichts über die Job-Fenstergrenzen hinaus.

---

## 6. Fehlerklassifikation: permanent vs. transitorisch (G4)

Die Klassifikation erfolgt im Provider und wird als `ProviderError` mit
expliziter Kategorie an die Queue gemeldet; die Queue trifft anhand der
Kategorie die Zustandsentscheidung (`failed` vs. `retry_wait`, Abschnitt 3.2).

### 6.1 Permanent (Konfiguration/Auth — Wiederholung ist sinnlos)

Kategorie `permanent`, konkrete Erkennungskriterien:

| Signal                                        | Erkennung                                                                                    |
| --------------------------------------------- | -------------------------------------------------------------------------------------------- |
| Azure: Key ungültig/entzogen                  | HTTP 401/403 vom Fast-Transcription-Endpunkt                                                 |
| Azure: Endpoint/Region falsch                 | HTTP 400/404; oder `azure_region`-Auflösung scheitert (keine Region ableitbar)               |
| Azure: Format/Limits verletzt                 | HTTP 415; oder Paket überschreitet die internen Deckel (400 MB/4 h) vor dem Upload           |
| Azure/Custom: Key/Endpoint nicht konfiguriert | `validate()`-Befund „Secret/Endpoint fehlt“ vor Jobstart (Job wird gar nicht erst `running`) |
| Custom: Token falsch                          | HTTP 401/403                                                                                 |
| Custom: Endpoint falsch                       | HTTP 404/400/405                                                                             |
| Custom: unparsebare Antwort bei 2xx           | Validierungsregeln aus 4.3                                                                   |
| Local: Modell fehlt/nicht ladbar              | Modell-ID nicht in Modellverwaltung, Ladevorgang fehlschlägt deterministisch                 |
| Alle: nicht unterstützte Sprache              | Provider meldet Sprachablehnung als Validierungsfehler                                       |

Permanent heißt: Übergang nach `failed` mit verständlicher `last_error`-Meldung
und konkreter nächster Nutzerhandlung („Einstellungen öffnen“, Plan 6.2
Punkt 8). **Kein** automatischer Retry.

### 6.2 Transitorisch (Netzwerk/Last — Retry mit Backoff)

Kategorie `transitory`, konkrete Erkennungskriterien:

| Signal                                    | Erkennung                                                                                                       |
| ----------------------------------------- | --------------------------------------------------------------------------------------------------------------- |
| Netzwerk nicht erreichbar                 | Verbindungsfehler, DNS-Fehlschlag, „connection refused/reset“                                                   |
| Langsam/Überlastet                        | HTTP 408, 429 (inkl. `Retry-After`), 5xx (500/502/503/504)                                                      |
| Zeitüberschreitung                        | Versuch überschreitet den Timeout aus 3.5 ohne Antwort                                                          |
| TLS-Störungen                             | Handshake-Fehler, Zertifikats-Zeitfenster-Probleme (nicht: dauerhaft ungültiges Zertifikat — das ist permanent) |
| Local: vorübergehende Ressourcensituation | abgefangene Inferenz-Panics/OOM-artige Fehler des einzelnen Segments bei validem Modell                         |

Transitorisch heißt: `retry_wait` mit Backoff (3.5), solange
`attempt_count < max_attempts`; danach `failed` mit Hinweis auf manuelle
Wiederholung. Aufnahme läuft währenddessen lokal weiter; keine Abschnitte
gehen verloren (Plan „Fehlerverhalten“).

### 6.3 Unklassifizierbares

Ein Fehler, den der Provider keiner Kategorie zuordnen kann, wird als
**transitorisch** behandelt (konservativ: lieber ein sinnloser Versuch als
ein verlorener Abschnitt), bis die Klassifikation im Code präzisiert wird.
Die Zuordnung ist pro Provider implementiert und in Tests gegen die
Kriterienlisten aus 6.1/6.2 abgesichert.

---

## 7. Secret-Handhabung (G5)

**Invariante:** Kein Secret-Material — API-Keys, Bearer-Tokens,
Verbindungszeichenfolgen mit Schlüsseln — darf in **keinem** persistierten
Job-Feld, keinem Event-Payload, keiner Log-Zeile und keinem Manifest
(`manifest.json`) erscheinen.

Verweis auf die bestehende Infrastruktur:

- Secrets liegen ausschließlich im vorhandenen Secret Store
  (`src-tauri/src/secret_store.rs`): OS-Credential Store (Windows Credential
  Manager / macOS Keychain, Fallback `secrets.json` unter dem
  App-Datenverzeichnis), Scope `speech-pipeline`, IDs `azure` und `custom`.
  Es wird **kein zweiter Key-Mechanismus** eingeführt.
- Der Key wird ausschließlich **zur Requestzeit** via
  `secret_store::get_secret(SPEECH_PIPELINE_SCOPE, id)` gelesen und sofort an
  den HTTP-Client übergeben; er wird nicht in Strukturen zwischengespeichert,
  die persistiert oder als Event versendet werden.
- `provider_settings_snapshot` (Job-Zeile, Manifest, Events) enthält nur
  nicht-geheime Konfiguration: Endpoint-URL **ohne** eingebettete
  Credentials, Region, Modell-ID, Sprache, Paketgrenzen. Das bestehende
  Muster bleibt maßgeblich: `SecretMap` serialisiert als `{}` und redaktiert
  Debug-Ausgaben (`src-tauri/src/settings/`); Meeting-Tabellen speichern
  Secret-Referenzen (Provider + ID), nie Werte — wie in Plan Abschnitt 9
  festgeschrieben („Secrets werden ausschließlich über die vorhandene
  Secret-Verwaltung referenziert“).
- Logs loggen niemals `Authorization`-Header oder Key-Werte; Fehlermeldungen
  verwenden die bestehende `compact_error`-Redaktionspraxis (Antwortkörper
  gekürzt, Header niemals).
- Tauri-Events an das Frontend tragen Jobstatus, Fortschritt und
  redaktierte Fehlermeldungen, nie Konfigurationswerte mit Credentials.
- Die Invariante wird durch Tests abgesichert: Serialisierung von Job-Zeilen,
  Manifesten und Event-Payloads darf nach dem Entfernen des Secret-Store-
  Inhalts keine Änderung zeigen (Negativtest: bekannter Dummy-Key taucht
  nirgends auf — analog zum bestehenden `secret_map_debug_redacts_values`-
  Test in `src-tauri/src/settings/tests.rs`).

---

## 8. Zusammenfassung der Gate-Abdeckung

| Gate                                                      | Abschnitt     |
| --------------------------------------------------------- | ------------- |
| G1 Zustandsdiagramm inkl. Neustart-Wiederaufnahme         | 3.1, 3.2, 3.4 |
| G2 Provider-Zuordnung ohne Leaks in Aufrufer              | 4.1–4.3       |
| G3 Idempotenz gegen doppelte Segmente                     | 3.3           |
| G4 Permanent-vs-transitorisch mit Erkennungskriterien     | 6.1–6.3       |
| G5 Secret-Invariante mit Verweis auf Secret-Infrastruktur | 7             |
