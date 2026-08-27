# Platform Notes: Betriebs-Knöpfe pro Plattform (Env-Variablen, Acceleration-Backends, Wayland, Autostart)

Status: **implemented behavior** (jede Variablen- und Verhaltensaussage ist mit
Datei-/Zeilenbeleg gegen den unten gepinnten Codestand verifiziert; jede
dokumentierte, vom Handy-Code gelesene Variable wurde per grep im Code
nachgewiesen — siehe Belege je Abschnitt).
Autor-Arbeitsstand: **HEAD-Commit `33c6e0fd7effef94daf13e6277ee4ca7e6d00e96`**
(verifiziert am 22.08.2026 via `git rev-parse HEAD` im sauberen Arbeitbaum;
`git status --porcelain` war bei der Verifikation leer).
Umfang: Operator-Nachschlagewerk der plattformabhängigen Betriebsknöpfe —
Laufzeit-Umgebungsvariablen, Build-Zeit-Variablen, Acceleration-Backend-
Auswahl (Vulkan/Metal/OpenBLAS), Wayland-Einschränkungen und
Autostart-Integration. Es ersetzt kein Troubleshooting-Dokument, sondern
verlinkt dorthin; nichts hier schlägt den Code.
Plan: [docs/meeting-mode-plan.md](../meeting-mode-plan.md)
Index: [docs/meeting-mode-documentation-index.md](../meeting-mode-documentation-index.md)
Aufgenommen am: 22.08.2026

---

## 1. Plattform-Matrix (G2)

Anwendungsbereich jedes Knopfes auf einen Blick. „—“ = ohne Wirkung
(nicht gelesen bzw. nicht kompiliert); Details und Belege in den genannten
Abschnitten.

| Knopf                                                                                          | Art                                                               | Windows                          | macOS                    | Linux X11                     | Linux Wayland                       |
| ---------------------------------------------------------------------------------------------- | ----------------------------------------------------------------- | -------------------------------- | ------------------------ | ----------------------------- | ----------------------------------- |
| `HANDY_NO_GTK_LAYER_SHELL`                                                                     | Laufzeit                                                          | —                                | —                        | wirkt¹                        | wirkt¹                              |
| `HANDY_METAL_RESIDENCY` / `GGML_METAL_NO_RESIDENCY`                                            | Laufzeit (Weitergabe an ggml)                                     | —²                               | wirkt²                   | —                             | —                                   |
| `RUST_LOG`                                                                                     | Laufzeit                                                          | wirkt                            | wirkt                    | wirkt                         | wirkt                               |
| `HANDY_FORCE_TRANSCRIPTION_FAILURE`                                                            | Laufzeit, nur Debug-Builds                                        | wirkt³                           | wirkt³                   | wirkt³                        | wirkt³                              |
| `HANDY_DEBUG_MIC_READY_DELAY_MS`                                                               | Laufzeit, nur Debug-Builds                                        | wirkt³                           | wirkt³                   | wirkt³                        | wirkt³                              |
| `ORT_LIB_LOCATION` + `ORT_PREFER_DYNAMIC_LINK`                                                 | Build-Zeit                                                        | wirkt⁴                           | wirkt⁵                   | —                             | —                                   |
| `HANDY_VC_REDIST_DIRS`                                                                         | Build-Zeit                                                        | wirkt⁴                           | —                        | —                             | —                                   |
| `HANDY_FORCE_AI_STUB`                                                                          | Build-Zeit                                                        | —                                | wirkt⁶                   | —                             | —                                   |
| Whisper-GPU-Backend (Feature-Flag, nicht zur Laufzeit wählbar)                                 | Build fest verdrahtet                                             | Vulkan (x86_64) / CPU-only (ARM) | Metal                    | Vulkan + CPU-Dynamik-Backends | Vulkan + CPU-Dynamik-Backends       |
| Wayland-Erkennung (`WAYLAND_DISPLAY`, `XDG_SESSION_TYPE`, KDE-Variablen)                       | Laufzeit, nur Auswertung                                          | —                                | —                        | —                             | wirkt                               |
| Paste-/Clipboard-Hilfswerkzeuge (`wtype`, `kwtype`, `dotool`, `ydotool`, `xdotool`, `wl-copy`) | Laufzeit, extern binär                                            | —                                | —                        | wirkt                         | wirkt                               |
| `WEBKIT_DISABLE_DMABUF_RENDERER`                                                               | Laufzeit, von WebKitGTK gelesen; von Handy auf Linux hart gesetzt | —                                | —                        | —                             | von Handy gesetzt (`main.rs:10-15`) |
| Autostart-Einstellung (`autostart_enabled`)                                                    | Laufzeit/Einstellung                                              | Login-Eintrag via Plugin         | SMAppService (macOS 13+) | Login-Eintrag via Plugin      | Login-Eintrag via Plugin            |

