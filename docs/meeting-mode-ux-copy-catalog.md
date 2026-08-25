# Meeting-Modus — UX-Copy-Katalog für Randzustände und Nutzertexte

Status: **Vorschau-Katalog (Proposal-only). Nichts davon ist implementiert oder
freigegeben.** Alle Texte sind Vorschläge für die englische i18n-Quelldatei.

Plan: [docs/meeting-mode-plan.md](meeting-mode-plan.md) — insbesondere
§6.2 (UX-Prinzipien, Regel 8), §6.8 (Farb-/Aktionslogik), §6.9 (Shortcut),
§6.11/§6.12 (laufende Ansicht, Processing), „Session-Zustände“ und
„Fehlerverhalten“ (nach §6.21), §11 (Löschen), §12 (Speicher- und Datenschutz).

Arbeitsstand geprüft am 22.08.2026 im Worktree dieses Tasks.

---

## 1. Verifizierungsstand gegen den Code (ehrliche Kennzeichnung)

Dieser Katalog beschreibt **geplante** Zustände. Der Meeting-Modus ist im
Repository **noch nicht implementiert**. Konkret geprüft:

| Prüfung                                                    | Methode                                                                                                             | Ergebnis                                                                                                                                                                                                |
| ---------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Meeting-Backend (`MeetingManager`, Session-Zustände, Jobs) | grep nach `meeting` in `src-tauri/src`                                                                              | Nur der Phase-1-Spike `src-tauri/src/bin/meeting_loopback_spike.rs` trifft zu. Kein Produktionscode.                                                                                                    |
| Meeting-Frontend (`src/components/meetings/**`)            | `glob src/components/meetings/**`                                                                                   | Keine Treffer. Existiert nicht.                                                                                                                                                                         |
| Meeting-i18n-Keys                                          | `grep "meeting"` über `src/`                                                                                        | Nur ein Azure-Beschreibungstext in `translation.json` (Zeile 179), der „meetings“ im Diktierkontext erwähnt. Keine Meeting-Keys.                                                                        |
| Tray-Infrastruktur                                         | `src-tauri/src/tray.rs`                                                                                             | **Existiert.** `TrayIconState::{Idle, Recording, Transcribing}`, Warn-Badge-Parameter, Menütexte werden aus dem `"tray"`-Abschnitt der `en/translation.json` abgeleitet (`src-tauri/src/tray_i18n.rs`). |
| Overlay-Fenster                                            | `src/overlay/RecordingOverlay.tsx`                                                                                  | **Existiert** (Diktat-Overlay). Ein Meeting-Overlay ist geplant, nicht vorhanden.                                                                                                                       |
| Systembenachrichtigungen                                   | `src-tauri/Cargo.toml`                                                                                              | **Kein** `tauri-plugin-notification` als Abhängigkeit vorhanden. Die „notification“-Surface ist daher doppelt unverifiziert: Zustand _und_ Surface existieren nicht.                                    |
| Sleep/Wake- und Geräteverlust-Messung                      | `docs/phase1-spike/tools/sleep-wake-test.ps1`, `device-loss-test.ps1`, CSV-Ergebnisse in `docs/phase1-spike/run-*/` | Messwerkzeuge und Rohdaten existieren (Phase-1-Spike). Eine produktive Behandlung dieser Zustände existiert nicht.                                                                                      |

**Folgerung für alle Einträge:** Jeder Eintrag unten ist als _Vorschlag_ zu
lesen, sofern nicht ausdrücklich „(existiert)“ an einer Surface steht. Kein
Text im Katalog ist als implementiert zu betrachten; keine der vorgeschlagenen
i18n-Pfade existiert derzeit in `src/i18n/locales/en/translation.json`.

## 2. Konventionen

### 2.1 Surfaces

| Surface        | Bedeutung                                             | Verifizierungsstand                                                                   |
| -------------- | ----------------------------------------------------- | ------------------------------------------------------------------------------------- |
| `main`         | Hauptfenster (React-App)                              | existiert (Diktat-Modus); Meeting-Ansichten geplant, nicht vorhanden                  |
| `overlay`      | kleines Aufnahme-Overlay, unabhängig vom Hauptfenster | Diktat-Overlay existiert (`src/overlay/`); Meeting-Overlay geplant                    |
| `tray`         | Tray-Icon, -Tooltip und -Menü                         | Icon-Zustände und Menü existieren (`src-tauri/src/tray.rs`); Meeting-Zustände geplant |
| `notification` | Betriebssystem-Benachrichtigung                       | **existiert nicht** (kein Notification-Plugin in `src-tauri/Cargo.toml`)              |

