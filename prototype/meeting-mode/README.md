# Speak – Meeting-Mode Klick-Prototyp

> **Nur Mock – kein Produkt, kein Backend.** Dies ist ein statischer,
> anklickbarer Klick-Prototyp aus HTML/JS/CSS ohne jegliches Backend:
> keine echten Aufnahmen, keine Transkription, keine Netzwerk- oder
> Provideraufrufe und keine Persistenz über einen Seitenneuladevorgang
> hinaus (`app.js` verwendet weder `fetch` noch `localStorage` o. ä.).
> Alle Daten, Zeiten, Zähler und Ergebnisse sind fest eingebaute
> Mock-Daten. Der Prototyp illustriert die Informationsarchitektur aus
> [Plan §6.3](../../../docs/meeting-mode-plan.md) (dekomponiert aus dem
> zeitüberschrittenen task-0025, Teil 1/5) und dient allein der
> Erprobung von Abläufen, Texten und Gate-Kriterien vor der
> Implementierung – **sein Verhalten sagt nichts über das Verhalten,
> die Performance oder die Zuverlässigkeit der späteren Anwendung
> aus.** Kein Build-Schritt, keine Abhängigkeiten.

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
| **Projekte** | Arbeitsübersicht mit Suche (Namen, Status, Aufgaben, normalisierte Quellentexte), Sortierung `Zuletzt aktiv` / `Dringendste Aufgabe` / `Name`, Karten mit Status und nächster dringender Aufgabe sowie Eingang „Nicht zugeordnet“ mit Zähler-Badge; Leerer Zustand mit genau einer Primäraktion `Projekt erstellen` und sekundärem Import (§6.16) |
| **Projekt-Detailansicht** | Kopfzeile mit belegbarem Kurzstatus + letztem KI-Lauf, Tabs `Übersicht / Aufgaben / Meetings / Quellen`; jede KI-Aussage und jeder Task mit Beleglink in die gemockte Quelle, Aufgabenboard `Vorgeschlagen / Offen / Erledigt / Verworfen` mit direkt erreichbaren Aktionen und `Warum dringend?` inkl. Begründung + Belegen (§6.17) |
| **Projektzuordnung** | Eingang mit Stapelzuordnung (immer mit Vorschau), KI-Projektvorschlag nach der Verarbeitung mit Confidence + zwei Belegen und den gleichwertigen Wahlen `Zuordnen / Anderes Projekt … / Ohne Projekt lassen` – ohne Wahl wird nichts verschoben (§6.18) |
| **WhatsApp-Import** | Vier-Schritt-Assistent: Datei wählen → Erkanntes prüfen (inkl. Dubletten-Vorschau) → Projekt & Verarbeitung mit exakter Cloud-Offenlegung + Kostenschätzung → Import bestätigen mit Fortschritt und Ergebnis (importiert/übersprungen/fehlerhaft) (§6.19) |
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

## Vom Prototyp intendierte Usability-Gates (Plan §6.15, §6.21)

Der Prototyp ist gebaut, um die Gate-Kriterien aus
[Plan §6.15](../../../docs/meeting-mode-plan.md) (Anfänger- und
Usability-Gates) und [§6.21](../../../docs/meeting-mode-plan.md)
(erweiterte Usability-Gates) im Mock durchspielbar zu machen. Da alles
Mock ist, kann der Prototyp diese Gates nur *vorbereiten* (Klickdurchlauf
mit Testpersonen, automatisiertes Prüfen des Mock-Verhaltens); ob das
echte Produkt sie erfüllt, entscheidet sich erst in der Implementierung.

Pflichtaufgaben §6.15 – im Prototyp nachspielbar:

| §6.15 | Aufgabe | Wo im Prototyp |
| --- | --- | --- |
| 1 | Normales Diktat starten/beenden | Ansicht `Diktieren` mit `Diktat testen` (reine Vorschau) |
| 2 | Lokales Meeting mit Mikrofon + Systemaudio starten | Startdialog-Statuszeilen Mikrofon/Systemaudio, `Meeting starten` |
| 3 | Azure wählen, Cloud-Hinweis verstehen | Provider-Zeile → Picker `Azure Cloud`; Upload-Hinweis auch in der Ersteinrichtung (§6.6) |
| 4 | Meeting pausieren und sicher beenden | Laufendansicht `Pause/Fortsetzen`, `Meeting beenden` mit Bestätigung (§6.9) |
| 5 | Fertige Zusammenfassung kopieren | Detailansicht-Tab `Übersicht`, Button `Kopieren` (Mock) |
| 6 | Meeting-Shortcut ändern, ohne Diktier-Shortcut zu überschreiben | Ersteinrichtung Schritt 3; die Diktier-Shortcuts sind dort ausdrücklich als unverändert ausgewiesen |
| 7 | Geräte-/Netzwerkfehler beheben | Simulationen in Ersteinrichtung/Startdialog: fehlendes Mikrofon, verweigerte Berechtigung, fehlender Azure-Schlüssel – jeweils mit Korrekturaktion direkt am Fehler |