¹ Der Code-Pfad existiert nur in Linux-Builds (`#[cfg(target_os = "linux")]`,
`src-tauri/src/overlay.rs:119,132`); unter X11 initialisiert gtk-layer-shell
typischerweise nicht (`gtk_layer_shell::is_supported()`, `overlay.rs:139`),
sodass der Fallback ohnehin greift — die Variable erzwingt ihn zusätzlich.
² `lib.rs:182` gate: `#[cfg(target_os = "macos")]`. Die Variable selbst wird
auf anderen Plattformen nie gelesen.
³ `cfg(debug_assertions)`-gated (`transcription.rs:1169`, `actions.rs:615`);
Release-Builds ignorieren beide Variablen vollständig.
⁴ Nur Windows-Target im Build-Skript (`build.rs:57,122`); sonst No-op.
⁵ `build.rs` stagt nur für Windows-Targets; der macOS-dev-Einsatz in
BUILD.md:26/32 geht an die ORT-Bindings des Builds, nicht an `build.rs`.
⁶ Nur im macOS-Zweig des Build-Skripts (`build.rs:420-444`).

## 2. Laufzeit-Variablen, die der Handy-Code liest

### 2.1 `HANDY_NO_GTK_LAYER_SHELL` (nur Linux)

- Gelesen in `init_gtk_layer_shell()`:
  `src-tauri/src/overlay.rs:134-137` — wenn gesetzt (truthy), wird die
  GTK-layer-shell-Initialisierung übersprungen (Log: „Skipping GTK layer
  shell init“).
- Wahrheitssemantik (`env_flag_enabled`, `overlay.rs:115-128`): `1`, `true`,
  `yes`, `on` (case-insensitive, getrimmt) gelten als wahr; `0`, `false`,
  `no`, `off` und Leerstring als falsch; nicht gesetzt = falsch. Jeder andere
  nicht-leere Wert gilt als wahr.
- Wirkung bei Aktivierung: das Aufnahme-Overlay fällt auf ein normales
  Always-on-top-Fenster zurück (`overlay.rs:425-433` beim Fensterbau,
  `overlay.rs:507-524` beim Anzeigen; `LAYER_SHELL_ACTIVE` bleibt false,
  der reguläre Größen-/Positionier-Pfad läuft). Positionierung folgt dann
  der normalen Overlay-Platzierung statt dem Layer-Shell-Anker.
- Operator-Anwendungsfall und Permanenz (Shell-Profil oder
  `.desktop`-`Exec=env HANDY_NO_GTK_LAYER_SHELL=1 handy`): README,
  Abschnitt „Linux Startup Crashes or Instability“, Schritt 2
  (`README.md:460-482`).

### 2.2 `HANDY_METAL_RESIDENCY` (nur macOS)

- Gelesen in `run()`: `src-tauri/src/lib.rs:176-189`, gated mit
  `#[cfg(target_os = "macos")]`, vor jeder transcribe-cpp/Metal-Initialisierung.