### 2.2 i18n

Alle Keys werden unter einem neuen Top-Level-Abschnitt `meeting` in
`src/i18n/locales/en/translation.json` vorgeschlagen (englische Quelldatei
zuerst, dann Übertrag in die übrigen Sprachdateien — wie im Plan §5 gefordert
und wie es ESLint/i18next heute erzwingen). Tray-Texte folgen der bestehenden
Sonderrolle: `tray_i18n.rs` leitet die Tray-Menüstrings aus dem `"tray"`-
Abschnitt der englischen Quelldatei ab; neue Tray-Einträge müssen daher
zusätzlich in `TrayStrings` (`src-tauri/src/tray_i18n.rs`) ergänzt werden.
`{{…}}` sind i18next-Interpolationen.

### 2.3 Gemeinsame Aktions-Buttons (§6.2 Regel 8)

Jeder Fehlerzustand nennt Buttons mit einer erkennbaren nächsten Handlung.
Vorgeschlagene wiederverwendbare Keys:

| Key                                          | Englischer Text                     |
| -------------------------------------------- | ----------------------------------- |
| `meeting.actions.tryAgain`                   | `Try again`                         |
| `meeting.actions.switchDevice`               | `Switch device`                     |
| `meeting.actions.openSettings`               | `Open settings`                     |
| `meeting.actions.openSystemSettings`         | `Open system settings`              |
| `meeting.actions.reconnect`                  | `Reconnect`                         |
| `meeting.actions.recordMicOnly`              | `Record microphone only`            |
| `meeting.actions.continueWithoutSystemAudio` | `Continue without system audio`     |
| `meeting.actions.keepRecording`              | `Keep recording`                    |
| `meeting.actions.endMeeting`                 | `End meeting`                       |
| `meeting.actions.pause`                      | `Pause`                             |
| `meeting.actions.resume`                     | `Resume`                            |
| `meeting.actions.workInBackground`           | `Continue in background`            |
| `meeting.actions.retryFailedSegments`        | `Retry failed segments`             |
| `meeting.actions.finishDictation`            | `Finish dictation`                  |
| `meeting.actions.cancelMeetingStart`         | `Cancel meeting start`              |
| `meeting.actions.openMeeting`                | `Open meeting`                      |
| `meeting.actions.deleteRecording`            | `Delete recording`                  |
| `meeting.actions.cancel`                     | `Cancel`                            |
| `meeting.actions.close`                      | `Close`                             |
| `meeting.actions.confirmDelete`              | `I understand — delete permanently` |

Primär-/Sekundärrolle je Eintrag angegeben; Farbe nach §6.8 (Rot erst bei
wirklicher Aufnahme/irreversibler Aktion, Orange = Aufmerksamkeit, Grün =
sicher abgeschlossen, Blau = Verarbeitung).

---

## 3. Session-Zustände (Plan „Session-Zustände“)

Der Plan definiert: `draft → starting → recording ⇄ paused → stopping →
transcribing → summarizing (optional) → completed`, jeder aktive Zustand kann
zusätzlich `recoverable_error` tragen; Nutzerabbrch führt zu
`stopped_unprocessed` oder `cancelled`.

### S1 — `draft` (Startdialog, nicht aufnehmend)

- **Surfaces:** `main`
- **i18n:** `meeting.session.draft.hint` = `Nothing is being recorded yet.`;
  `meeting.actions.startMeeting` = `Start meeting`
- **Buttons:** `Start meeting` (primär, Markenfarbe — nicht rot, §6.8),
  `Cancel` (sekundär)
- **Verifiziert:** nein — Startdialog existiert nicht.

### S2 — `starting` (Validierung/Countdown)

- **Surfaces:** `overlay`, `main`
- **i18n:** `meeting.session.starting.title` = `Starting meeting…`;
  `meeting.session.starting.body` = `Checking microphone and system audio…`;
  `meeting.session.starting.countdown` = `Recording starts in {{seconds}} s`;
  `meeting.session.starting.escHint` = `Press Esc to cancel`
- **Buttons:** `Cancel` (sekundär; bricht nur den Countdown ab, §6.9)
- **Verifiziert:** nein.

### S3 — `recording`