Nicht im Mock abbildbar: §6.15 Aufgabe 8 (unterbrochene Verarbeitung nach
Neustart fortsetzen) – der Prototyp hat keinen Neustart und keine
Persistenz. Auch die §6.15-Anforderung „Status aus Hauptfenster, Overlay
und Tray eindeutig“ ist nur teilweise darstellbar: Es gibt Hauptfenster-
Statuszeile und angehefteten Listeneintrag, aber kein Overlay- oder
Tray-Mock.

Erweiterte Gates §6.21 – im Prototyp nachspielbar:

| §6.21 | Aufgabe | Wo im Prototyp |
| --- | --- | --- |
| 1 | Projekt erstellen | `Projekte` → `Projekt erstellen` (genau ein Pflichtfeld Name) |
| 2 | Beim Meetingstart Projekt wählen/entfernen | Startdialog-Projektchip inkl. gestricheltem `Kein Projekt` |
| 3 | Fertiges, nicht zugeordnetes Meeting zuweisen | Eingang `Nicht zugeordnet` mit Stapelzuordnung + Vorschau; KI-Vorschlag nach Verarbeitung |
| 4 | WhatsApp-Export lokal importieren, Vorschau verstehen | Import-Assistent Schritte 1–4 inkl. Dedup-Vorschau und lokaler Offenlegung |
| 5 | Erkennen, welche Bilder bei Cloud-Verarbeitung hochgeladen würden | Importschritt 3: Cloud-Offenlegung nennt Nachrichtentexte/Bilder (Base64)/Sprachnachrichten und was bleibt |
| 6 | KI-Aufgabenvorschlag annehmen/ändern/verwerfen | Projekt-Detail `Aufgaben`: Annehmen/Erledigen/Verwerfen; Meeting-Vorschlag: `Zuordnen / Anderes Projekt … / Ohne Projekt lassen` |
| 7 | Von Statusaussage zur Belegstelle springen | Jeder KI-Aussage/Jeder Aufgabe Beleglink → Quellenansicht mit hervorgehobener Zeile |
| 8 | Gleiches Projekt im kleinen Fenster und maximiert | Nur teilweise: `Geteilte Ansicht` erscheint nur auf breiten Bildschirmen; responsives Verhalten selbst ist nicht Gegenstand der Mock-Prüfung |

Release-Gates aus beiden Abschnitten, die der Prototyp mitprüft:
Dedup ohne Dubletten bei Wiederholimport, keine Cloud-Verarbeitung vor
expliziter verständlicher Bestätigung, keine Hauptfunktion nur per
Hover/Drei-Punkte-Menü, Löschbestätigung nennt die betroffenen Artefakte,
Fehlermeldungen nennen eine erkennbare nächste Aktion. Nicht im Prototyp
prüfbar: Tastatur-/Screenreader-Zugänglichkeit des echten Produkts und
die Verwechslungsfreiheit von Diktat- und Meetingstart unter echten
Testpersonen (dazu braucht es den Testdurchlauf mit Personen, den §6.15
vorsieht).

## Verifizierung (in scope)

Es gibt zwei Prüfharnesses in diesem Verzeichnis:

