# Meeting-Modus — Phase-0-Baseline

Status: **Baseline-Aufzeichnung, Phase-1-Vorbereitung (Checkpoint-Arbeitsstand gesichert)**
Plan: [docs/meeting-mode-plan.md](meeting-mode-plan.md), Phase 0 (Abschnitt 13) und
Testmatrix (Abschnitt 14)
Aufgenommen am: 22.08.2026, 06:13 (+02:00)

## 1. Checkpoint

| Punkt                  | Wert                                                    |
| ---------------------- | ------------------------------------------------------- |
| Checkpoint-Branch      | `checkpoint/custom-stt-pre-meeting`                     |
| Checkpoint-Commit      | `2355da6c7ebb16701c6f9ce82be584f03ce9e486`              |
| Commit-Betreff         | `docs: add meeting-mode planning notes`                 |
| Commit-Datum           | 2026-08-22 05:08:18 +0200                               |
| identisch mit HEAD von | `feat/custom-stt-pipeline`                              |
| Push-Remote            | `fork/feat/custom-stt-pipeline` (Branch-Tracking aktiv) |

Zum Zeitpunkt der Baseline-Ausführung enthielt der Arbeitbaum auf
`feat/custom-stt-pipeline` zusätzlich 14 uncommittete, modifizierte Dateien
(`git status --porcelain`):

```text
 M src-tauri/src/commands/audio.rs
 M src-tauri/src/commands/history.rs
 M src-tauri/src/commands/mod.rs
 M src-tauri/src/commands/models.rs
 M src-tauri/src/commands/transcription.rs
 M src-tauri/src/lib.rs
 M src-tauri/src/managers/audio.rs
 M src-tauri/src/managers/model/discovery.rs
 M src-tauri/src/managers/model/mod.rs
 M src-tauri/src/settings/mod.rs
 M src-tauri/src/settings/store.rs
 M src-tauri/src/settings/types.rs
 M src-tauri/src/shortcut/mod.rs
 M src/stores/settingsStore.ts
```

**Aktualisierung (22.08.2026):** Diese Änderungen sind inzwischen gesichert.
Die 14 Dateien sind auf `feat/custom-stt-pipeline` im Bereich
`checkpoint/custom-stt-pre-meeting..HEAD` committet — 13 Dateien durch
`b677107` (`refactor: make settings persistence transactional`) und
`src/stores/settingsStore.ts` durch `a7226b6`
(`fix: serialize settings store updates and surface backend errors`); die
zugehörige Dokumentation liegt in `8fd9dee`
(`feat: add meeting loopback measurement spike binary`) und `beb286e`
(`docs: record meeting-mode phase-0 baseline`). Vor dieser Dokumentaktualisierung
war der Arbeitsbaum sauber (`git status`: nothing to commit, working tree clean).

## 2. Automatisierte Baseline-Ergebnisse

Erste Messung vom Morgen des 22.08.2026 (Windows x64, rustc 1.98.0,
Node v24.19.0); `bun` ist in dieser Shell-Umgebung nicht im PATH, daher wurden
die Paket-Skripte über die äquivalenten lokalen Binaries aus
`node_modules\.bin` bzw. direkt ausgeführt.

| #   | Kommando                                                                     | Arbeitsverzeichnis | Ergebnis                           | Exit-Code |
| --- | ---------------------------------------------------------------------------- | ------------------ | ---------------------------------- | --------- |
| 1   | `node_modules\.bin\eslint.cmd src` (entspricht `bun run lint`)               | Repo-Root          | **PASS**                           | 0         |
| 2   | `node_modules\.bin\prettier.cmd --check .` (Teil von `bun run format:check`) | Repo-Root          | **FAIL**                           | 1         |
| 3   | `cargo fmt -- --check` (Teil von `bun run format:check`)                     | `src-tauri`        | **PASS**                           | 0         |
| 4   | `node scripts/check-translations.ts` (`bun run check:translations`)          | Repo-Root          | **PASS**                           | 0         |
| 5   | `cargo test`                                                                 | `src-tauri`        | **FAIL** (Build-Skript, siehe 2.3) | 101       |

Nach der Behebung des `transcribe-cpp-sys`-Buildproblems (Abschnitt 2.3) wurde
die Backend-Baseline am selben Tag erneut ausgeführt:

| #   | Kommando                                                               | Arbeitsverzeichnis | Ergebnis                                                                                                                                                                                                 | Exit-Code |
| --- | ---------------------------------------------------------------------- | ------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------- |
| 6   | `cargo test`                                                           | `src-tauri`        | **PASS** — 220 Tests: 208 (lib) + 12 (bin `meeting_loopback_spike`) bestanden, 0 fehlgeschlagen; Log `src-tauri/target/baseline-tools/logs/11-cargo-test-serial-fix.log`                                 | 0         |
| 7   | `cargo clippy --all-targets`                                           | `src-tauri`        | **PASS** — 0 Warnungen nach Fix der 6 Doc-Kommentar-Indentierungs-Warnungen in `src/bin/meeting_loopback_spike.rs`; Logs `13-cargo-clippy-final.log` / `14-cargo-clippy-final.log` im selben Verzeichnis | 0         |
| 8   | `cargo fmt -- --check` (nach Clippy-Fix)                               | `src-tauri`        | **PASS**                                                                                                                                                                                                 | 0         |
| 9   | `cargo test` (unabhängiger Verifikationslauf, 22.08.2026 08:0x +02:00) | `src-tauri`        | **PASS** — erneut 208 (lib) + 12 (`meeting_loopback_spike`) = 220 bestanden, 0 fehlgeschlagen; Log `15-cargo-test-verification.log`                                                                      | 0         |
| 10  | `cargo clippy --all-targets` (Verifikationslauf)                       | `src-tauri`        | **PASS** — 0 Lint-Warnungen (einzige „warning:"-Zeilen sind Build-Skript-`cargo:warning=`-Ausgaben); Log `16-cargo-clippy-verification.log`                                                              | 0         |
| 11  | `cargo fmt -- --check` (Verifikationslauf)                             | `src-tauri`        | **PASS**                                                                                                                                                                                                 | 0         |

Die Zeilen 9–11 sind ein unabhängiger Wiederholungslauf in einer frischen
Sandbox-Shell ohne geerbte Maschinen-Umgebung: `VULKAN_SDK` musste dort wie in
Abschnitt 2.3 beschrieben explizit auf `C:\VulkanSDK\1.4.357.0` gesetzt werden.
Der serielle Native-Build von `transcribe-cpp-sys` lief damit reproduzierbar
durch — der Fix ist nicht an eine bestimmte Shell gebunden.

### 2.1 Prettier-Formatfehler (Kommando 2)

`prettier --check .` meldete:

```text
[warn] docs/meeting-mode-plan.md
[warn] src/components/ui/Textarea.tsx
[warn] src/i18n/locales/de/translation.json
[warn] src/i18n/locales/en/translation.json
Code style issues found in 4 files. Run Prettier with --write to fix.
```

### 2.2 Übersetzungsprüfung (Kommando 4)

Referenzsprache `en` mit 475 Keys; alle 23 Sprachen (AR, BG, CS, DA, DE, ES, FR,
HE, HI, IT, JA, KO, NE, NL, PL, PT, RU, SV, TR, UK, VI, ZH, ZH-TW) vollständig:

```text
✓ All 23 languages have complete translations!
```

### 2.3 Backend-Testlauf: `transcribe-cpp-sys`-Buildfehler — Ursache und Behebung

#### Ursprünglicher Fehler (Kommando 5, erste Messung)

`cargo test` brach vor dem Kompilieren der Crate ab — das Build-Skript von
`transcribe-cpp-sys v0.2.0` schlug fehl (cargo exit 101). Relevanter
aufgezeichneter Auszug:

```text
error: failed to run custom build command for `transcribe-cpp-sys v0.2.0`
  running: "cmake" ... "-G" "Visual Studio 17 2022" "-Thost=x64" "-Ax64"
           ... "-DCMAKE_BUILD_TYPE=Release"
  running: "cmake" "--build"
           "C:\\Users\\haeni\\AppData\\Local\\tcs\\32df4e9f173a51bb\\build"
           "--target" "install" "--config" "Release" "--parallel" "20"
  thread 'main' panicked at cmake-0.1.57\src\lib.rs:1132:5:
  command did not execute successfully, got: exit code: 1
  build script failed, must exit now
```

Das Fehlverhalten war in zwei aufeinanderfolgenden `cargo test`-Läufen
reproduzierbar. Eine gezielte Differentialmessung am selben CMake-Build-Verzeichnis
(`C:\Users\haeni\AppData\Local\tcs\32df4e9f173a51bb\build`):

| Aufruf                                                                                                  | Ergebnis                                                                                                      |
| ------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------- |
| `cmake --build <dir> --target install --config Release` (seriell)                                       | **erfolgreich**, Exit 0; installiert u. a. `transcribe.dll`, `ggml-vulkan.dll`, `transcribe-link.json`        |
| `cmake --build <dir> --target install --config Release --parallel 20` (wie vom Build-Skript aufgerufen) | **Fehler**, Exit 1; MSBuild bricht direkt nach `Checking File Globs` ab, ohne sichtbare Compilerfehlermeldung |

Eine zusätzliche MSBuild-Diagnose-Aufzeichnung (`msbuild.log`, `/v:diag /flp`,
`/m:2`) bestätigte zwei unabhängige Belastungsfaktoren auf dieser Maschine:

1. **MSBuild-Worker-Knoten (`/m`) schlagen lautlos fehl.** Die verschachtelten
   `GetNativeManifest`-Sub-Builds des ggml-Bäums enden mit Exit 1 und ohne
   Diagnoseausgabe, sobald Worker-Parallelität aktiv ist; seriell läuft
   derselbe Baum durch.
2. **MAX_PATH im FileTracker.** Der Windows-Registry-Schlüssel
   `LongPathsEnabled` ist zwar gesetzt, MSBuilds `GetOutOfDateItems`/FileTracker
   ignoriert ihn jedoch für die `.tlog`-`%(FullPath)`-Berechnung
   (FTK1011); die tiefen Pfade unter dem Cargo-`OUT_DIR`
   (Vulkan-`ExternalProject`-TryCompile-Scratchdirs) überschreiten die
   klassische 260-Zeichen-Grenze.

#### Umgesetzte Lösung: erzwungener serieller Native-Build

Da `NUM_JOBS` von Cargo für Build-Skripte immer überschrieben wird
(rust-lang/cargo#9972) und cmake-rs 0.1.x keinen Konfigurationsknopf dafür
besitzt, wurde cmake-rs 0.1.57 nach `vendor/cmake` kopiert und per
`[patch.crates-io]` in `src-tauri/Cargo.toml` ersetzt. Der lokale Patch:

1. Ehrt `CMAKE_BUILD_PARALLEL_LEVEL` vor `NUM_JOBS` und übergibt es als
   `--parallel <N>` an `cmake --build`.
2. Verlegt lange Build-Verzeichnisse (>120 Zeichen) unter Windows best-effort
   über ein kurzes NTFS-Junction im Temp-Verzeichnis (`%TEMP%\cmake-rs-short-build\<fnv-hash>`),
   um den FileTracker-MAX_PATH-Träger zu entschärfen.

`.cargo/config.toml` (Repo-Root) pinnt darüber hinaus:

```toml
[env]
CMAKE_BUILD_PARALLEL_LEVEL = "1"
```

Damit läuft der Native-Build von `transcribe-cpp-sys` deterministisch seriell;
das ist auf dieser Maschine verifiziert stabil. Erhöhung von `--parallel`
nur nach erneuter Verifikation der MSBuild-Parallelität.

#### Maschinenvoraussetzungen für den Native-Build

- **Vulkan SDK:** Ein frischer CMake-Konfigurierungslauf benötigt
  `VULKAN_SDK` bzw. `glslc` in der Umgebung. Auf dieser Maschine ist das SDK
  maschinenweit registriert (`C:\VulkanSDK\1.4.357.0`), aber eingeschränkte
  Shells (z. B. Sandbox-Harness) erben die Maschinen-Umgebung nicht — dort muss
  `$env:VULKAN_SDK = 'C:\VulkanSDK\1.4.357.0'` (+ `\Bin` im PATH) explizit
  gesetzt werden, sonst schlägt `FindVulkan` mit
  `Could NOT find Vulkan` fehl. Normale Nutzer-Shells erben die Variable.
- **Junction-Schreibrechte:** `transcribe-cpp-sys` legt sein eigenes
  Kurzpfad-Junction unter `%LOCALAPPDATA%\tcs\<hash>` an. In
  beschränkten Shells kann das an Berechtigungen scheitern ("Access is
  denied"); dann baut die Crate im tiefen `OUT_DIR` weiter und das
  cmake-rs-Junction aus dem lokalen Patch hält den Pfad kurz. Beide Pfade
  wurden verifiziert.

#### Ergebnis nach der Behebung

`cargo test`: **PASS**, Exit 0 — 208 Tests in `speak_app_lib` plus 12 Tests in
`meeting_loopback_spike`, 0 fehlgeschlagen (Log
`src-tauri/target/baseline-tools/logs/11-cargo-test-serial-fix.log`).
`cargo clippy --all-targets`: **PASS**, Exit 0 mit 0 Warnungen, nachdem sechs
Doc-Kommentar-Indentierungswarnungen in `src/bin/meeting_loopback_spike.rs`
behoben wurden (Logs `13-`/`14-cargo-clippy-final.log`). `cargo fmt --check`
weiterhin PASS. Damit steht die Rust-Baseline als Phase-0-Abnahme zur
Verfügung.

## 3. Diktier-Baseline — Reproduktionsschritte

Abgeleitet aus dem tatsächlich im Checkpoint vorhandenen Code
(`src-tauri/src/speech_pipeline.rs`, `src-tauri/src/settings/`,
`src/components/settings/general/SpeechPipelineSettings.tsx`). Der Provider wird
in den Einstellungen unter dem Speech-Pipeline-Abschnitt per
`speech_input_provider` gewählt; zulässige Werte sind `local`, `azure`, `custom`
(`src-tauri/src/settings/types.rs`, `SpeechInputProvider`).

Vorbereitung für alle Varianten:

1. App bauen und starten (`bun run tauri dev`; `bun` laut AGENTS.md erforderlich).
2. Mikrofon in den Audio-Einstellungen wählen; Diktier-Shortcut prüfen.
3. In `Einstellungen → Speech Pipeline` den jeweiligen Provider setzen.

### 3.1 Local

1. `speech_input_provider = local` wählen.
2. Lokales Whisper-/ONNX-Modell auswählen (Modellverwaltung; Modelle liegen unter
   `src-tauri/resources/models`, VAD-Modell `silero_vad_v4.onnx` erforderlich,
   siehe AGENTS.md).
3. Diktier-Shortcut drücken, kurz sprechen, Shortcut lösen/erneut drücken.
4. Erwartung: Transkript erscheint im Overlay und wird in die aktive Anwendung
   eingefügt; Eintrag landet im Diktatverlauf. Inferenz läuft vollständig lokal
   über `TranscriptionManager` (`speech_pipeline.rs`:
   „Local STT must be handled by TranscriptionManager“).

### 3.2 Azure Short Audio / Azure-Verbindung

Im Checkpoint läuft das Azure-Diktat als kontinuierliche Streaming-Session
(`AzureSpeechStreamManager`, Webview-SDK-Session); der Short-Audio-Endpunkt wird
für den Verbindungstest und als Endpunktauflösung verwendet
(`azure_short_audio_endpoint`).

1. `speech_input_provider = azure` wählen.
2. Azure-Endpoint konfigurieren: entweder Region-Kürzel (z. B. `germanywestcentral`)
   oder vollständige URL; eine Region wird zu
   `https://<region>.stt.speech.microsoft.com/speech/recognition/conversation/cognitiveservices/v1`
   aufgelöst (`speech_pipeline.rs`, `azure_short_audio_endpoint`).
3. Azure-API-Key hinterlegen (OS-Keychain, Scope `speech_pipeline`, Id `azure`).
4. Verbindungstest ausführen (`test_azure_speech_connection`): sendet eine kurze
   stille WAV-Datei an den Short-Audio-Endpunkt mit
   `?language=<azure_speech_language>&format=detailed`; erwartete Rückmeldung
   `Azure Speech connection verified` (Erkennungsergebnis ist erwartbar `NoMatch`,
   da das Testaudio still ist).
5. Diktat starten: Erwartung — Audio wird als fortlaufende Session an Azure
   gestreamt, Zwischen- und Endergebnisse erscheinen im Overlay, Finaltext wird
   eingefügt. Hinweis: Bei Azure wird `selected_output_device`-unabhängig
   aufgezeichnet; Cloud-Verarbeitung ist aktiv.

### 3.3 Custom STT

1. `speech_input_provider = custom` wählen.
2. `custom_stt_endpoint` (volle URL) in den Speech-Pipeline-Einstellungen setzen;
   optional Bearer-Token im Keychain-Scope `speech_pipeline`, Id `custom`.
3. Diktat starten: Erwartung — die App sendet die Aufnahme als
   `POST <endpoint>` mit `Content-Type: audio/wav`, Header
   `X-Speech-Language: <azure_speech_language>` und optional
   `Authorization: Bearer <token>` (`speech_pipeline.rs`, `transcribe_custom`).
4. Erwartung: Antwort (JSON oder Text) enthält das Transkript
   (`extract_transcript`), das bereinigt und eingefügt wird; Fehlerstatus
   (`Custom STT returned <status>`) wird sichtbar, wenn der Endpunkt nicht
   erfolgreich antwortet.

## 4. Manuelle Windows-Testmatrix (Plan Abschnitt 14)

Checkliste für Phase 1+ (Loopback-Spike, Dual-Recorder, Meeting-Modus). Alle
Punkte sind zu Beginn der Phase 1 **pending** — es liegt kein Testergebnis vor.
Spalten `Ergebnis` nur mit tatsächlich beobachtetem Ausfüllen: `pass`,
`fail` (mit Notiz) oder weiter `pending`.

| #   | Testfall                                                                   | Ergebnis | Pass | Fail | Notizen |
| --- | -------------------------------------------------------------------------- | -------- | ---- | ---- | ------- |
| M1  | Teams, WhatsApp Desktop, Browser-Meet, Zoom und Discord als Audioquelle    | pending  | ☐    | ☐    |         |
| M2  | Headset, Laptoplautsprecher und Bluetooth-Gerät als Output                 | pending  | ☐    | ☐    |         |
| M3  | Standardgerät sowie explizit ausgewähltes Gerät                            | pending  | ☐    | ☐    |         |
| M4  | 30 Sekunden Stille, lange Stille und wechselnde Gesprächsaktivität         | pending  | ☐    | ☐    |         |
| M5  | Mikrofon hört Lautsprecher mit                                             | pending  | ☐    | ☐    |         |
| M6  | Output-Gerät wird entfernt und neu verbunden                               | pending  | ☐    | ☐    |         |
| M7  | Standardgerät wird während der Aufnahme geändert                           | pending  | ☐    | ☐    |         |
| M8  | App wird hart beendet und neu gestartet                                    | pending  | ☐    | ☐    |         |
| M9  | Azure-Key ungültig, Endpoint falsch, Netzwerk offline und Netzwerk langsam | pending  | ☐    | ☐    |         |
| M10 | Lokales Modell langsamer als Echtzeit                                      | pending  | ☐    | ☐    |         |
| M11 | Diktat wird während lokaler Meetingverarbeitung ausgelöst                  | pending  | ☐    | ☐    |         |
| M12 | Datenträger nähert sich der Kapazitätsgrenze                               | pending  | ☐    | ☐    |         |
| M13 | App klein, maximiert und Vollbild; Fensterzustand nach Neustart            | pending  | ☐    | ☐    |         |
| M14 | Projekt erstellen, archivieren und Quelle neu zuordnen                     | pending  | ☐    | ☐    |         |
| M15 | `.txt`- sowie `.zip`-Export mit/ohne Medien und überlappendem Verlauf      | pending  | ☐    | ☐    |         |
| M16 | Beschädigtes, verschlüsseltes und bösartig konstruiertes ZIP               | pending  | ☐    | ☐    |         |
| M17 | OpenRouter-Modell mit und ohne passende Bildeingabe-Fähigkeit              | pending  | ☐    | ☐    |         |
| M18 | Cloudfreigabe abbrechen und denselben Import vollständig lokal fortsetzen  | pending  | ☐    | ☐    |         |
| M19 | KI-Aufgabe bearbeiten/erledigen und danach Projekt erneut analysieren      | pending  | ☐    | ☐    |         |
| M20 | Suche springt zur richtigen Chatnachricht oder Meeting-Zeitposition        | pending  | ☐    | ☐    |         |

Hinweis: M13–M20 betreffen Phasen ab Phase 3 (App-Shell, Projekte, Import,
KI-Aufgaben) und bleiben solange pending, bis die jeweilige Phase implementiert
ist.

## 5. Offene Punkte aus der Baseline

1. ~~14 uncommittete Dateien auf `feat/custom-stt-pipeline` sichern (Checkpoint
   erweitern oder Commit erstellen).~~ **erledigt am 22.08.2026:** gesichert
   durch die Commits `b677107`, `a7226b6`, `8fd9dee`, `beb286e` auf
   `feat/custom-stt-pipeline` (Bereich
   `checkpoint/custom-stt-pre-meeting..HEAD`), siehe Abschnitt 1.
2. Prettier-Formatfehler in 4 Dateien beheben (`bun run format` bzw.
   `prettier --write` für die in Abschnitt 2.1 genannten Dateien) — **offen**,
   Owner: task-0010.
3. ~~`transcribe-cpp-sys`-Buildfehler unter `--parallel 20` klären~~ **erledigt
   am 22.08.2026:** erzwungener serieller Native-Build über den
   `cmake`-Patch (`vendor/cmake`) und `.cargo/config.toml`
   (`CMAKE_BUILD_PARALLEL_LEVEL=1`); `cargo test` und
   `cargo clippy --all-targets` laufen, siehe Abschnitt 2.3.
4. Diktier-Baseline manuell nach Abschnitt 3 auf der Zielhardware ausführen und
   Ergebnisse ergänzen (in dieser Baseline nicht ausgeführt) — **offen**,
   Owner: task-0009.