- **Surfaces:** `overlay`, `main` (fixierte Statusleiste, §6.11), `tray`
- **i18n:** `meeting.session.recording.title` = `Recording`;
  `meeting.session.recording.saved` = `Audio saved`;
  `meeting.session.recording.provider.local` = `Local`;
  `meeting.session.recording.provider.cloud` = `Cloud ({{provider}})`;
  Tray: `tray.meetingRecording` = `Meeting: recording ({{duration}})`;
  Einwilligungsanzeige dauerhaft sichtbar: `meeting.consent.indicator.label` =
  `This meeting is being recorded.`
- **Buttons:** `Pause` (neutral), `End meeting` (rot, Stop-Symbol + Text)
- **Cloud-Hinweis (§12):** Bei Azure-STT zeigt die Statusleiste zusätzlich
  `meeting.cloud.active.azure` = `Audio is being uploaded to Azure.` —
  hochgeladen werden die synchronisierten Stereo-Audiopakete (links Mikrofon,
  rechts Systemaudio, §8). Bei lokaler STT verlässt nichts das Gerät.
- **Verifiziert:** nein. Tray-Icon-Zustand `Recording` existiert bereits fürs
  Diktat (`src-tauri/src/tray.rs`) und würde wiederverwendet/erweitert.

### S4 — `paused`

- **Surfaces:** `overlay`, `main`, `tray`
- **i18n:** `meeting.session.paused.title` = `Paused`;
  `meeting.session.paused.body` = `No audio is being saved or uploaded while
paused.`; Tray: `tray.meetingPaused` = `Meeting: paused`
- **Buttons:** `Resume` (primär), `End meeting` (rot)
- **Verifiziert:** nein.

### S5 — `stopping`

- **Surfaces:** `overlay`, `main`
- **i18n:** `meeting.session.stopping.title` = `Finishing recording…`;
  `meeting.session.stopping.body` = `Finalising the audio files. This takes
only a moment.`; danach `meeting.session.stopping.done` = `Recording
ended — audio saved.`
- **Buttons:** keine; `End meeting` ist während der Finalisierung deaktiviert
  (Zustand ist kurzlebig, §6.9 Schritt 4)
- **Verifiziert:** nein.

### S6 — `transcribing`

- **Surfaces:** `main` (Processing-Checkliste, §6.12), `tray`
- **i18n:** `meeting.processing.title` = `Processing meeting`;
  `meeting.processing.step.saved` = `Recording saved safely`;
  `meeting.processing.step.prepared_one` = `{{count}} of {{total}} audio
segments prepared`;
  `meeting.processing.step.transcribed_one` = `{{count}} of {{total}} segments
transcribed`;
  `meeting.processing.closeOk` = `You can close this window`;
  `meeting.processing.backlog` = `{{count}} segments waiting`;
  Tray: `tray.meetingProcessing` = `Meeting: processing`
- **Buttons:** `Continue in background` (primär), `Retry failed segments`
  (nur sichtbar, wenn Fehler vorliegen)
- **Cloud-Hinweis (§12):** bei Azure sichtbar:
  `meeting.cloud.active.azureProcessing` = `Audio segments are being uploaded
to Azure for transcription.` — hochgeladen werden begrenzte Audiopakete der
  fertigen Capture-Chunks; bei lokaler Verarbeitung erscheint diese Zeile nie.
- **Verifiziert:** nein.

### S7 — `summarizing` (optional)

- **Surfaces:** `main`, `tray`, `notification` (Abschluss)
- **i18n:** `meeting.processing.summarizing` = `Creating summary…`;
  Abschluss: `meeting.notify.completed.title` = `Meeting ready`;
  `meeting.notify.completed.body` = `“{{title}}” has been processed.`
- **Buttons:** `Continue in background`; nach Abschluss `Open meeting`
- **Cloud-Hinweis (§12):** bei Cloud-LLM sichtbar:
  `meeting.cloud.active.llm` = `Transcript text is being sent to {{provider}}
to create the summary.` — gesendet wird **nur Transkripttext** (bei langen
  Meetings die Map-Reduce-Textfenster), **kein Audio**.
- **Verifiziert:** nein; `notification`-Surface existiert im Code nicht.

### S8 — `completed`

- **Surfaces:** `main`, `notification`
- **i18n:** `meeting.session.completed.title` = `Meeting completed`;
  `meeting.session.completed.saved` = `Transcript and summary are stored on
this device.`
- **Buttons:** `Open meeting` (primär), `Close`
- **Verifiziert:** nein.

### S9 — `recoverable_error` (Muster, überlagerbar auf jeden aktiven Zustand)

