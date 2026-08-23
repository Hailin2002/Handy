# Speak – Meeting-Mode Klick-Prototyp

Statischer, selbständiger Prototyp der neuen Informationsarchitektur aus
[Plan §6.3](../../../docs/meeting-mode-plan.md) (dekomponiert aus dem
zeitüberschrittenen task-0025, Teil 1/5). Kein Build-Schritt, keine
Abhängigkeiten, keine echten Aufnahmen – alle Daten sind Mock-Daten.

## Öffnen (in scope)

Die Datei `index.html` direkt im Browser öffnen – sie funktioniert über `file://`
ohne Server:

```
prototype/meeting-mode/index.html
```

Alternativ in PowerShell:

```powershell
Start-Process "prototype\meeting-mode\index.html"
```

## Inhalt

Primärnavigation gemäß Plan §6.3:

| Bereich | Umfang im Prototyp |
| --- | --- |
| **Diktieren** | Statuszeilen, Shortcut-Keycap, `Diktat testen` mit reiner Vorschau; bewusst kein Meeting-Startbutton (§6.4) |
| **Meetings** | Leerer Zustand, Liste mit Suche/Filter, Karten mit Fortschritt und Zusammenfassung, sichtbarem Drei-Punkte-Menü (Export/Erneut verarbeiten/Löschen ohne Hover); aktives Meeting immer oben (§6.5). Kartenklick öffnet die Detailansicht |
| **Meeting-Detailansicht** | Tabs `Übersicht` / `Transkript` / `Audio` / `Verarbeitung`; editierbarer Titel; Kopieren/Exportieren plus beschriftetes Menü „Weitere Aktionen“ inkl. Löschen mit benannten Artefakten; Transkript-Suche springt zur Segmentposition inkl. Audioposition; optionale `Geteilte Ansicht` nur auf breiten Bildschirmen (§6.13) |
| **Meeting-Ersteinrichtung** | Drei Schritte: Verarbeitung wählen (Lokal empfohlen, Azure mit Upload-Hinweis, Custom unter „Weitere Optionen“), Audio prüfen mit Livepegel-Mocks und Korrekturaktionen bei fehlendem Gerät/verweigerter Berechtigung, Meeting-Shortcut mit Verfügbarkeitsprüfung und konfliktfreien Alternativen (§6.6) |
| **Kompakter Startdialog** | Vier komplett anklickbare Statuszeilen mit Mini-Pickern (eine Zeile = eine Entscheidung), Projektauswahl per Chip inkl. sichtbar unterschiedenem „Kein Projekt“, Summary-Schalter, Speicherhinweis, genau ein Primärbutton `Meeting starten` mit nachvollziehbarem Deaktivierungsgrund (§6.7) |
| **Projekte** | Karten inkl. Eingang „Nicht zugeordnet“ mit Zähler-Badge (§6.16) |
| **Diktatverlauf** | kurzer Verlauf mit Kopieren-Mock |
| **Einstellungen** | eigene Seite mit sekundären Tabs: Diktieren · Meetings · Modelle & Provider · Allgemein · Erweitert |

Während ein Mock-Meeting läuft, erscheint direkt unter `Meetings` der
angeheftete Eintrag **„Aufnahme läuft · 00:00:12“** mit rotem Statuspunkt
(§6.3) und zählt live.

## Ablauf zum Ausprobieren

1. `Meetings` → `Ersteinrichtung` führt durch die Meeting-Ersteinrichtung
   (§6.6); `Abschließen` landet direkt im kompakten Startdialog.
2. `Meetings` → `Neues Meeting` öffnet den kompakten Startdialog (§6.7).
3. `Meeting starten` zeigt zuerst den 3-Sekunden-Countdown (§6.9);
   `Escape` oder `Abbrechen` bricht ab, ohne etwas aufzunehmen. Danach
   wechselt der Prototyp zur Laufendansicht (§6.11) und aktiviert den Pin.
4. In der Laufendansicht hält das Transkript automatisch zum aktuellen Text,
   solange nicht nach oben gescrollt wird; sonst erscheint
   `Zum aktuellen Text`. `Pause/Fortsetzen` schreibt einen Timeline-Marker.
5. `Meeting beenden` fragt erst nach (`Meeting wirklich beenden?`, §6.9), dann
   läuft die Verarbeitungs-Checkliste (§6.12): deterministische Stückzahlen
   („37 von 42“), bewusst keine Zahl und nie eine Prozentanzeige, solange die
   Gesamtmenge unbekannt ist, `Im Hintergrund weiterarbeiten` zurück zur Liste,
   und bei simuliertem Fehler `Fehlerhafte Abschnitte erneut versuchen`.