- **`verify.cjs`** – das vollständige Node/Playwright-Harness. Es prüft
  alle dokumentierten Gates und Regressionen dieses Prototyps
  (task-0179 bis task-0181 sowie die Journey-Gates G1–G4 des
  Klick-Prototypen der Kern-Journey: Navigation, Countdown,
  Farbsemantik, Scroll-Lock, Stop-Bestätigung, Verarbeitungs-
  Checkliste, Listen-/Detailansicht, Projekte, Zuordnung,
  WhatsApp-Import, 44px-/Namen-/Fokus-Audit, Plan-Annotationen)
  gegen `index.html` und gibt je Prüfung eine `PASS`-/`FAIL`-Zeile aus.
  Aufruf aus dem Repo-Stamm:

  ```powershell
  node prototype\meeting-mode\verify.cjs
  ```

  Voraussetzungen: einmal `bun install` im Repo-Stamm (liefert
  Playwright über `@playwright/test`) sowie die installierten
  Playwright-Chromium-Browser (`npx playwright install chromium`);
  das Skript findet die installierte Chromium-Version selbständig unter
  `%LOCALAPPDATA%\ms-playwright`. Erfolg = Exit-Code 0 ohne
  `FAIL`-Zeilen. Das Skript lädt ausschließlich `index.html` per
  `file://` – es startet kein Backend, weil es keines gibt.

  Fehlersuche in restriktiven Umgebungen: Blockiert die Sandbox
  Playwrights Pipe-Transport (EPERM), startet das Skript selbst einen
  Chromium mit TCP-Debugport und hängt sich per CDP an. Mit
  `VERIFY_CDP_URL=http://127.0.0.1:9333` verbindet es sich stattdessen
  mit einem extern gestarteten Chromium unter dieser CDP-Adresse.
  Darf Chromium keine Kindprozesse/Crashpad starten, ist der Lauf mit
  erweiterten Prozessrechten auszuführen.