- **Surfaces:** `main` (Banner), `overlay` (Kurzzeile), `tray`
  (Warn-Badge; Badge-Mechanismus existiert bereits in
  `tray::get_icon_path(..., warning)`)
- **i18n:** `meeting.errors.recoverable.title` = `Something needs your
attention`; `meeting.errors.recoverable.detail` = `{{detail}}`
- **Buttons:** kontextabhängig aus §6.2-Regel-8-Repertoire (`Try again`,
  `Switch device`, `Open settings`)
- **Verifiziert:** nein (Muster); Tray-Warn-Badge-Infrastruktur existiert.

### S10 — `stopped_unprocessed` / `cancelled` (Nutzerabbruch)

- **Surfaces:** `main`, `notification`
- **i18n:** `meeting.session.cancelled.title` = `Recording cancelled`;
  `meeting.session.cancelled.body` = `The audio captured so far has been kept.
Nothing was deleted.`
- **Buttons:** `Open meeting` (primär), `Delete recording` (sekundär, führt in
  den Löschdialog P8–P10)
- **Verifiziert:** nein. Plan fordert ausdrücklich: Abbruch darf keinen
  stillen Datenverlust erzeugen.

---

## 4. Geräte- und Aufnahme-Randzustände

Quelle: Plan „Fehlerverhalten“, „Startvalidierung“, §7 (Loopback).

### E1 — Mikrofon fällt während der Aufnahme aus

- **Surfaces:** `overlay`, `main` (Statusleisten-Warnung), `tray` (Warn-Badge)
- **i18n:** `meeting.errors.micLost.title` = `Microphone disconnected`;
  `meeting.errors.micLost.body` = `Your microphone stopped working. System
audio is still being recorded.`
- **Buttons:** `Reconnect` (primär = `tryAgain`), `Switch device`,
  `Open settings`
- **Verifiziert:** nein. Messwerkzeug existiert
  (`docs/phase1-spike/tools/device-loss-test.ps1`), Produktionslogik nicht.

### E2 — Systemausgabe-Gerät fällt während der Aufnahme aus

- **Surfaces:** `overlay`, `main`, `tray` (Warn-Badge)
- **i18n:** `meeting.errors.systemLost.title` = `System audio disconnected`;
  `meeting.errors.systemLost.body` = `Your microphone is still being recorded.
The sound of the other participants is currently not being captured.`
- **Buttons:** `Reconnect` (primär), `Continue without system audio`,
  `Switch device`, `Open settings`
- **Verifiziert:** nein (siehe E1).

### E3 — Standardgerät wechselt während der Aufnahme

- **Surfaces:** `overlay`, `main`
- **i18n (bindendes Gerät, Standardoption):**
  `meeting.errors.defaultChanged.title` = `Default device changed`;
  `meeting.errors.defaultChanged.body` = `Your output device has changed.
Speak keeps recording from “{{device}}”.`;
  **i18n (Option „Standardgerät folgen“ aktiv):**
  `meeting.errors.defaultChanged.followed` = `A new source section was started
on “{{device}}”.`
- **Buttons:** `Switch device`, `Open settings` (kein stiller Wechsel, §12
  Fehlerverhalten)
- **Verifiziert:** nein.

### E4 — Keine Systemlautstärke / Stille (Informationszustand, kein Fehler)

- **Surfaces:** `main`, `overlay`
- **i18n:** `meeting.warnings.silence.title` = `No system sound detected`;
  `meeting.warnings.silence.body` = `The selected output device is silent. If
you hear sound through another device, the wrong one may be selected.`
- **Buttons:** `Test system audio` (neu: `meeting.actions.testSystemAudio`),
  `Open settings`, `Keep recording`
- **Verifiziert:** nein. Plan: „Stille ist kein Fehler“.

### E5 — Loopback beim Start nicht verfügbar → Mikrofon-Only-Fallback

- **Surfaces:** `main` (Startdialog), `overlay`
- **i18n:** `meeting.errors.noLoopback.title` = `System audio can't be
captured`; `meeting.errors.noLoopback.body` = `Speak can't listen to this
output device. You can record the microphone only.`;
  `meeting.session.micOnly.badge` = `Microphone only — system audio is not
being recorded`
- **Buttons:** `Record microphone only` (primär; Zustand wird laut Plan
  deutlich in der Session gespeichert und angezeigt), `Switch device`,
  `Open settings`
- **Verifiziert:** nein.

