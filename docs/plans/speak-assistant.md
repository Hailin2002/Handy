# Plan: Speak Assistant (Audio-Ducking + zwei KI-Assistenten)

Branch: `feat/speak-assistant`

Dieses Dokument ist der verbindliche Implementierungsvertrag. Jede Phase ist
eigenständig verifizierbar. Zeilennummern beziehen sich auf den Stand bei
Branch-Erstellung.

## Architektur-Entscheidungen (vorab getroffen, mit Begründung)

**E1 — Ergebnis-Karte lebt im bestehenden Overlay-Fenster, nicht in einem
dritten Fenster.** Das Overlay (`recording_overlay`) ist bereits ein eigener
WebviewWindow mit Zustandsmaschine, Positionierung pro Monitor, Layer-Shell auf
Linux, NSPanel auf macOS und Topmost-Reassert auf Windows
(`src-tauri/src/overlay.rs:383-583`). Ein neues Fenster müsste all das
duplizieren und zusätzlich in `capabilities/default.json:5` und beiden
Vite-Configs registriert werden. Die Anforderung lautet "kleine Karte direkt
beim Speak-Overlay" — ein neuer Overlay-State ist die passende Umsetzung.

**E2 — Assistenten laufen über den TranscriptionCoordinator, nicht daneben.**
`AudioRecordingManager` (`managers/audio.rs:820-866`) und
`AzureSpeechStreamManager` (`speech_pipeline.rs:211-265`) sind harte Singletons;
`start()` ruft intern `cancel()` auf und würde eine laufende Diktier-Session
töten. Die neuen Bindings werden deshalb in
`transcription_coordinator.rs:78-80` als Pipeline-Bindings registriert, damit
`Stage` sie serialisiert. Das erfüllt "Die normale Diktierfunktion bleibt davon
unabhängig" im Sinne von: sie kollidieren nie.

**E3 — `mute_while_recording` bleibt erhalten und wird write-through
gespiegelt.** Neues Feld `recording_audio_mode` ist die Wahrheit für die
Laufzeit; der alte bool wird bei jeder Änderung auf `mode == Mute`
mitgeschrieben, damit bestehende UI-Tests, Fixtures und der
frozen-store-Test (`settings/tests.rs:83-201`) intakt bleiben.

**E4 — Eigener Secret-Scope pro Assistent.** `ASSISTANT_SCOPE = "assistant"`,
id = Profil-id. Nicht der Provider-id-Scope von Post-Processing, weil beide
Assistenten denselben Provider mit verschiedenen Keys nutzen können sollen.

---

## Phase A — Audio-Ducking

Ziel: Modi Aus / Leiser / Stumm, einstellbare Ducking-Lautstärke, Fade,
exakte Wiederherstellung.

### A1 — Plattform-Lautstärkeschicht

Neue Datei `src-tauri/src/audio_ducking.rs`, Modul in `lib.rs` registrieren.

```rust
pub fn get_output_volume() -> Option<f32>;   // 0.0..=1.0
pub fn set_output_volume(v: f32);            // clamped
```

- Windows: `IAudioEndpointVolume::GetMasterVolumeLevelScalar` /
  `SetMasterVolumeLevelScalar`. COM-Boilerplate 1:1 aus
  `managers/audio.rs:33-67` übernehmen. **Keine neuen Cargo-Features nötig** —
  `Win32_Media_Audio_Endpoints` ist bereits aktiv.
- macOS: `osascript -e "output volume of (get volume settings)"` (0..100) bzw.
  `set volume output volume N`.
- Linux: `wpctl get-volume @DEFAULT_AUDIO_SINK@` → `pactl get-sink-volume` →
  `amixer get Master`; Setzen analog zur Kette in `managers/audio.rs:69-101`.
  `LC_ALL=C` erzwingen wie bei `get_mute` (`:147-199`).
- Fehler werden geschluckt und als `None` gemeldet (Muster `unwrap_or_return!`
  aus `managers/audio.rs:44-51`).

### A2 — Settings

`src-tauri/src/settings/types.rs`: neues Enum

```rust
pub enum RecordingAudioMode { Off, Duck, Mute }
```

`src-tauri/src/settings/schema.rs` neben `mute_while_recording:186-187`:

- `recording_audio_mode: RecordingAudioMode` (default `Off`)
- `duck_volume_percent: u8` (default 20, clamp 0..=100)
- `duck_fade_ms: u16` (default 120, clamp 0..=1000)

Default-Fns im selben Modul (Konvention `schema.rs:371-373`), Einträge in
`get_default_settings()` (`schema.rs:794` Nachbarschaft).
Migration in `settings/store.rs:176-230`, Muster "Key fehlt":
`recording_audio_mode` absent → `Mute` falls `mute_while_recording == true`,
sonst `Off`.

### A3 — Lifecycle

`managers/audio.rs`: `MuteState` (`:256-260`) erweitern zu

```rust
struct DuckState { engaged: bool, prev_muted: Option<bool>, prev_volume: Option<f32> }
```

- `apply_mute()` (`:581-602`) → `apply_ducking()`: liest `recording_audio_mode`,
  `Off` → return. `Mute` → altes Verhalten. `Duck` → Volume snapshotten und in
  `duck_fade_ms` linear in ~10 Schritten auf `duck_volume_percent` fahren.
- `remove_mute()` (`:606-616`) → `remove_ducking()`: hochfaden auf
  `prev_volume`, dann exakt `prev_volume` setzen (kein Rundungsdrift), Mute
  restaurieren.
- Die drei Safety-Net-Stellen `:667-673`, `:690-696`, `:759-765` mitziehen.
- **Bug mit beheben:** `cancel_recording()` (`:1020-1050`) stellt heute nicht
  wieder her, wenn der Stream offen bleibt (always-on oder `lazy_stream_close`).
  Beim Ducking wäre das eine dauerhaft leise Anlage. `remove_ducking()` dort
  explizit aufrufen. Gleiches in `stop_recording()` (`:933-1009`) absichern.
- **Meeting-Interaktion:** Meeting-Aufnahme captured System-Loopback
  (`audio_toolkit/audio/meeting_recorder.rs:2021`). Ducking muss übersprungen
  werden, solange eine Meeting-Aufnahme läuft, sonst wird der Mitschnitt leise.
- Fade läuft in einem eigenen `std::thread` (blockierend, plattformgebunden —
  siehe Concurrency-Regel in AGENTS.md); ein Generations-Zähler bricht ein
  laufendes Fade ab, wenn das Gegenstück startet, damit Down/Up nie kreuzen.

### A4 — Frontend

`src/components/settings/general/MuteWhileRecording.tsx` ersetzen durch
`RecordingAudioSettings.tsx`: 3-Wege-Select + Slider (nur bei `Duck` sichtbar).
Mount-Stelle `general/GeneralSettings.tsx:13,36`. Store-Updater
`src/stores/settingsStore.ts:201-202` erweitern. Neue Commands nach dem Muster
`change_audio_feedback_volume_setting` (`shortcut/mod.rs:565-568`), registriert
in `lib.rs:410`-Nachbarschaft. `bun run bindings:generate`.
i18n: alle 24 Locales, Keys unter `settings.debug.muteWhileRecording` ersetzen
bzw. `settings.general.recordingAudio.*` neu anlegen.

### A5 — Tests

Rust: Migrationstest (bool→mode), Clamp-Tests, DuckState-Idempotenz.
TS: Render-Test für den neuen Settings-Block. Fixtures
`src/mocks/fixtures.ts:72` und `src/mocks/settingCommands.ts:193-196` anpassen.

---

## Phase B — llm_client: max_tokens + Timeout

`src-tauri/src/llm_client.rs` kennt heute weder `max_tokens` noch ein Timeout
(`create_client:176-182`, `ChatCompletionRequest:112-121`). Beides ist für die
Assistenten-Profile gefordert.

- Neuer Options-Struct `ChatOptions { disable_reasoning: bool,
max_tokens: Option<u32>, timeout: Option<Duration> }` mit `Default`.
- `ChatCompletionRequest` bekommt `max_tokens: Option<u32>`
  (`skip_serializing_if = "Option::is_none"`).
- `create_client` bekommt einen Timeout-Parameter.
- Neue Public-Fn `send_chat_completion_opts(provider, api_key, model,
user_content, system_prompt, json_schema, opts)`. Die beiden bestehenden
  Funktionen (`:300`, `:329`) bleiben als dünne Wrapper erhalten — **keine
  Änderung an den Aufrufern** in `actions.rs:265,323` und
  `managers/meeting_summary.rs:294`.