- **`verify.html`** – ein schlankerer In-Browser-Prüfharness für einen
  Teil der Prüfungen (task-0179-Gates G1–G4 plus Regressionen zu
  Ersteinrichtung, Startdialog, Liste, Detail). Er lädt `index.html`
  in einem iframe und schreibt `PASS`/`FAIL`-Zeilen ins DOM; am Ende
  muss `RESULT: PASS` stehen. Ohne Server ausführbar, z. B. mit einem
  beliebigen installierten Chromium/Chrome-Headless-Build (die
  Versionsnummer im Pfad variiert):

  ```powershell
  & "$env:LOCALAPPDATA\ms-playwright\<chromium-build>\chrome-headless-shell-win64\chrome-headless-shell.exe" `
    --headless --no-sandbox --allow-file-access-from-files `
    --virtual-time-budget=25000 --dump-dom `
    "file:///<absoluter-Pfad>/prototype/meeting-mode/verify.html"
  ```

Beide Harnesses prüfen nur das Mock-Verhalten des Prototyps – ein
grüner Lauf besagt nichts darüber, ob die spätere Implementierung
diese Verhaltensweisen erfüllt.

## Bewusste Nicht-Ziele (Scope dieses Blattes)

- Keine echte Audio-, Transkriptions-, Parser- oder Providerlogik.
- Kein Anschluss an die Tauri-App; vollständig losgelöst von `src/`.

## Gates dieses Blattes (Klick-Prototyp der Kern-Journey, §6.5–§6.13)

| Gate | Umsetzung | Nachweis |
| --- | --- | --- |
| G1 – Alle sieben Stationen (leer, Ersteinrichtung, Startdialog, Aufnahme, Pausiert, Verarbeitung, Detail) klickbar End-to-End mit realistischen deutschen UND englischen Inhalten | Leerer Zustand per Simulation klickbar (`sim-no-meetings`); englische Sprachwahl im Startdialog schaltet Live-Transkript und auto-generierte Kurzfassung auf Englisch; Seed-Bibliothek enthält „Weekly Sync (English)" neben deutschen Meetings; Journey endet in der Detailansicht mit allen vier Tabs | verify.cjs: alle `leaf-G1 …`-Prüfungen |
| G2 – Jedes interaktive Element ≥44×44 CSS-px, mit Textlabel oder zugänglichem Namen, sichtbarer Tastaturfokus | Systematischer Audit über alle Ansichten + Dialoge (inkl. geöffnetem Aktionsmenü): Größe und Name jedes Elements; native Checkboxen/Radios sind auf 44 px vergrößert, `<a>`-Buttons via inline-flex, Beleglinks über Hitflächen-Padding; Fokus-Outline an 12 Tab-Stops geprüft | verify.cjs: alle `leaf-G2 …`-Prüfungen inkl. „keyboard focus visibly outlined" |
| G3 – Lauf über lokalen Vorschaubefehl, keine Dateien außerhalb `prototype/meeting-mode/` | `index.html` funktioniert direkt über `file://` (siehe oben); das Harness lädt genau diese Datei und meldet „No JS errors."; es werden keine App-Quelldateien berührt | verify.cjs-Lauf: Exit 0, „No JS errors." |
| G4 – Jeder Screen trägt eine sichtbare Annotation seines Plan-Abschnitts | Sichtbare `.plan-tag`-Zeile pro Screen: §6.5 (Meetings inkl. leer), §6.6, §6.7, §6.11 („Aufnahme/Pause"), §6.9 (Countdown + Stop-Bestätigung), §6.12, §6.13 | verify.cjs: alle `leaf-G4 …`-Prüfungen |

Anti-Muster nach §6.1/§6.8 sind ebenfalls geprüft: Rot nur im echten
Aufnahmezustand (Pause orange, beide mit Icon+Text), keine Icon-only-Navigation,
Löschen nie nur per Hover (Drei-Punkte-Menü per Klick).

## Gates dieses Blattes (task-0181, Teil 5/5)

| Gate | Umsetzung | Nachweis |
| --- | --- | --- |
| G1 – Jede KI-Aussage und jeder Task zeigt ≥1 Beleglink zu einer gemockten Quelle | Übersicht (Status, Entscheidungen, Fragen), Aufgabenkarten, `Warum dringend?`, Detail-Kopfzeile und Projektvorschlag enthalten Beleglinks; Klick öffnet die Quellenansicht mit hervorgehobener Stelle | verify.cjs: „G1 every overview statement has ≥1 evidence link“, „G1 every task shows ≥1 evidence link“, „G1 evidence link opens mocked source with highlighted line“ |
| G2 – KI-Projektvorschlag erfordert explizite Wahl | Nach der Verarbeitung erscheint der Vorschlag mit Confidence + zwei Belegen; `Zuordnen` / `Anderes Projekt …` / `Ohne Projekt lassen` sind gleichwertig erreichbar, ohne Wahl wird nichts verschoben | verify.cjs: „G2 suggestion appears after processing…“, „G2 exactly three equal choices…“, „G2 nothing is moved without an explicit user choice“ |
| G3 – Importschritt 3 listet exakt, was das Gerät verlässt | Standard ist „Nur lokal importieren“; bei Cloud-Wahl listet die Offenlegung Nachrichten, Bilder, Sprachnachrichten und ausdrücklich, was bleibt (Video, nicht unterstützte Dateien), plus Kostenschätzung | verify.cjs: „G3 default is local-only…“, „G3 cloud choice discloses exactly which content leaves the device“, „G3 disclosure names what explicitly stays on the device“ |
| G4 – Wiederholter Import zeigt Dedup-Vorschau, keine Dubletten | Datei 2 überlappt mit dem importierten Archiv; Schritt 2 listet jede Dublette vorab, Schritt 4 importiert nur neue Nachrichten und weist übersprungene aus | verify.cjs: „G4 repeated import shows dedup preview listing each duplicate“, „G4 result imports only new messages, skips duplicates without doubling“ |

## Gates vorheriger Blätter (task-0180, Teil 4/5)

| Gate | Umsetzung | Nachweis |
| --- | --- | --- |
| G1 – Checkliste mit deterministischen Stückzahlen, keine fake Prozentanzeige bei unbekannter Gesamtmenge | Checkliste zählt aus den gespeicherten Meeting-Daten (`processing.chunksTotal`); in der Vorbereitungsphase steht bewusst keine Zahl; Prozentzeichen kommen nirgends vor | verify.cjs: „G1 while total unknown…“, „G1 deterministic n von n…“, „G1 checklist never uses a fake percentage“ |
| G2 – Löschen ohne Hover erreichbar, Bestätigung nennt Artefakte | Drei-Punkte-Menü per Klick (Liste) und „Weitere Aktionen“ (Detail); `window.confirm` nennt Titel sowie Audio, Transkript und Zusammenfassung | verify.cjs: „G2 three-dot menu opens by click…“, „G2 delete confirm names the artifacts…“ |
| G3 – Transkript-Suche springt zur gemockten Segmentposition | Suche hebt Treffer hervor, fokussiert den ersten, scrollt ihn in Sicht und nennt „Segment k von n · Audioposition mm:ss“ | verify.cjs: „G3 search jumps to mocked segment position“ |
| G4 – Liste → Detail → zurück verzahnt | Kartenklick öffnet Detail, `← Meetings` zurück, Titeländerungen erscheinen in der Liste, Checkliste bietet „Meeting öffnen“ | verify.cjs: „G4 list card opens detail view“, „G4 detail back returns to list“, „G4 edited detail title appears in list“ |