### E6 — Output-Gerät im Exclusive-Mode (WASAPI)

- **Surfaces:** `main` (Startdialog oder Statusleiste), `overlay`
- **i18n:** `meeting.errors.exclusiveMode.title` = `Output device is busy`;
  `meeting.errors.exclusiveMode.body` = `“{{device}}” is running in exclusive
mode, so system audio can't be captured.`
- **Buttons:** `Switch device` (primär), `Record microphone only`,
  `Open settings`
- **Verifiziert:** nein. Plan §7 fordert, diesen Fehler verständlich statt als
  Rohfehler zu zeigen.

### E7 — Datenträger fast voll (Frühwarnung)

- **Surfaces:** `main` (Startdialog und Statusleiste), `overlay`
- **i18n:** `meeting.warnings.diskLow.title` = `Storage almost full`;
  `meeting.warnings.diskLow.body` = `{{freeGb}} GB free. A one-hour meeting
with microphone and system audio needs roughly 0.25 GB.`
- **Buttons:** `Keep recording` (primär), `End meeting`, `Open settings`
- **Verifiziert:** nein. Bedarfswert 230 MB/h ≈ 0,25 GB stammt aus Plan §12
  (PCM16, 16 kHz, zwei Mono-Spuren).

### E8 — Datenträger voll → kontrollierter Stopp

- **Surfaces:** `overlay`, `main`, `tray`, `notification`
- **i18n:** `meeting.errors.diskFull.title` = `Recording stopped — storage
full`; `meeting.errors.diskFull.body` = `Speak stopped the recording to
protect your data. Everything captured so far is saved.`
- **Buttons:** `Open meeting` (primär), `Open settings`
- **Verifiziert:** nein.

---

## 5. Verarbeitungs- und Cloud-Randzustände

### E9 — Azure/Custom nicht erreichbar (`retry_wait`)

- **Surfaces:** `main` (Processing-Checkliste und Statusleiste), `overlay`
- **i18n:** `meeting.errors.cloudUnreachable.title` = `Cloud transcription is
waiting`; `meeting.errors.cloudUnreachable.body` = `{{provider}} can't be
reached. Recording continues locally. {{count}} segments are waiting and
will be sent automatically.`; `meeting.errors.cloudUnreachable.waiting` =
  `Retrying in {{seconds}} s`
- **Buttons:** `Try again` (primär), `Continue in background`, `Open settings`
- **Cloud-Hinweis (§12):** Solange der Provider nicht erreichbar ist, **verlässt
  kein Audiobit das Gerät**; die lokalen Audiodaten bleiben bis zum bestätigt
  gespeicherten Ergebnis erhalten (Plan §8).
- **Verifiziert:** nein.

### E10 — Permanenter Authentifizierungs-/Konfigurationsfehler (Cloud)

- **Surfaces:** `main`, `overlay`
- **i18n:** `meeting.errors.cloudConfig.title` = `Cloud transcription can't
start`; `meeting.errors.cloudConfig.body` = `The {{provider}} key or region
looks invalid. No audio was uploaded.`
- **Buttons:** `Open settings` (primär — fokussierter Reparaturdialog laut
  „Startvalidierung“), `Try again`, `Continue in background`
- **Cloud-Hinweis (§12):** ausdrücklich: **es wurde nichts hochgeladen.**
- **Verifiziert:** nein.

### E11 — Lokales Modell zu langsam / Backlog wächst

- **Surfaces:** `main` (nur wenn tatsächlich Backlog besteht, §6.11), `overlay`
- **i18n:** `meeting.warnings.backlog.title` = `Transcription is behind`;
  `meeting.warnings.backlog.body` = `{{count}} segments are waiting. Recording
and saving continue normally — nothing is lost.`
- **Buttons:** `Continue in background` (primär), `Open settings`
- **Verifiziert:** nein.

### E12 — App-Absturz → Crash-Recovery-Banner beim Neustart

- **Surfaces:** `main` (Banner, geplante Komponente `MeetingRecoveryBanner.tsx`),
  `notification`
- **i18n:** `meeting.recovery.title` = `Recover meeting recording?`;
  `meeting.recovery.body` = `Speak was closed unexpectedly during “{{title}}”.
The audio captured up to that point was saved and can be processed.`
- **Buttons:** `Recover and process` (neu: `meeting.actions.recover`, primär),
  `Decide later` (neu: `meeting.actions.decideLater`), `Delete recording`
  (sekundär, führt in P8–P10)