- Timeout-Fehler klar unterscheidbar zurückgeben, damit die Karte
  "Zeitüberschreitung" anzeigen kann.

---

## Phase C — Assistenten-Profile in Settings & Secrets

`settings/types.rs`:

```rust
pub struct AssistantProfile {
    pub id: String, pub provider_id: String, pub model: String,
    pub system_prompt: String, pub max_output_tokens: u32,
    pub timeout_secs: u32, pub output_mode: AssistantOutputMode,
}
pub enum AssistantOutputMode { Show, Copy, Paste }
```

`schema.rs`: `assistant_profiles: HashMap<String, AssistantProfile>` mit
`#[serde(default = "default_assistant_profiles")]`; Defaults `fast_helper`
(kleines Modell, `disable_reasoning`, 512 Tokens, 15 s) und `smart_assistant`
(starkes Modell, 2048 Tokens, 90 s). Reparatur/Seeding in
`ensure_post_process_defaults` (`schema.rs:576-647`), damit Bestandsnutzer die
Profile bekommen. Accessor `assistant_profile(&str)`.

`secret_store.rs`: `ASSISTANT_SCOPE = "assistant"` neben `:26-28`;
`hydrate_secret_maps` (`:276-306`) um die Profile erweitern; neues
`assistant_api_keys: SecretMap` im Schema.

Zwei Bindings in `get_default_settings()` (`schema.rs:648-735`), **unbound
ausgeliefert** (`current_binding: String::new()`, Muster `meeting_pause_resume`
`:726-735`): `assistant_fast`, `assistant_smart`. Vorschlags-Defaults
`ctrl+alt+space` / `ctrl+alt+shift+space`.

Commands in `shortcut/mod.rs` nach Muster `:1166-1210`, alle mit
`validate_profile_exists`: `set_assistant_provider`,
`change_assistant_api_key_setting` (leer ⇒ `delete_secret`),
`change_assistant_model_setting`, `change_assistant_prompt_setting`,
`change_assistant_limits_setting`, `set_assistant_output_mode`.
Registrierung in `lib.rs:405-593`. Bindings regenerieren.

---

## Phase D — Ausführungs-Flow

`actions.rs`: gemeinsame Teile von `TranscribeAction::start` (`:507-695`) in eine
Hilfsfunktion ziehen, damit `AssistantAction { profile_id }` denselben
Aufnahme-Start nutzt (Overlay, Azure-Start, `try_start_recording`,
Cancel-Shortcut).

`AssistantAction::stop`: Transkript exakt wie `TranscribeAction::stop`
(`:790-883`) beschaffen, dann am Fork-Punkt `:916` abbiegen:

- **kein** `hm.save_entry` (`:957`) — Assistenten-Anfragen verschmutzen die
  History nicht
- **kein** `utils::paste` (`:984`) außer bei `output_mode == Paste`
- Overlay-State `asking`, dann LLM-Call über `send_chat_completion_opts` mit den
  Profilwerten und dem Systemprompt
- Ergebnis als typisiertes Event `AssistantAnswerEvent` emittieren
- Fehlerfälle (kein Key, Timeout, HTTP-Fehler, leeres Transkript) als
  `AssistantAnswerEvent` mit `error`-Feld, nicht als stiller Log

Registrierung in `ACTION_MAP` (`:1090-1111`) und in
`transcription_coordinator.rs:78-80`. Prüfen: `shortcut/mod.rs:276,466`,
`tauri_impl.rs:27`, `handy_keys.rs:437`, `secure_input.rs:534` — überall dort,
wo Post-Processing bedingt übersprungen wird, entscheiden ob die Assistenten
analog gated werden (Vorgabe: nein, sie sind immer registriert, aber unbound).

Event-Vertrag (verbindlich für Phase E):

```rust
pub struct AssistantAnswerEvent {
    pub profile_id: String,      // "fast_helper" | "smart_assistant"
    pub question: String,        // das Transkript
    pub answer: Option<String>,
    pub error: Option<String>,
    pub links: Vec<String>,      // aus der Antwort extrahierte URLs
    pub elapsed_ms: u32,
}
```

