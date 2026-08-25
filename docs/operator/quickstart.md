# Operator-Quickstart: Von der Installation bis zum ersten erfolgreichen Diktat

Status: **implemented behavior** (jede Verhaltensaussage ist mit Datei- und
Zeilen-/Symbolbeleg gegen den Codestand unten versehen; abgedeckt ist der
heutige Diktiermodus — der Meeting-Modus ist hier nicht implementiert und
wird nicht beschrieben).
Autor-Arbeitsstand: **HEAD-Commit `d546dfd80bfabf593df17820ca0915b0c5a943b3`**
(verifiziert am 22.08.2026 via `git rev-parse HEAD`; `git status --porcelain`
war bei der Verifikation leer).
Umfang: Einkanaliger Happy-Path für Endnutzer — Build beschaffen, erster
Start, Berechtigungen, Modellwahl, erstes Diktat per Shortcut, Ausgabe
bestätigen — plus Verweis in die drei Troubleshooting-Matrizen bei
Fehlschlag. Belegbasis: `src/App.tsx`,
`src/components/onboarding/{Onboarding,AccessibilityOnboarding}.tsx`,
`src-tauri/src/settings/schema.rs`, `src-tauri/src/settings/types.rs`,
`src-tauri/src/clipboard.rs`.
Plan: [docs/meeting-mode-plan.md](../meeting-mode-plan.md)
Index: [docs/meeting-mode-documentation-index.md](../meeting-mode-documentation-index.md)
Aufgenommen am: 22.08.2026

Dieses Dokument ist ein **Wegweiser, kein Ersatz**: Installationsdetails
stehen im [README](../../README.md) (Abschnitt „Quick Start“),
Build-Details in [BUILD.md](../../BUILD.md); hier wird weder dupliziert noch
widersprochen. Bei Abweichungen gilt: nichts schlägt den Code.

---

## 1. Der Happy-Pfad in fünf Schritten

### Schritt 1 — Build beschaffen

Für Endnutzer gilt unverändert der README-Weg (Beleg: `README.md`,
Abschnitt „Quick Start → Installation“, Zeilen 35–46):