- **Verifiziert:** nein. Plan: Session bleibt nie dauerhaft in `recording`;
  offene Chunks werden als wiederherstellbar markiert, finalisiert und zur
  Verarbeitung angeboten.

### E13 — Sleep/Wake während der Aufnahme

- **Surfaces:** `overlay`, `main`, `notification`
- **i18n:** `meeting.errors.slept.title` = `Recording was interrupted`;
  `meeting.errors.slept.body` = `Your PC went to sleep at {{time}}. The
recording stopped safely and can be continued.`
- **Buttons:** `Resume` (primär), `End meeting`, `Delete recording`
- **Verifiziert:** nein. Sleep/Wake-Verhalten wurde nur im Phase-1-Spike
  gemessen (`docs/phase1-spike/tools/sleep-wake-test.ps1`,
  `docs/phase1-spike/run-*/meeting_loopback_spike_120min.csv`); eine
  produktive Behandlung existiert nicht.

### E14 — Gerätewechsel während Pause (Resume-Validierung)

- **Surfaces:** `main` (Resume-Dialog), `overlay`
- **i18n:** `meeting.warnings.devicesChanged.title` = `Devices changed while
paused`; `meeting.warnings.devicesChanged.body` = `Microphone is now
“{{mic}}”, output is “{{output}}”. A new source section will be started in
the transcript.`
- **Buttons:** `Resume` (primär, Text: `Continue with these devices` —
  eigener Key `meeting.actions.resumeWithDevices`), `Switch device`,
  `Open settings`
- **Verifiziert:** nein. Plan „Pause“: Beim Fortsetzen werden beide Geräte
  erneut validiert; Wechsel wird als neuer Quellenabschnitt dokumentiert.

### E15 — Diktat läuft, Meetingstart angefordert (§6.10)

- **Surfaces:** `main`, `overlay`
- **i18n:** `meeting.conflicts.dictation.title` = `A dictation is running`;
  `meeting.conflicts.dictation.body` = `Finish the dictation before starting
a meeting.`
- **Buttons:** `Finish dictation` (primär), `Cancel meeting start`
- **Verifiziert:** nein.

### E16 — Provider-Key oder Meeting-Modell fehlt (Startvalidierung)

- **Surfaces:** `main` (fokussierter Reparaturdialog im Startfluss)
- **i18n:** `meeting.errors.missingKey.title` = `Setup incomplete`;
  `meeting.errors.missingKey.body.cloud` = `No {{provider}} key is configured
for cloud transcription.`; `meeting.errors.missingKey.body.local` = `The
local meeting model isn't downloaded yet.`; `meeting.errors.missingKey.body.summary`
  = `No summary provider is configured.`
- **Buttons:** `Open settings` (primär), `Try again`, `Cancel`
- **Verifiziert:** nein.

### E17 — Mikrofon-Berechtigung fehlt (Windows)

- **Surfaces:** `main` (Startdialog), `notification`
- **i18n:** `meeting.errors.micPermission.title` = `Microphone access
required`; `meeting.errors.micPermission.body` = `Speak needs microphone
access to record meetings. Grant access in Windows privacy settings.`
- **Buttons:** `Open system settings` (primär), `Cancel`
- **Verifiziert:** nein für den Meeting-Kontext. Für den Diktat-Kontext
  existieren vergleichbare Texte bereits
  (`errors.micPermissionDenied.*` in `src/i18n/locales/en/translation.json`,
  Zeilen 729–746) — Struktur und Ton orientieren sich daran.

---

## 6. Datenschutz, Einwilligung, Aufbewahrung, Löschen

Quellen: Plan §12 (Speicher- und Datenschutzlogik), §11 (Löschen), §6.9
(Stop-Bestätigung), §6.14 (irreversible Löschung nennt Bestandteile).

### P1 — Consent-Hinweis vor dem Start

- **Surfaces:** `main` (Startdialog)
- **i18n:** `meeting.consent.beforeStart.title` = `Before you record`;
  `meeting.consent.beforeStart.body` = `Recording a conversation requires the
consent of everyone involved. Speak shows a recording indicator for the
whole meeting and is not designed as a stealth recorder.`;
  `meeting.consent.beforeStart.acknowledge` = `Everyone involved has consented`
- **Buttons:** `Start meeting` (erst aktiv, wenn Bestätigung gesetzt ist),
  `Cancel`
- **Verifiziert:** nein.

### P2 — Consent-Indikator während der gesamten Aufnahme