Registrierung in `collect_events!` (`lib.rs:595-599`).

---

## Phase E — Ergebnis-Karte

Neuer Overlay-State `answer` in `overlay.rs`: Dimension z. B. 420x300
(`overlay_dimensions:54-60`), CSS-Vars in `RecordingOverlay.css:53-62`
synchron halten (dort steht die Single-Source-of-Truth-Notiz).
`show_answer_overlay()` neben `:632`. Auto-Hide unterdrücken solange die Karte
offen ist.

`src/overlay/RecordingOverlay.tsx`: Listener auf `events.assistantAnswerEvent`
(`:123`-Muster), Karte mit Antworttext (scrollbar), Buttons Kopieren / Einfügen /
Ersetzen / Link öffnen / Schließen / "Mit Smart Assistant weiterfragen".
Styling in plain CSS mit den `--s-*`-Tokens — **kein Tailwind im
Overlay-Bundle**.

Neue Commands (Wrapper, in `commands/mod.rs`):

- `assistant_copy(text)` → `clipboard::write_text_to_clipboard` muss von `fn`
  auf `pub(crate) fn` (`clipboard.rs:128`). `navigator.clipboard` ist im
  Overlay unzuverlässig, weil das Fenster `focusable(false)` ist.
- `assistant_paste(text)` → `clipboard::paste` (`:1096`)
- `assistant_replace(text)` → Ctrl/Cmd+A via `input.rs` und dann `paste`
- `assistant_dismiss()` → Overlay verstecken
- `assistant_escalate()` → Frage erneut an `smart_assistant` schicken

Link öffnen: `openUrl` aus `@tauri-apps/plugin-opener`; `recording_overlay` steht
bereits in `capabilities/default.json:5`, also keine Capability-Arbeit.

i18n: neue Keys unter `overlay.assistant.*` in allen 24 Locales
(`check-translations.ts` erzwingt Vollständigkeit).

---

## Phase F — Einstellungen-UI je Assistent

Neue Sektion `src/components/settings/assistants/AssistantSettings.tsx`, ein
`SettingsGroup` je Profil, wiederverwendet `ShortcutInput`, `ProviderSelect`,
`ApiKeyField`, `ModelSelect` (`settings/PostProcessingSettingsApi/`) plus
Textarea für den Systemprompt, Zahlenfelder für Länge/Timeout und ein Select für
den Ausgabemodus. Vorbild ist
`settings/meetings/MeetingSummarySettings.tsx:1-205` — dort existiert bereits ein
eigenständiges Profil-UI mit eigenem Provider/Modell/Key.
Registrierung in `src/components/shell/settingsSections.ts:61-97`.
State-Hook analog `usePostProcessProviderState.ts`.

---

## Phase G — Verifikation

`bun run check` (Translations, Bindings-Drift, Log-Hygiene), dann `bun run gate`
(Lint, Format, Frontend-Build, cargo fmt/clippy/test).

## Reihenfolge / Parallelität

```
A ─┐
B ─┴→ C → D ─┬→ E ─┐
             └→ F ─┴→ G
```

A und B sind disjunkt (A: audio + schema, B: llm_client) und laufen parallel.
C berührt schema.rs und muss auf A warten. E und F sind nach D disjunkt
(E: overlay + commands, F: settings-UI).

---

## Review-Runde (2026-09-02): Befunde und Reparaturen

Nach Abschluss der Phasen A bis F liefen ein adversarialer Code-Review und
eine externe Recherche (Vorbilder, Prompt-Richtlinien der Anbieter, bekannte
Fehlermuster von Sprache-zu-LLM-Pipelines, Windows-Audio-Dokumentation). Die
Befunde sind hier festgehalten, weil sie das Design an mehreren Stellen
korrigieren und die Begründung sonst nur in der Git-Historie stünde.

### Kritisch (behoben)

- **Beenden während geduckt liess die Lautstärke dauerhaft bei 20 %.** Der
  Snapshot lebte nur im RAM, `RunEvent::Exit` stellte nichts wieder her.
  Jetzt: `pending_duck_restore` wird beim Engagieren persistiert und beim
  nächsten Start konsumiert; `Exit`/`ExitRequested` stellen synchron ohne Ramp
  wieder her (`remove_recording_audio_immediately`).