- Fertiges Paket von der
  [Releases-Seite](https://github.com/cjpais/Handy/releases) oder der Website;
  alternativ macOS Homebrew (`brew install --cask handy`) bzw. Windows
  winget (`winget install cjpais.Handy` — Community-Pakete, siehe
  README-Hinweis).
- Wer aus dem Quellcode baut, folgt [BUILD.md](../../BUILD.md) samt den
  Voraussetzungen aus `AGENTS.md` (Rust, Bun; im Dev-Baum zwingend das
  Silero-VAD-Prerequisite — warum, steht in der Modell-Matrix, R11).

### Schritt 2 — Erster Start: die Onboarding-Reihenfolge

Die App prüft beim Start die persistierte Einstellung `onboarding_completed`
(`commands.getAppSettings()`, `src/App.tsx:226-233`) und wählt daraus die
Komponentenfolge. Die möglichen Schritte sind genau drei, in dieser Reihenfolge:
`accessibility` → `model` → Hauptapp (`OnboardingStep`-Typ
`src/App.tsx:33`; Renderverzweigung `src/App.tsx:327-332`).

| Startsituation | Ablauf | Beleg |
|---|---|---|
| Neuer Nutzer (`onboarding_completed !== true`) | Vollständiges Onboarding: zuerst Berechtigungen, dann Modellwahl | `src/App.tsx:274-278` |
| Rückkehrer mit vollständigem Onboarding, Berechtigungen intakt | Direkt Hauptapp, kein Onboarding | `src/App.tsx:234-273` |
| Rückkehrer, aber macOS-Berechtigungen (Mikrofon/Accessibility) oder Windows-Mikrofonfreigabe inzwischen entzogen | Erneut nur der Berechtigungs-Schritt, dann Hauptapp | `src/App.tsx:238-271` |
| Prüffehler beim Lesen der Einstellungen | Sicherungskurs: volles Onboarding | `src/App.tsx:279-282` |

### Schritt 3 — Berechtigungen erteilen (nur wo nötig)

Der Berechtigungs-Schritt verhält sich plattformabhängig
(`src/components/onboarding/AccessibilityOnboarding.tsx`):

- **macOS**: Zwei Karten — Mikrofon und Bedienungshilfen (Accessibility).
  Erst wenn **beide** gewährt sind, schreitet das Onboarding fort
  (`allGranted`, Zeilen 54–59). Nach dem Klick auf „Erteilen“ fragt die App
  sekündlich den Status ab (Polling, Zeilen 162–236); sobald Accessibility
  gewährt ist, initialisiert das Backend Enigo (Texteingabe) und die globalen
  Shortcuts (Zeilen 103–113, 193–201).
- **Windows**: Nur die Mikrofon-Karte. Der Button öffnet direkt die
  Windows-Datenschutzeinstellung fürs Mikrofon
  (`commands.openMicrophonePrivacySettings()`, Zeilen 261–275); ein
  Accessibility-Prompt existiert unter Windows nicht (Zeilen 50–52).
- **Linux/sonstige**: Der Schritt wird komplett übersprungen
  (Zeilen 88–93) — es gibt dort keinen Berechtigungsdialog.

Nach Abschluss werden Audio-Ein-/Ausgabegeräte neu eingelesen und nach
300 ms zum nächsten Schritt übergegangen (Zeilen 61–64).

### Schritt 4 — Modell wählen

Die Modellauswahl zeigt zuerst bereits auf der Platte vorhandene Modelle,
darunter die Downloadliste (`src/components/onboarding/Onboarding.tsx:157-177`).
Die Downloadliste ist kuratiert: veraltete `.bin`/ONNX-Quellen werden nie
angeboten (Zeilen 39–42), die ersten zwei empfohlenen Modelle des
rank-sortierten Katalogs erscheinen als Featured Cards — aktuell Parakeet
Unified (Englisch) und Nemotron Streaming (mehrsprachig), Kommentar an
Zeilen 34–38; alles Weitere liegt hinter „Show all“ (Zeilen 218–235).

Ablauf nach dem Klick: Download → Verifikation → Extraktion → automatische
Selektion → Übergang zur Hauptapp; erst wenn alle drei Phasen abgeschlossen
sind **und** das Modell als `is_downloaded` gemeldet wird, wird selektiert
und weitergeschaltet (Zeilen 62–102). Ein Fehlschlag bricht die Auswahl ab,
ohne das Onboarding zu verlassen.

### Schritt 5 — Erstes Diktat auslösen und Ausgabe bestätigen

Die Standardbindings sind plattformabhängig fest verdrahtet
(`src-tauri/src/settings/schema.rs:551-601`) und lassen sich später in den
Einstellungen ändern:

| Aktion | Windows/Linux | macOS | Default Push-to-Talk? |
|---|---|---|---|
| Diktieren | `Ctrl+Leertaste` | `Option+Leertaste` | ja — gedrückt halten, sprechen, loslassen |
| Diktat mit Nachbearbeitung | `Ctrl+Shift+Leertaste` | `Option+Shift+Leertaste` | ja |
| Aufnahme abbrechen | `Escape` | `Escape` | — |

Push-to-Talk ist ab Werk aktiviert (`default_push_to_talk() -> true`,
`schema.rs:197-199`): Shortcut **halten**, sprechen, loslassen.

Dann: In ein beliebiges Textfeld klicken (Ziel-App muss den Fokus haben),
Shortcut drücken, sprechen, loslassen. Der Text wird per Zwischenablage in
das fokussierte Feld eingefügt: Die Paste-Routine sichert den bisherigen
Zwischenablage-Inhalt, schreibt den Text hinein, sendet den Paste-Chord
(`Ctrl+V` bzw. Plattformäquivalent) und stellt danach den alten Inhalt wieder
her — der Default `clipboard_handling = "dont_modify"` hinterlässt die
Zwischenablage also unverändert (`src-tauri/src/clipboard.rs:54-121`,
Default-Zuweisung `schema.rs:642-643`). **Erfolgskriterium:** Der
gesprochene Text erscheint an der Cursorposition. Zum Abbrechen einer
laufenden Aufnahme `Escape` drücken (Binding-Tabelle oben).

Beleg zum Default: `clipboard_handling = ClipboardHandling::default()`
(`schema.rs:643`) und `#[default] DontModify`
(`src-tauri/src/settings/types.rs:158-164`).

## 2. Wenn etwas nicht funktioniert — Verweis in die Matrizen

Jeder Fehlschlag gehört in genau eine der drei bestehenden
Symptom-Matrizen; diese hier wiederholt keine Ursachen, sondern ordnet nur zu.
Log-Evidenz immer mit `--debug` sammeln (CLI-Flag, Definition in
`src-tauri/src/cli.rs`).

| Symptom im Quickstart | Zuständige Matrix | Einstieg |
|---|---|---|
| Kein Mikrofon gefunden, falsches Gerät, Aufnahme bleibt stumm, Gerät wechselt mitten im Diktat, Mikrofonberechtigung | [troubleshooting-audio-devices.md](troubleshooting-audio-devices.md) | Symptom-Matrix §2 (R-Fälle je Unterabschnitt 2.1–2.5), Diagnose-Schnellpfad §3 |
| Modell-Download startet nicht / bricht ab / friert ein, Prüfsummen- oder Größenfehler, Offline-/Drop-in-Setup, Speicherplatz | [troubleshooting-models.md](troubleshooting-models.md) | Symptom-Matrix §3 (R1–R15), Diagnose-Schnellpfad §4 |
| Shortcut reagiert nicht oder doppelt belegt, PTT-Eigenheiten, CLI-Flags wirkungslos, Tray-/Hidden-Start-Sonderfälle | [troubleshooting-shortcuts-cli.md](troubleshooting-shortcuts-cli.md) | Symptom-Matrizen §4 (R1–R20), Diagnose-Schnellpfad §5 |

Sonderfälle, die bewusst **nicht** in diesen Quickstart gehören:

- Proxy/Firewall/Netzwerk-Einschränkungen beim Modellbezug: manueller
  Drop-in laut [README → Troubleshooting](../../README.md#troubleshooting)
  („Manual Model Installation“) sowie Modell-Matrix R12/R13.
- Dev-Baum ohne `src-tauri/resources/models/silero_vad_v4.onnx`
  (Aufnahme startet nicht, „VAD pre-load failed“): Modell-Matrix R11.
- Backup/Restore eines Bestands:
  [backup-restore-drill.md](backup-restore-drill.md).

## 3. Nicht-Umfang dieses Dokuments

- Keine Duplikation der README-Installations- oder BUILD-Bauanleitungen
  (nur Verweise, Schritt 1).
- Keine Wiederholung von Ursachen/Maßnahmen aus den drei Matrizen — bei
  Konflikt gelten die Matrizen mit ihren Codebelegen.
- Keine Beschreibung von Meeting-Modus-Pfaden: nicht implementiert,
  siehe Index, Abschnitt 1.
- Keine Aussage über künftige Onboardingschritte (z. B. Retentionsfrage):
  reine Planthemen gemäß Plan §12; solange kein Code existiert, steht hier
  nichts dazu.