- **Surfaces:** `overlay`, `main`, `tray` (Tooltip)
- **i18n:** `meeting.consent.indicator.label` = `This meeting is being
recorded.` (dauerhaft sichtbar, nicht nur Farbe — §6.8)
- **Buttons:** keine (passiver Indikator)
- **Verifiziert:** nein.

### P3 — Cloud-Upload-Hinweis **vor** dem Start (Azure-STT)

- **Surfaces:** `main` (Startdialog, wenn Azure gewählt)
- **i18n:** `meeting.cloud.notice.azure.title` = `Cloud transcription
selected`; `meeting.cloud.notice.azure.body` = `Audio from your microphone
and system audio will be uploaded to Microsoft Azure and transcribed there.
With local processing, nothing leaves this device.`;
  `meeting.cloud.notice.azure.detail` = `Uploads happen in small packages
(each well under 500 MB).`
- **Buttons:** `Start meeting` (primär, bestätigt den Upload), `Process
locally` (neu: `meeting.actions.useLocal`), `Open settings`
- **Cloud-Hinweis (§12):** Gerät verlassen: **Audiopakete** (Stereo: links
  Mikrofon, rechts Systemaudio) an Azure Speech Fast Transcription. Zurück
  kommen Transkriptsegmente. Audio bleibt bis zum bestätigten Ergebnis lokal
  gespeichert.
- **Verifiziert:** nein.

### P4 — Cloud-Upload-Hinweis **während** der Verarbeitung (Azure/Custom)

- **Surfaces:** `main` (Checkliste/Statusleiste), `overlay`, `tray` (Tooltip)
- **i18n:** `meeting.cloud.active.azureProcessing` = `Audio segments are being
uploaded to {{provider}} for transcription.`
- **Buttons:** `Open settings` (sekundär), sonst passiver Status
- **Cloud-Hinweis (§12):** siehe P3; Custom-STT erhält begrenzte
  Audiosegmente über den vorhandenen Multipart-Vertrag (Plan §8).
- **Verifiziert:** nein.

### P5 — Cloud-Upload-Hinweis für die Zusammenfassung (LLM)

- **Surfaces:** `main` (vor Start im Startdialog-Schalter und während S7)
- **i18n (vorher):** `meeting.cloud.notice.llm.title` = `Cloud summary
selected`; `meeting.cloud.notice.llm.body` = `The transcript (text only —
no audio) will be sent to {{provider}} to create the summary.`;
  **i18n (während):** `meeting.cloud.active.llm` = `Transcript text is being
sent to {{provider}}.`
- **Buttons:** `Use {{provider}}` (neu: `meeting.actions.useCloudSummary`,
  primär), `Summarize locally` (neu: `meeting.actions.summarizeLocally`),
  `Don't summarize` (neu: `meeting.actions.disableSummary`)
- **Cloud-Hinweis (§12):** Gerät verlässt: **nur Transkripttext** (bei langen
  Meetings Map-Reduce-Textfenster). Kein Audio. Bei lokaler Zusammenfassung
  verlässt nichts das Gerät.
- **Verifiziert:** nein.

### P6 — Aufbewahrungsfrage nach dem ersten Meeting

- **Surfaces:** `main` (Dialog nach erster vollständig erfolgreicher
  Verarbeitung)
- **i18n:** `meeting.retention.question.title` = `How long should meeting
audio be kept?`; `meeting.retention.question.body` = `Until you choose,
nothing is deleted automatically. Failed or unconfirmed transcription jobs
always prevent automatic deletion.`;
  Optionen: `meeting.retention.option.deleteNow` = `Delete immediately`,
  `meeting.retention.option.7days` = `Keep for 7 days`,
  `meeting.retention.option.30days` = `Keep for 30 days`,
  `meeting.retention.option.forever` = `Keep permanently`
- **Buttons:** die vier Optionsbuttons; zusätzlich `Decide later`
  (= `meeting.actions.decideLater`) zum vorläufigen Schließen. Bis eine Option
  gewählt wird, wird nichts automatisch gelöscht (Plan §12).
- **Verifiziert:** nein.

### P7 — Stop-Bestätigung per Shortcut (§6.9)

- **Surfaces:** `overlay`
- **i18n:** `meeting.stopConfirm.title` = `End meeting?`;
  `meeting.stopConfirm.body` = `Press {{shortcut}} again or Enter to confirm.
Press Esc to keep recording.`; `meeting.stopConfirm.timeout` = `Confirmation
expired — recording continues.`
- **Buttons:** `End meeting` (rot, bestätigt), `Keep recording` (Esc)
- **Verifiziert:** nein.