- Default (Variable nicht gesetzt oder ≠ `1`): Handy setzt
  `GGML_METAL_NO_RESIDENCY=1` in der Prozess-Umgebung, um ggml-metal
  Residency-Set-Teardown-Assertions beim Herunterfahren zu vermeiden
  (`lib.rs:178-188`, Issue #1902 im Kommentar).
- `HANDY_METAL_RESIDENCY=1` (exakt `1`): entfernt ein geerbtes
  `GGML_METAL_NO_RESIDENCY` und stellt damit das ggml-upstream-
  Residency-Verhalten wieder her (`lib.rs:183-186`).
- Hinweis zur Weiterreichung: `GGML_METAL_NO_RESIDENCY` selbst liest nicht
  der Handy-Code, sondern ggml innerhalb von transcribe-cpp (Präsenz-basiert,
  laut Kommentar `lib.rs:184-185`). Für Operatoren ist nur
  `HANDY_METAL_RESIDENCY` die unterstützte Einstiegsvariable.

### 2.3 `RUST_LOG` (alle Plattformen)

- Gelesen in `build_console_filter()`: `src-tauri/src/logging.rs:38-58`;
  Aufrufstelle beim Start: `src-tauri/src/lib.rs:199-201`.
- Semantik: gültige `env_filter`-Direktiven steuern das Konsolen-Log;
  unset/leer → Info-Level; ungültiger Wert → Warnung im Log und Fallback
  auf Info-Level (`logging.rs:41-55`). Das Datei-Log ist davon unberührt
  (separates Level über Debug-Modus/Settings, `logging.rs:60-68`).

### 2.4 Debug-only Variablen (wirken ausschließlich in Debug-Builds)

Beide sind mit `#[cfg(debug_assertions)]` kompiliert und in
Release-Paketen strukturell nicht vorhanden:

- `HANDY_FORCE_TRANSCRIPTION_FAILURE` — Präsenz genügt (Wert egal):
  `transcribe()` liefert sofort einen simulierten Fehler
  (`src-tauri/src/managers/transcription.rs:1168-1174`). Nützlich zum
  Testen von Fehlerpfaden/UX.
- `HANDY_DEBUG_MIC_READY_DELAY_MS` — ganzzahlige Millisekunden; verzögert
  den Mikrofon-Bereitschafts-Cue (Preview der Arming-Animation), hart
  gedeckelt auf 10 000 ms, Werte ≤ 0 und Nicht-Zahlen werden ignoriert
  (`src-tauri/src/actions.rs:612-625`).

## 3. Build-Zeit-Variablen

Diese Variablen wirken beim Kompilieren/Paketieren, nicht zur Laufzeit.
Build-Grundanliegen stehen in [BUILD.md](../../BUILD.md); hier nur die
vom Code gelesenen Knöpfe.

### 3.1 `ORT_LIB_LOCATION` + `ORT_PREFER_DYNAMIC_LINK`

- `build.rs` reagiert nur, wenn **beide** gesetzt sind und das Target
  Windows ist: dann wird `onnxruntime.dll` aus `ORT_LIB_LOCATION` nach
  `transcribe-libs/` gestagt und neben `Handy.exe` gebündelt
  (`src-tauri/build.rs:105-147`; Checks `119-127`, Panic bei fehlender DLL
  `130-136`). Ohne die Variablen bleibt es beim statisch eingebetteten ORT.
- Hintergrund: der dynamische ORT ist eine Baseline-Build für Pre-Haswell-
  CPUs, die pyke's AVX2-Embedded-Build abstürzen lässt
  (`build.rs:28-32`, `Cargo.toml:114-120`).
- macOS-Entwicklung nutzt dieselben zwei Variablen für die ORT-Bindings
  (nicht für `build.rs`-Staging): `BUILD.md:26` (dev) und `BUILD.md:32`
  (build), typischerweise `$(brew --prefix onnxruntime)/lib`.

### 3.2 `HANDY_VC_REDIST_DIRS` (Windows-Build)

- Gelesen in `stage_vc_runtime_dlls()`: `src-tauri/build.rs:40-103`. Setzt
  CI auf die Redist-Verzeichnisse desselben Visual-Studio-Installs; kopiert
  `msvcp140*`/`vcruntime140*`/`vcomp140*` nach `transcribe-libs/`
  (`77-85`). Fehlt `msvcp140.dll` oder `vcruntime140.dll`, failt der Build
  bewusst hart (`89-98`, Issue #1527). Ungesetzt = No-op.

### 3.3 `HANDY_FORCE_AI_STUB` (macOS-Build)

- Gelesen in `build.rs:424` (exakt `1`); erzwingt den Apple-Intelligence-
  Stub auch dann, wenn FoundationModels gefunden wurde — Escape Hatch für
  CLT-only-Toolchains oder Fehl-Detektion (`build.rs:417-444` inkl.
  Warnungen `436-442`).

### 3.4 Verwandte, aber nicht betriebsrelevant

- `VULKAN_SDK`/`glslc`: Build-Voraussetzung des Vulkan-Backends, gelesen
  von CMake, **nicht** vom Handy-Code (`BUILD.md:45-50`, `428-436`).
- `DEP_TRANSCRIBE_CPP_RUNTIME_DIR`/`DEP_TRANSCRIBE_CPP_MODULE_DIR`:
  interne Cargo-`links`-Metadaten zwischen Sys-Crate und `build.rs`
  (`build.rs:149-190`) — keine Operator-Knöpfe.
- `XDG_DATA_HOME`/`HOME` (Linux): wählt den Fallback-Pfad für den
  Secret-Store, wenn kein Portable-Datenverzeichnis gesetzt ist
  (`src-tauri/src/secret_store.rs:117-129`); Standard-Systemvariablen,
  hier nur der Vollständigkeit halber.

## 4. Acceleration-Backends (zur Build-Zeit fest verdrahtet)

Es gibt **keine Laufzeit-Auswahl** des GPU-Backends; die Auswahl geschieht
über Cargo-Target-Features (`src-tauri/Cargo.toml`):

| Target              | transcribe-cpp-Features                                                                               | Beleg                                      |
| ------------------- | ----------------------------------------------------------------------------------------------------- | ------------------------------------------ |
| Windows x86_64      | `dynamic-backends` + `vulkan` (per-ISA-CPU-Module + `ggml-vulkan.dll` werden geladen)                 | `Cargo.toml:150-154`                       |
| Windows ARM64       | statisch, CPU-only (keine Vulkan-Features; Adreno-Treiber + shader-generator-Begründung im Kommentar) | `Cargo.toml:156-157`, Begründung `141-149` |
| macOS (beide Archs) | `metal`, statisch                                                                                     | `Cargo.toml:159-161`                       |
| Linux               | `dynamic-backends` + `vulkan`                                                                         | `Cargo.toml:169-176`                       |

Operator-relevante Konsequenzen:

- **OpenBLAS (Linux)** ist eine Paket-Abhängigkeit, keine Env-Variable:
  deb hängt an `libopenblas0`, rpm an `libopenblas.so.0()(64bit)`
  (`src-tauri/tauri.conf.json:48,56`). Fehlt sie, schlägt die
  Paketinstallation fehl — nicht die Laufzeit. Die dlopen'd ggml-CPU/Vulkan-
  Module landen app-privat unter `/usr/lib/Speak` bzw. im AppImage
  (`tauri.conf.json:49-51,61-63,66-69`; Staging `build.rs:22-26,149-190`).
- **Metal (macOS)** ist statisch gelinkt; das Residency-Verhalten wird
  über `HANDY_METAL_RESIDENCY` (Abschnitt 2.2) beeinflusst, nicht durch
  Backend-Wahl.
- **ONNX-Seite (Parakeet/Moonshine/SenseVoice/u. a.) ist überall CPU-only**
  — kein DirectML/CoreML/CUDA-EP (`Cargo.toml:114-120`). GPU-Beschleunigung
  betrifft nur die Whisper-Familie über transcribe-cpp.
- Vulkan-Module müssen neben der Binärdatei liegen, sonst registriert
  `init_backends_default()` keine Compute-Backends (`build.rs:177-183`).
  Diagnose bei „nutzt nur CPU“: zuerst prüfen, ob `ggml-vulkan.*` im
  App-Verzeichnis bzw. `/usr/lib/Speak` vorhanden ist.

## 5. Wayland-Einschränkungen (Linux)

- Erkennung: `is_wayland()` wertet `WAYLAND_DISPLAY` bzw.
  `XDG_SESSION_TYPE=wayland` aus; `is_kde_wayland()` zusätzlich
  `XDG_CURRENT_DESKTOP`/`KDE_SESSION_VERSION`
  (`src-tauri/src/utils.rs:141-163`).
- Texteingabe-/Paste-Wege wählen externe Hilfswerkzeuge aus:
  Wayland bevorzugt `wtype` (außer KDE), dann `dotool`, dann `ydotool`;
  KDE-Wayland bevorzugt `kwtype`; X11 bevorzugt `xdotool`, dann `ydotool`
  (`src-tauri/src/clipboard.rs:120-157` für Key-Kombos, `159-224` für
  Direkteingabe). Clipboard-Schreiben (z. B. Transcript-Ausgabe) nutzt
  `wl-copy` unter Wayland (`src-tauri/src/clipboard.rs:31-36`). Fehlt ein
  passendes Werkzeug, fällt Handy auf enigo zurück — unter Wayland kann das
  je nach Compositor fehlschlagen; das Nachinstallieren des Werkzeugs ist
  der Operator-Hebel.
- Overlay: gtk-layer-shell wird genutzt, wo unterstützt; Störungen behebt
  `HANDY_NO_GTK_LAYER_SHELL` (Abschnitt 2.1). Fehlende/beschädigte
  `libgtk-layer-shell0` ist die häufigste Startcrash-Ursache
  (`README.md:444-458`).
- WebKitGTK-Renderprobleme: `WEBKIT_DISABLE_DMABUF_RENDERER` wird vom
  **WebKitGTK gelesen**, aber von Handy selbst gesetzt: `main.rs` setzt es
  auf Linux bedingungslos auf `1`, bevor die WebView entsteht
  (`src-tauri/src/main.rs:10-15`, Hintergrund tauri#9394 im Kommentar) und
  überschreibt damit einen extern gesetzten Wert. Der README-Workaround
  „Variable von Hand setzen“ (`README.md:468-474`) ist am aktuellen Code
  damit obsolet bzw. wirkungslos; bleibt eine Renderkrisen-Verdachtsfläche,
  ist der Hebel ein Issue mit GPU/Compositor-Angaben, nicht diese Variable.
- Ein temporärer tao-Pin trägt den Wayland-Client-Side-Decorations-Fix und
  einen Windows-Shutdown-Fix (`Cargo.toml:178-196`) — für Operatoren nur
  relevant als Erklärung, warum ein git-gepinnter tao im Lockfile steht.

## 6. Autostart-Integration

- Einstellung: `autostart_enabled` (persistiert,
  `src-tauri/src/settings/schema.rs:41-42`, Default `schema.rs:213`);
  UI-/Command-Pfad `change_autostart_setting`
  (`src-tauri/src/shortcut/mod.rs:849-859`).
- Anwendung: bei jedem Start erneut angewendet (`lib.rs:168-170`), sodass
  transienter Fehler sich selbst heilt; Fehler werden geloggt und blockieren
  den Start nicht (`src-tauri/src/autostart.rs:14-42`).
- Mechanismen pro Plattform (`autostart.rs:1-42`):
  - **macOS 13+**: `SMAppService`-Login-Item (App-attributiert, erscheint
    unter „Open at Login“ mit App-Icon); Legacy-Launch-Agent-Plist des
    Plugins wird beim Start entfernt, um Doppelstarts zu vermeiden
    (`autostart.rs:44-111`, Issue #337 im Modulkommentar). In Dev-Builds
    (unsigned bundle) schlägt die Registrierung bewusst fehl und wird nur
    geloggt (`autostart.rs:73-77`).
  - **Windows/Linux**: tauri-plugin-autostart (Registry-Run-Key bzw.
    Desktop-Autostart-Eintrag via auto-launch crate).
- Interaktion mit Env-Variablen: ein Workaround wie
  `HANDY_NO_GTK_LAYER_SHELL` muss in denselben Autostart-Eintrag
  (`Exec=env …` bzw. Shell-Profil), sonst gilt er beim Login-Start nicht
  (`README.md:476-482`). Für Kopplung mit „minimiert starten“ existiert
  der CLI-Parameter `--start-hidden` (siehe
  [troubleshooting-shortcuts-cli](troubleshooting-shortcuts-cli.md) bzw.
  AGENTS-CLI-Tabelle).

## 7. Non-Goals / Komplexitätsbudget

- Dieses Dokument führt **keine neuen Knöpfe ein** und dokumentiert keine
  undokumentierten Variablen; alles hier ist gegen Code belegt.
- Keine Runtime-Umschaltung der GPU-Backends (Abschnitt 4) — solche Wünsche
  sind ein Build-Feature-Change, kein Operator-Knopf.
- Drittanbieter-gelesene Variablen (`WEBKIT_DISABLE_DMABUF_RENDERER` →
  WebKitGTK, `VULKAN_SDK`/`glslc` → CMake) sind explizit als solche
  markiert und nicht als Handy-API dargestellt; bei ersterer ist zusätzlich
  dokumentiert, dass Handy sie auf Linux selbst setzt (Abschnitt 5).