- **Mit abgeschaltetem Overlay produzierte der Assistent keine Ausgabe** -
  auch keine Fehlermeldung. Auf Linux ist das der ausgelieferte Standard.
  Jetzt: `plan_answer_delivery` kennt `card_available`; ohne Karte wird die
  Antwort in die Zwischenablage kopiert und jede Meldung geht als
  `assistant-notice` an das Hauptfenster (Toast).

### Wichtig (behoben)

- Stale Fade-Write konnte nach dem Restore landen (Prozess-Spawn auf
  Linux/macOS dauert länger als die Rampe): Schreibzugriffe laufen jetzt
  unter `VOLUME_WRITE_LOCK`, Generationsprüfung und Write sind atomar.
- Readiness-Thread konnte nach dem Stop noch ducken: `apply_recording_audio`
  prüft die Generation erneut _unter dem Lock_.
- Restore überschrieb eine Lautstärke, die der Nutzer währenddessen selbst
  geändert hatte: `duck_target` wird gemerkt, Abweichung > 0.015 gilt als
  Nutzeraktion und der Restore entfällt.
- Eskalation nach Schliessen der Karte tauchte Minuten später wieder auf bzw.
  verschwand bei `Copy`/`Paste`-Profil stumm: `answer_card_generation`
  entscheidet - Schliessen bricht die Eskalation ab, eine offene Karte
  bekommt das Ergebnis unabhängig vom Ausgabemodus des Smart-Profils.
- Doppeltes Lesen des Ausgabemodus (Emit vs. Present) konnte den Spinner
  dauerhaft stehen lassen: `expects_card` reist im Event-Payload, dazu ein
  2,5-s-Watchdog (`hide_overlay_if_unchanged`).
- Clipboard-Fehler im `Copy`-Modus war unsichtbar: neuer
  `AssistantError::ClipboardFailed`, gemeldet über `assistant-notice`.
- Overlay ausschalten bei offener Karte liess die Karte stehen:
  `change_overlay_style_setting` schliesst sie.
- STT-Fehler wurde als Assistenten-Fehler etikettiert und schickte den Nutzer
  zu den falschen Einstellungen: `AssistantError::TranscriptionFailed`.
- Unit-Test las den echten Schlüsselbund und schickte bei vorhandenem Key eine
  echte Anfrage an OpenAI: `resolve_request` (rein) von `perform_request`
  (Netz) getrennt; Tests laufen ohne Keychain und ohne Netz.

### Aus der Recherche übernommen

- **Hotkey-Defaults weg von Ctrl+Alt**: das ist AltGr auf internationalen
  Layouts (Ctrl+Alt+S tippt `s-acute` auf Polnisch, `d-stroke` auf
  Kroatisch). Neu `ctrl+shift+a` / `ctrl+shift+d`, macOS `option+ctrl+a/d`.
- **Reasoning-Budget**: Smart Assistant 2048 -> 4096 Tokens. Anbieter zählen
  Reasoning-Tokens gegen dieselbe Obergrenze; ein antwortgrosses Budget kam
  nach 90 s leer zurück. `finish_reason == "length"` wird jetzt gelesen:
  leer + abgeschnitten -> `AnswerBudgetExhausted` (nennt die Einstellung),
  Text + abgeschnitten -> Karte zeigt den Hinweis.
- **Prompts** nach Anbieter-Richtlinien überarbeitet: positive Ausgabe-Spec
  statt Verbotsliste, Kontext "wo erscheint die Antwort", explizites
  Plain-Text/kein LaTeX (Modelle fallen bei Einheitenumrechnung sonst in
  LaTeX), begrenzte Korrekturlizenz für ASR-Text, Annahme benennen statt
  Rückfrage, Erlaubnis für "weiss ich nicht"; Fast Helper mit Beispielen,
  Smart Assistant zielorientiert mit Längenziel.
- **Blank-Guard** gegen ASR-Halluzinationen auf Stille ("Thank you." ist ein
  Viertel aller solchen Ausgaben): exakte Phrasenliste, bewusst _kein_
  Wortzahl-Gate, damit "why?" weiter funktioniert.
- **Eingabeobergrenze** 24.000 Zeichen (`QuestionTooLong`), damit eine
  vergessene Aufnahme nicht als bezahlter 400er endet.