### P8 — Löschstufe 1: nur Rohaufnahme löschen (§11)

- **Surfaces:** `main` (Löschdialog in Meeting-Detailansicht und Listenmenü)
- **i18n:** `meeting.delete.level1.label` = `Delete audio only`;
  `meeting.delete.level1.body` = `Removes the recording. Transcript and
summary are kept.`
- **Buttons:** `Delete audio only` (bestätigt), `Cancel`
- **Verifiziert:** nein.

### P9 — Löschstufe 2: Transkript und Zusammenfassung löschen (§11)

- **Surfaces:** `main`
- **i18n:** `meeting.delete.level2.label` = `Delete transcript and summary`;
  `meeting.delete.level2.body` = `Removes the transcript and summary. The
audio recording is kept.`
- **Buttons:** `Delete transcript and summary` (bestätigt), `Cancel`
- **Verifiziert:** nein.

### P10 — Löschstufe 3: gesamtes Meeting endgültig löschen (§11, §6.14)

- **Surfaces:** `main` (zweistufig: Optionsdialog → finale Bestätigung)
- **i18n:** `meeting.delete.level3.label` = `Delete entire meeting`;
  `meeting.delete.level3.body` = `Permanently deletes “{{title}}” with all of
its parts:`; `meeting.delete.level3.itemAudio` = `Audio recordings`,
  `meeting.delete.level3.itemTranscript` = `Transcript`,
  `meeting.delete.level3.itemSummary` = `Summary`;
  `meeting.delete.level3.irreversible` = `This cannot be undone.`;
  finale Bestätigung: `meeting.delete.level3.confirm` = `I understand —
delete permanently`
- **Buttons:** `I understand — delete permanently` (rot, erst nach explizitem
  Klick; nennt konkret Audio, Transkript und Zusammenfassung — §6.14),
  `Cancel`
- **Verifiziert:** nein.

---

## 7. Gate-Selbstprüfung

| Gate                                                               | Nachweis                                                                                                                                                                                                 |
| ------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| G1 — alle Session-Zustände + ≥ 12 Fehler-/Randzustände mit Buttons | S1–S10 (10 Zustände inkl. `stopped_unprocessed`/`cancelled` und `recoverable_error`-Muster) und E1–E17 (17 Randzustände), jeder mit Buttons-Abschnitt. P1–P10 ergänzen Consent/Cloud/Retention/Löschung. |
| G2 — Surface + i18n-Key-Pfad je Eintrag                            | Jeder Eintrag hat **Surfaces:** und **i18n:**-Felder mit konkreten Key-Pfaden unter `meeting.*` bzw. `tray.*`.                                                                                           |
| G3 — Cloud-Zustände nennen die das Gerät verlassenden Daten        | P3, P4, P5, E9, E10 sowie S3/S6/S7 enthalten explizite „Cloud-Hinweis“-Angaben (Audio vs. nur Transkripttext, nichts bei lokal).                                                                         |
| G4 — nichts als implementiert behauptet                            | Abschnitt 1 weist per grep/glob/Cargo.toml nach, dass der Meeting-Modus nicht implementiert ist; jeder Eintrag trägt eine **Verifiziert:**-Zeile; Header kennzeichnet den Katalog als proposal-only.     |

## 8. Offene Punkte für die umsetzende Phase

1. `tauri-plugin-notification` muss für die `notification`-Surface neu
   eingeführt werden (derzeit nicht in `src-tauri/Cargo.toml`).
2. Neue Tray-Einträge (`tray.meetingRecording`, `tray.meetingPaused`,
   `tray.meetingProcessing`) erfordern eine Erweiterung von `TrayStrings` in
   `src-tauri/src/tray_i18n.rs` — dort werden die Tray-Strings generiert, nicht
   zur Laufzeit aus i18next gelesen.
3. Die konkreten Zahlenwerte in E7 (0,25 GB/h) und P3 (500-MB-Pakete) stammen
   aus dem Plan (§12 bzw. §8) und sind bei Implementierung gegen Messwerte des
   Phase-1-/Phase-2-Spikes zu bestätigen.
4. Alle Texte sind englische Erstentwürfe; Übersetzungen und die Prüfung durch
   `scripts/check-translations.ts` erfolgen erst bei tatsächlicher Anlage der
   Keys in `src/i18n/locales/en/translation.json`.