6. Der Pin verschwindet; das Meeting erscheint in der Liste als „In Verarbeitung“.
7. Klick auf eine Meeting-Karte öffnet die Detailansicht (§6.13) mit den Tabs
   Übersicht, Transkript, Audio und Verarbeitung – inklusive Transkript-Suche,
   die zum ersten Treffer springt und dessen Audioposition nennt. Zurück über
   `← Meetings`; auf breiten Bildschirmen steht optional die `Geteilte Ansicht`
   bereit.

Im Startdialog ist jede Statuszeile (Mikrofon, Systemaudio, Transkription,
Sprache) komplett anklickbar und öffnet einen Mini-Picker, der auf diese eine
Entscheidung begrenzt ist – der Dialog-Zustand (Titel, Projekt, Schalter)
bleibt erhalten. Fehlende Geräte oder ein fehlender Azure-Schlüssel lassen
sich direkt im Picker beheben; solange etwas fehlt, ist `Meeting starten`
deaktiviert und nennt den Grund (Gate G3).

Der Startdialog merkt sich die letzte Konfiguration über mehrere Öffnungen
(§6.7); nur der allererste Öffnungsvorgang und `Mock zurücksetzen` starten von
den Defaults. Der Titel wird bei jedem Öffnen mit Datum und Uhrzeit
vorbefüllt.

## Mock-Steuerung

Der Sidebar-Button **Mock zurücksetzen** beendet eine Mock-Session und setzt den
Prototyp zurück.

## Verifizierung (in scope)

`verify.html` ist ein In-Browser-Prüfharness für die Gates G1–G4: Er lädt
`index.html` in einem iframe und schreibt `PASS`/`FAIL`-Zeilen ins DOM. Ohne
Server ausführbar, z. B.:

```powershell
& "$env:LOCALAPPDATA\ms-playwright\chromium_headless_shell-1234\chrome-headless-shell-win64\chrome-headless-shell.exe" `
  --headless --no-sandbox --allow-file-access-from-files `
  --virtual-time-budget=25000 --dump-dom `
  "file:///<absoluter-Pfad>/prototype/meeting-mode/verify.html"
```

In der Ausgabe muss am Ende `RESULT: PASS` stehen. `verify.cjs` ist das
äquivalente Playwright-Harness (`node prototype\meeting-mode\verify.cjs` aus dem
Repo-Stamm; benötigt die Playwright-Chromium-Browser). Es findet die installierte
Chromium-Version automatisch unter `%LOCALAPPDATA%\ms-playwright`. Falls
Playwrights Pipe-Transport blockiert ist (EPERM), startet es selbst einen
Chromium mit TCP-Debugport; mit `VERIFY_CDP_URL=http://127.0.0.1:9333` hängt es
sich an einen extern gestarteten Chromium an. In restriktiven Sandboxes, in
denen Chromium keine Kindprozesse/Crashpad starten darf, ist der Lauf mit
erweiterten Prozessrechten auszuführen.

## Bewusste Nicht-Ziele (Scope dieses Blattes)

- Keine echte Audio-, Transkriptions- oder Providerlogik.
- Keine Projekt-Detailansicht, kein Importassistent (spätere Teilblätter von task-0025).
- Kein Anschluss an die Tauri-App; vollständig losgelöst von `src/`.

## Gates dieses Blattes (task-0180, Teil 4/5)

| Gate | Umsetzung | Nachweis |
| --- | --- | --- |
| G1 – Checkliste mit deterministischen Stückzahlen, keine fake Prozentanzeige bei unbekannter Gesamtmenge | Checkliste zählt aus den gespeicherten Meeting-Daten (`processing.chunksTotal`); in der Vorbereitungsphase steht bewusst keine Zahl; Prozentzeichen kommen nirgends vor | verify.cjs: „G1 while total unknown…“, „G1 deterministic n von n…“, „G1 checklist never uses a fake percentage“ |
| G2 – Löschen ohne Hover erreichbar, Bestätigung nennt Artefakte | Drei-Punkte-Menü per Klick (Liste) und „Weitere Aktionen“ (Detail); `window.confirm` nennt Titel sowie Audio, Transkript und Zusammenfassung | verify.cjs: „G2 three-dot menu opens by click…“, „G2 delete confirm names the artifacts…“ |
| G3 – Transkript-Suche springt zur gemockten Segmentposition | Suche hebt Treffer hervor, fokussiert den ersten, scrollt ihn in Sicht und nennt „Segment k von n · Audioposition mm:ss“ | verify.cjs: „G3 search jumps to mocked segment position“ |
| G4 – Liste → Detail → zurück verzahnt | Kartenklick öffnet Detail, `← Meetings` zurück, Titeländerungen erscheinen in der Liste, Checkliste bietet „Meeting öffnen“ | verify.cjs: „G4 list card opens detail view“, „G4 detail back returns to list“, „G4 edited detail title appears in list“ |