- URL-Extraktion: nicht-ASCII-Satzzeichen am Ende werden abgeschnitten,
  `]` beendet eine URL (Markdown-Link mit URL als Text).

### Bekannte Grenzen (bewusst offen)

- `assistant-notice` ist ein Toast im Hauptfenster. Ist die App in den Tray
  minimiert, sieht ihn niemand. Nächster Schritt wäre eine Tray-Notification.
- Microsoft rät von `IAudioEndpointVolume` für Shared-Mode-Streams ab; der
  OS-eigene Ducking-Pfad (Capture-Stream auf dem `eCommunications`-Endpoint)
  würde Absturzsicherheit vom Audio-Engine geschenkt bekommen, aber die
  Dämpfungsstärke wäre nicht mehr unsere Wahl - der Slider wäre eine Lüge.
  Bewusst beim Endpoint-Pfad geblieben, jetzt mit Persistenz und Abbruchregel.
- Der Mute-Zustand (Modus "Stumm") wird nicht persistiert; ein Absturz während
  Mute lässt die Anlage stumm. Gleiche Reparatur wie beim Volume wäre möglich.
- Keine Streaming-Antwort: bei 90 s Timeout nur Spinner plus Abbrechen. Alle
  Vorbilder auf dieser Oberfläche streamen.
- `assistant_replace` sendet Ctrl+A an das gerade fokussierte Fenster - nicht
  an das Fenster der Frage - und ist von der App nicht rückgängig zu machen.
  Verweigert leeren Text; mehr Schutz ist technisch nicht möglich.
- Fehlertexte des Backends sind Englisch (Repo-Praxis), also nicht lokalisiert.
- Bestehende Stores behalten ihre gespeicherten Profilwerte (alter Prompt,
  `max_output_tokens: 2048`): `ensure_assistant_defaults` sät nur fehlende
  Profile und überschreibt bewusst keine editierbaren Felder. Die neuen Prompts
  und das 4096-Budget erreichen einen Bestandsnutzer nur per Hand oder über eine
  noch nicht gebaute "Auf Standard zurücksetzen"-Aktion.

### Live-Tests (bewusst `#[ignore]`, nie im Gate)

Alle anderen Tests enden an der Netzwerkgrenze. Diese sechs verlassen die
Maschine, brauchen echte Zugangsdaten und kosten Kontingent; sie werden
absichtlich von Hand gestartet.

- `llm_client::tests::live_*` (3): Modellliste, gekappte Anfrage in
  Fast-Helper-Form, Timeout-Marker - gegen den ausgelieferten
  OpenRouter-Katalogeintrag. Braucht `OPENROUTER_API_KEY`.
- `assistant::tests::live_*` (3): WAV-Fixture -> Azure Fast Transcription ->
  Blank-Guard -> `resolve_request` -> OpenRouter. Braucht den Azure-Key im
  Schlüsselbund, `OPENROUTER_API_KEY` und die Fixtures aus
  `scripts/verify/generate-tts-fixtures.ps1` (Windows SAPI, nach
  `logs/tts-fixtures`).

```
cargo test --lib llm_client::tests::live_ -- --ignored --nocapture
cargo test --lib assistant::tests::live_  -- --ignored --nocapture
```

Letzter Lauf (2026-09-02): `silence.wav` -> Azure liefert kein Transkript,
kein Anbieter-Aufruf. `short_thanks.wav` -> "Thank you." -> vom Guard
verweigert. `question_en.wav` -> "How many grams are in 4 ounces?" ->
"113.4 g.", nicht abgeschnitten.

### Nutzerentscheidung: Hotkeys unter Diktieren (2026-09-02)

Entgegen dem Phase-F-Entwurf (Kürzel im Profil-Block) stehen die beiden
Assistenten-Kürzel jetzt unter Diktieren neben Transkribieren und Abbrechen
(`general/AssistantShortcuts.tsx`); der Assistenten-Tab verweist nur noch
dorthin. Begründung des Nutzers: jede Tastenkombination, die eine Aufnahme
startet, gehört zum Speak-Feature - an einen Ort. Die Binding-IDs bleiben der
Backend-Vertrag in `assistantProfiles.ts`.
