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
| **Meetings** | Leerer Zustand, Liste mit Suche/Filter, sichtbarem Drei-Punkte-Menü; aktives Meeting immer oben (§6.5) |
| **Projekte** | Karten inkl. Eingang „Nicht zugeordnet“ mit Zähler-Badge (§6.16) |
| **Diktatverlauf** | kurzer Verlauf mit Kopieren-Mock |
| **Einstellungen** | eigene Seite mit sekundären Tabs: Diktieren · Meetings · Modelle & Provider · Allgemein · Erweitert |

Während ein Mock-Meeting läuft, erscheint direkt unter `Meetings` der
angeheftete Eintrag **„Aufnahme läuft · 00:00:12“** mit rotem Statuspunkt
(§6.3) und zählt live.

## Ablauf zum Ausprobieren

1. `Meetings` → `Neues Meeting` öffnet den kompakten Startdialog (§6.7).
2. `Meeting starten` wechselt zur Laufendansicht (§6.11) und aktiviert den Pin.
3. `Meeting beenden` fragt erst nach (`Meeting wirklich beenden?`, §6.9), dann
   läuft die Verarbeitungs-Checkliste (§6.12).
4. Der Pin verschwindet; das Meeting erscheint in der Liste als „In Verarbeitung“.

## Mock-Steuerung

Der Sidebar-Button **Mock zurücksetzen** beendet eine Mock-Session und setzt den
Prototyp zurück.

## Verifizierung (in scope)

`verify.html` ist ein In-Browser-Prüfharness für die Gates G1/G2: Er lädt
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
Repo-Stamm; benötigt die Playwright-Chromium-Browser).

## Bewusste Nicht-Ziele (Scope dieses Blattes)

- Keine echte Audio-, Transkriptions- oder Providerlogik.
- Keine Projekt-Detailansicht, kein Importassistent (spätere Teilblätter von task-0025).
- Kein Anschluss an die Tauri-App; vollständig losgelöst von `src/`.
