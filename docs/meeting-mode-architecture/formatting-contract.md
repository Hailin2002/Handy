# Locale-Formatierungsvertrag (Formatting Contract) — Zeitstempel, Dauern und Größen in Meeting-Oberflächen

Status: **spec contract — keine Implementierung vorhanden; ändert keinen
Quellcode.** Dieser Vertrag fixiert die eine Naht, über die alle
Meeting-Modus-Oberflächen Zeitpunkte, Dauern, Timeline-Marker, Bytegrößen und
relative Zeiten pro Locale rendern: ein gemeinsames Formatter-Modul im
Frontend, genau eine Zeitzonenregel je Fläche (Anzeige, Exportdateiname,
Exportinhalt) und die Export-Sprachregel.

Plan-Anker: [docs/meeting-mode-plan.md](../meeting-mode-plan.md) — §6.3
(Zeile 427: angehefteter Eintrag `Aufnahme läuft · 00:42:18`), §6.11
(Fixierte Statusleiste mit Dauer), §6.12 (Processing-Checkliste),
§6.13/§6.16/§6.17 (Meeting-Detail, Listen, Projektdetail mit „letzte
Aktualisierung“), §11 (Export als Markdown/TXT/JSON), §12 (Zeile 1438:
Speicherbedarf in MB pro Stunde).

Verwandte Dokumente (Autoritätsordnung wie im
[Dokumentationsindex](../meeting-mode-documentation-index.md): Plan schlägt
Verträge, Verträge schlagen Kataloge, nichts schlägt den tatsächlichen Code):

- [i18n-Namespace-Vertrag](i18n-namespace-contract.md) (I18N) — §7 Dauer-
  Zweiklassenmodell (`{{duration}}` als aufruferformatierter String),
  §10.2 Bidi-Isolation für eingebettete LTR-Token;
- [Export-/Projekt-Datenfluss](export-project-dataflow.md) (EPD) — §4.2
  Serializer-Formen (`[mm:ss]` / `[h:mm:ss]`-Marker), §4.3 Dateinamenmuster,
  §5 Provenance-Envelope; dieser Vertrag adjudiziert dessen offene
  Zeitzonen- und Sprachfragen (§3 und §8 hier);
- [Summary-Pipeline-Datenfluss](summary-pipeline-dataflow.md) (SPD) — Zeile 80
  (`t(ms)` = `HH:MM:SS`, Stunden **nicht** modulo 24);
- [Ingestion-Pipeline-Vertrag](ingestion-pipeline-contract.md) (IF) — §6.2
  Regel 2 (`occurred_at` ist ISO-8601 UTC ms);
- [Navigations-Seam-Vertrag](navigation-seam-contract.md) (NAV) — rAF-Uhr des
  angehefteten Eintrags ohne Store-Traffic;
- [Modulplatzierungskarte](module-placement-map.md) (MPM).

Arbeitsstand geprüft am 22.08.2026 im Worktree dieses Tasks (HEAD der
Dokumentationsbasis; alle Codebelege in §1 direkt gelesen).

---

## 1. Verifizierungsstand gegen den Code (ehrliche Kennzeichnung)

Alle Aussagen dieses Vertrags über bestehende Infrastruktur wurden direkt im
Code geprüft:

| Prüfung                                    | Methode                                                                | Ergebnis                                                                                                                                                                                                                                                                                                                                                                    |
| ------------------------------------------ | ---------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Bestehende Datums-/Relativformatierung     | Lesezugriff `src/utils/dateFormat.ts`                                  | **Existiert**, aber nur im Diktatpfad genutzt: `formatDateTime`/`formatDate`/`formatRelativeTime` (Intl.DateTimeFormat bzw. Intl.RelativeTimeFormat, Locale-Parameter, Unix-Sekunden-String-Eingabe). Einziger Import: `HistorySettings.tsx` Zeile 14. Keine Meeting-Nutzung.                                                                                               |
| Bestehende Größenformatierung              | Lesezugriff `src/lib/utils/format.ts`                                  | **Existiert:** `formatModelSize` (Teiler 1024, Labels `MB`/`GB`, Intl.NumberFormat, `"Unknown size"`-Sonderfall). Modellbezogen, nicht meeting-tauglich (Input MB-Zahl statt Bytes).                                                                                                                                                                                        |
| Bereits existierende ad-hoc-Meetingdauer   | Lesezugriff `src/components/shell/productNav.ts` Zeilen 70–83          | **Existiert:** `formatMeetingDuration(totalSeconds)` → `mm:ss` unter einer Stunde, sonst `hh:mm:ss` (beide Felder zero-padded); Clamp auf 0 via `Math.max`. Genau diese Funktion ist der Seed für den Seam (§5.2); sie wandert bei Implementierung dorthin (§4.3) — wobei ihr Unter-Stunden-Zweig gemäß §2 an den Plan (`00:42:18`) angeglichen wird.                       |
| Weitere verstreute Formatierung im Bestand | grep `padStart\|getMinutes` über `src/**`                              | Overlay-Diktattimer (`src/overlay/RecordingOverlay.tsx` Zeile 170), AudioPlayer-Position (`src/components/ui/AudioPlayer.tsx` Zeile 263), Debug-Logzeit (`src/components/settings/debug/LiveLogViewer.tsx` Zeile 70–71). Alles Diktat-/Debug-Pfad — bleibt unangetastet (NG-6), belegt aber das Muster „jede Oberfläche baut sich ihr Format selbst“, das der Seam beendet. |
| Locale-Bestimmung der App                  | Lesezugriff `src/i18n/index.ts`                                        | i18next mit `lng` aus Settings bzw. Systemlocale (`getSupportedLanguage`, 24 Locale-Ordner); `i18n.on("languageChanged")` setzt `<html dir>`/`<html lang>`. Folgerung: der App-Locale für Formatter ist **`i18n.language`**, nicht `navigator.language` und nicht `undefined` (das würde die OS-Locale statt der App-Sprache nehmen).                                       |
| RTL-Infrastruktur                          | `src/lib/utils/rtl.ts`, `src/i18n/index.ts` Zeilen 127–132             | Existiert (`isRTLLanguage`, `getLanguageDirection`); RTL-Sprachen u. a. `ar`, `he`.                                                                                                                                                                                                                                                                                         |
| Zeitzonenhaltung im Datenmodell            | Plan §9 (Zeilen 1168–1178: `started_at`, `ended_at` ohne Offsetspalte) | Der Plan speichert keine UTC-Offset-Spalte. Dieser Vertrag legt die Offset-Erfassung als additive Session-Metadaten fest (§3.1) — eine Schemaergänzung, kein Umbau.                                                                                                                                                                                                         |

**Folgerung:** Es gibt heute keinen gemeinsamen Formatter für Meetingflächen
und keine dokumentierte Zeitzonen-/Export-Sprachregel. Beides fixiert dieser
Vertrag, ohne bestehenden Diktatpfad-Code anzufassen.

## 2. Autorität und Notations-Adjudikation

Vier bestehende Dokumente notieren Dauerformate leicht abweichend. Die
Auflösung — zwei bewusst getrennte Formatklassen, jede mit genau einem
kanonischen Muster:

| Klasse                                                          | Kanonisch                                                                                               | Quellen, die damit wörtlich übereinstimmen                                                                                                           | Abweichungen, die hier adjudiziert werden                                                                                                                                                                                                                              |
| --------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **A: Statusuhr** (Tray, Statusleiste, Nav-Pill, Detailkopf)     | **immer `HH:MM:SS`** (auch unter einer Stunde), Stundenfeld unbegrenzt wachsend                         | Plan Zeile 427 (`Aufnahme läuft · 00:42:18` — Stundenfeld auch bei 42 min); I18N §7.2 (`H:MM:SS`/`HH:MM:SS`); SPD Zeile 80 (Stunden nicht modulo 24) | Der produktive Seed `productNav.ts` lässt das Stundenfeld unter 1 h weg (`mm:ss`) — das widerspricht dem Plan-Beispiel; Plan schlägt Code-Seed, also wird der Seed bei der Seam-Migration (§4.3) auf always-`HH:MM:SS` korrigiert. Keine `MM:SS`-Variante in Klasse A. |
| **B: Belegmarker** (Transkript-, Export-, Clipboard-Zeitmarker) | `mm:ss` unter 1 h (Minuten gepolstert), sonst `h:mm:ss` (Stunden ungepolstert); Truncation, nie Rundung | EPD §4.2 (`[mm:ss]` / `[h:mm:ss]`), EPD §6 (Clipboard); SPD Zeile 80 (truncating division only)                                                      | SPD nutzt durchgehend `HH:MM:SS` für die Zusammenfassungs-Achse — das bleibt dort gültig (Maschinen-/Belegachsenformat); Nutzerflächen-Marker folgen Klasse B.                                                                                                         |

Keine dritte Klasse wird eingeführt. Countdown-Sekunden („Aufnahme beginnt in
{{seconds}} s“) sind kein Dauerformat, sondern ein i18n-Parameter gemäß
I18N §7.1 und gehören nicht in diesen Seam.

## 3. Zeitzonen-Entscheidung (G1)

Genau **eine** Regel je Fläche, abschließend entschieden:

### 3.1 Speicherseite (Voraussetzung, nicht eigene Anzeigeregel)

- Alle Instants werden maschinenlesbar als RFC-3339-Utc gespeichert
  (`started_at`, `ended_at`, `occurred_at`; IF §6.2 Regel 2 bereits
  normativ).
- Jede `meeting_sessions`-Zeile erhält **eine additive Metadatenspalte**
  `utc_offset_minutes INTEGER NOT NULL` — Snapshot des Offsets des
  Aufnahmegeräts zum Startzeitpunkt (z. B. +120 für Sommerzeit-Mitteleuropa).
- Mehrere Offsets in einer Session (DST-Wechsel mitten im Meeting, Reisen)
  werden in v1 **nicht** modelliert: Der Startoffset gilt für die gesamte
  Session. Diese Einschränkung ist dokumentierte Vereinfachung (§11 NG-5);
  sie betrifft nur die Exportdarstellung, nie die Anzeige.

### 3.2 Die drei Regeln (G1-Matrix)

| #   | Fläche                                                               | Regel (genau eine)                                                                                                                                                                                                                                                                                     | Begründung                                                                                                                                                                                                                                            |
| --- | -------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| R1  | **Anzeige** (Listen, Detailansichten, Live-Ansicht, relative Zeiten) | Viewer-Zone: Konvertierung in die OS-Zeitzone des Betrachters via Intl-Default (`Intl.DateTimeFormat(locale)` ohne explizite `timeZone`); der aufgezeichnete Offset wird **nie** für die Anzeige benutzt.                                                                                              | Ein Nutzer öffnet Meetings immer dort, wo er sitzt; die Frage „wann war das für mich?“ beantwortet die lokale Zone. Der Aufnahme-Offset hat nur in Exporten Bedeutung.                                                                                |
| R2  | **Export-Dateinamen**                                                | Ausschließlich UTC, festes Ziffernmuster aus EPD §4.3 (`<yyyymmdd-hhmmss>`, exportzeitpunkt), ASCII-Ziffern, niemals lokalisiert, niemals Viewer-Zone, niemals Session-Offset.                                                                                                                         | Dateinamen sind Dateisystem-/Plattformgrenzen-Identifikatoren: sortierbar, kollisionsarm und gebietsneutral. EPD lässt die Zone bislang offen — hiermit: UTC.                                                                                         |
| R3  | **Export-Inhalt**                                                    | Session-Offset: alle menschenlesbaren Zeitangaben im Exportkörper werden im `utc_offset_minutes`-Wert der Session gerendert, mit explizitem Suffix `(UTC+02:00)` bzw. `(UTC−05:30)`; Maschinenfelder (JSON `started_at`/`ended_at`, Provenance `generated_at_utc`) bleiben rohes RFC-3339-Utc mit `Z`. | Ein exportiertes Protokoll ist ein Artefakt über das Meeting, nicht über den Exporteur: Teilnehmer-bezogene Zeiten bleiben stabil, egal wer aus welcher Zone exportiert. Das Suffix macht die Zone selbsterklärend; Maschinenfelder bleiben parsebar. |

Konsequenz-Tabellen für Grenzfälle:

| Fall                                                            | R1 Anzeige                           | R2 Dateiname                | R3 Inhalt                                                                                          |
| --------------------------------------------------------------- | ------------------------------------ | --------------------------- | -------------------------------------------------------------------------------------------------- |
| Nutzer fliegt nach der Aufnahme in eine andere Zone             | zeigt neue lokale Zone               | unverändert UTC             | unverändert Session-Offset                                                                         |
| Zwei Nutzer in verschiedenen Zonen exportieren dasselbe Meeting | jeweils lokal                        | identisch (UTC, Exportzeit) | byte-identische Zeitangaben                                                                        |
| DST-Übergang während der Session                                | Intl konvertiert korrekt pro Instanz | unverändert                 | Startoffset gilt für alles (§3.1)                                                                  |
| Session ohne Offsetwert (Legacy-/Importfall)                    | normal                               | unverändert                 | Fallback UTC + Suffix `(UTC+00:00)`, Warnung im Provenance-Envelope (`timezone_assumption: "utc"`) |

Die Zeitzonenermittlung geschieht ausschließlich über die Intl-Bibliothek.
Es wird niemals manuell `getTimezoneOffset()`-Arithmetik auf Bestandteile
angerechnet und die Zone nie aus Titel, Provider oder Dateinamen abgeleitet.

## 4. Der Formatter-Seam (G3)

### 4.1 Genau ein Modul

Kanonischer Pfad (neu, Phase 4): **`src/lib/utils/meetingFormat.ts`**

- Konventionell neben den bestehenden Utils (`src/lib/utils/rtl.ts`,
  `format.ts`); MPM §4 listet nur Komponenten, daher ergänzt dieser Vertrag
  die Platzierung nach derselben Logik wie MPM §5 F1 (`@/`-Alias-Import).
- Alle Meetingflächen (F5–F10, F12–F18, Tray-Vorbereitung, Exportvorschau)
  importieren ausschließlich von dort. Eigenmächtige `padStart`-/`getHours`-
  Datumsarithmetik oder zweite `Intl.*`-Aufrufe mit Formatcharakter in
  Meetingkomponenten sind Review-Defect.
- Ausnahmen (bewusst, mit Begründung): reine String-Weiterreichen ohne
  Formatentscheidung (Backend liefert fertigen Text), und die
  Diktatpfad-Formatter `dateFormat.ts`/`format.ts`/Overlay/AudioPlayer
  (§1, NG-6) — sie werden nicht migriert.

### 4.2 Geschlossene API-Oberfläche

```ts
// Statusuhr (Klasse A) — Seed: productNav.formatMeetingDuration (§1, §2:
// Unter-Stunden-Zweig wird auf always-HH:MM:SS korrigiert)
formatMeetingDuration(totalSeconds: number): string;

// Belegmarker (Klasse B) — EPD §4.2
formatTimelineMarker(startMs: number, endMs?: number): string;

// Absoluter Zeitpunkt, Viewer-Zone (Regel R1); style: "datetime" | "date" | "time"
formatMeetingInstant(isoUtc: string, locale: string, style: InstantStyle): string;

// Relativzeit ("vor 2 Stunden"), Viewer-Zone, Schwellenlogik §6.3
formatRelativeMeetingTime(isoUtc: string, locale: string, now?: Date): string;

// Bytegröße §7 (App-Locale für die Zahl, Einheit angehängt)
formatMeetingByteSize(bytes: number | null, locale: string): string;

// App-Locale-Accessor: getrimmter BCP-47-Tag aus i18n.language
currentLocale(): string;
```

Bindende Eigenschaften:

1. **Reine Funktionen, keine React-Abhängigkeit** — testbar ohne Renderer;
   `now` ist injizierbar (letzter Parameter), das Modul besitzt keine Uhr.
2. **Jede Funktion nimmt `locale` explizit** (wie `dateFormat.ts`);
   Komponenten übergeben `currentLocale()` (= `i18n.language`). Keine
   Formatter-Funktion liest `navigator.language`.
3. **Uhr-/Markerformate sind ASCII-latn-Ziffern** (handgebaut, kein
   NumberFormat) — bidi-stabil und alignment-fest (I18N §7.2/§10.2).
   Nur `formatMeetingByteSize` nutzt `Intl.NumberFormat(locale)` für den
   Zahlenanteil (lokale Ziffernsysteme erlaubt, da eigenständiges Token
   mit Einheit).
4. **Null/unknown ist kein Formatter-Fall:** Eine noch unbekannte Dauer
   (Meeting läuft noch / Verarbeitung offen) rendert die aufrufende
   Komponente über einen i18n-Copy-Placeholder (`—`-Konvention im
   UX-Copy-Katalog), **nie** über den Seam. Der Seam bekommt nur echte
   Zahlen. Damit sind „unbekannt“ und „genuine 0“ unterscheidbar (§5.4).
5. **Kein Markup-Output:** Alle Funktionen liefern plain strings; Bidi-
   Isolation (§9) ist Aufgabe der Komponente.

### 4.3 Migration des Seeds

`productNav.formatMeetingDuration` (Zeilen 74–83) wird bei Landung des Seams
(Phase 4) dorthin verschoben; `productNav.ts` behält keinen zweiten Abdruck
(MPM E5-Geist: genau eine Definition). Die Verschiebung enthält genau eine
Verhaltenskorrektur gemäß §2: der Unter-Stunden-Zweig gibt das Stundenfeld
aus (`00:42:18` statt `42:18`). Bis dahin gilt der Seed als
referenzimplementierungstreu abzüglich dieser dokumentierten Abweichung;
dieser Vertrag ändert keinen Code.

## 5. Dauerformate (G2)

### 5.1 Klassen und Muster (siehe §2)

Klasse A Statusuhr: immer `HH:MM:SS`. Klasse B Marker: `mm:ss` /
`h:mm:ss`, Range `start–end` (EN-DASH, EPD §4.2). Truncating division only
(SPD Zeile 80); Subsekunden werden nie angezeigt (NG-7).

### 5.2 Exakte Algorithmusfixierung Klasse A

```text
Eingabe totalSeconds (number):
  non-finite (NaN/±∞)  -> "00:00:00"
  < 0                  -> clamp auf 0 (Defect-Log im Debug-Modus)
  sonst                 -> floor auf ganze Sekunden
h = floor(t / 3600)          // unbegrenzt: 0, 24, 100, … nie modulo (SPD Zeile 80)
m = floor((t % 3600) / 60)
s = t % 60
Ergebnis IMMER dreifeldig: pad(h) + ":" + pad(m) + ":" + pad(s)   // HH:MM:SS
```

Das Stundenfeld erscheint auch bei Dauern unter einer Stunde (`00:42:18`,
Plan Zeile 427) — feste Feldbreite macht Uhren spaltenstabil und von
Belegmarkern (Klasse B) auf einen Blick unterscheidbar.

### 5.3 >24-h-Kante (Mehrtagessessions, G2-Pflicht)

Das Stundenfeld wächst unbegrenzt und bleibt rein numerisch:

| Dauer           | Klasse A (Uhr) | Klasse B (Marker) |
| --------------- | -------------- | ----------------- |
| 26 h 15 min 9 s | `26:15:09`     | `26:15:09`        |
| 100 h 0 min 5 s | `100:00:05`    | `100:00:05`       |
| 3 Tage 2 min    | `72:02:00`     | `72:02:00`        |

Explizit verboten: Tagesumbruch-Darstellungen (`1d 2:15`), modulo-24-Stunden
(`02:15:09` bei 26 h), Lokalisierung des Stundenfelds. Begründung: Die
Session-Timeline ist streng monoton (Plan §7); jede Kalender-artige
Zerlegung bricht Monotonie der Sortier-/Sprunglesemöglichkeit und erzeugt
Pluralprobleme (I18N §7.2). Mehrtagessessions sind real (vergessene
Stopptaste nach Recovery), also Pflichtfall, kein Theoriefall.

### 5.4 Nulllängen-Kante (G2-Pflicht)

Zwei Fälle, strikt getrennt:

| Fall                  | Quelle                                                                                                | Darstellung                                                                                                                                                                                                |
| --------------------- | ----------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Unbekannte Dauer**  | `ended_at IS NULL` oder Dauer noch nicht berechnet                                                    | i18n-Placeholder der Komponente (Copy-Katalog-Konvention `—`); Formatter wird nicht aufgerufen (§4.2 Regel 4). Beispiel: Meeting in Verarbeitung in der Liste.                                             |
| **Genuine Nulllänge** | `ended_at - started_at = 0` (Start sofort gestoppt, beide Timestamps persistiert, gültiger Datensatz) | `00:00:00` (Klasse A) bzw. `00:00`–Marker (Klasse B). Nie leeres Feld, nie `—` — eine existierende Session mit null Sekunden ist ein echter Wert und muss von „noch nicht bekannt“ unterscheidbar bleiben. |

Negative berechnete Dauern (Uhrverschiebung zwischen started/ended) werden
auf 0 geklemmt und im Debug-Log als Datendefect markiert; sie erscheinen nie
negativ.

## 6. Zeitpunkte und relative Zeiten

### 6.1 Absolute Zeitpunkte (Regel R1)

`formatMeetingInstant` mappt `style` auf genau eine Intl-Kombination:

| style      | Intl-Optionen                           | Einsatzfläche                                                                            |
| ---------- | --------------------------------------- | ---------------------------------------------------------------------------------------- |
| `datetime` | `dateStyle:"medium", timeStyle:"short"` | Detailkopf, Processing-Ansicht                                                           |
| `date`     | `dateStyle:"medium"`                    | Listenzeilen jenseits des Relativfensters, Projekt-Quellenliste                          |
| `time`     | `timeStyle:"medium"`                    | Live-Transkriptsegment-Zeitstempel (mit Sekunden, da Segmente sekundengenau belegt sind) |

Keine weiteren Styles ohne Vertrags-Amendment; keine `weekday`/`era`-
Varianten (Komplexitätsbudget).

### 6.2 Relative Zeiten („vor 2 Stunden“)

- Mechanik: `Intl.RelativeTimeFormat(locale, { numeric: "auto" })`, Stufen-
  tabelle nach dem Muster des bestehenden `formatRelativeTime` (dateFormat.ts
  Zeilen 80–122: s → min → h → d → w → mo → y). Der Seam reimplementiert die
  Stufenlogik für Millisekunden-Input (der Bestand nimmt Unix-Sekunden-
  Strings — ein Diktatpfad-Artefakt, das nicht übernommen wird). Wegen der
  7-Tage-Schwelle sind für Meetingflächen faktisch nur die Stufen Sekunde,
  Minute, Stunde und Tag erreichbar; die tieferen Stufen des Bestands
  (Woche/Monat/Jahr) werden nicht übernommen.
- Einsatzschwelle: Relativzeit nur bis **7 Tagen**; darüber absolut
  (`date`-Style). Begründung: „vor 3 Monaten“ ist für Bibliotheksarbeit
  weniger nützlich als ein Datum und driftet stilistisch stark je Locale.
- Flächen: Meetingliste (Datumsspalte), Projekte-Home („letzte
  Aktualisierung“), Eingang, Aufgabenlisten. Nicht im Detailkopf (dort immer
  absolut — ein geöffnetes Detail braucht den exakten Zeitpunkt).
- Aktualisierungskadenz: Neuberechnung bei Render plus einem 60-s-Intervall
  solange die Liste sichtbar ist; kein Sekunden-Ticker. Einzige
  Ausnahme mit schnellerer Uhr bleibt die NAV-rAF-Pille der laufenden
  Aufnahme (Klasse A, keine Relativzeit).

### 6.3 Zukunftseziten

Relative Formatter werden nur für Vergangenheitswerte eingesetzt. Ein
zukünftiges `occurred_at` (falscher Systemclock, importierte Daten) fällt
auf absolut zurück (`datetime`) — `Intl.RelativeTimeFormat`-Ausgaben wie
„in 2 Stunden“ haben in keiner Meetingfläche Bedeutung.

## 7. Bytegrößen

- **Eine Funktion** `formatMeetingByteSize(bytes, locale)`: Teiler 1024
  (Repo-Konvention `formatModelSize` Zeile 6), Staffel `B` → `KB` → `MB` →
  `GB`; eine Dezimalstelle unter 10 des Skalenwerts, keine Dezimalstellen
  ab 10 (identische Rundungsregel wie `formatModelSize` Zeilen 8–19);
  Zahlenanteil via `Intl.NumberFormat(locale)`, Einheit als ASCII-Label
  angehängt.
- Eingaben < 1024 rendern als `<N> B`; negative/nicht-finite Eingaben sind
  Defect → `"0 B"` + Debug-Log.
- **`null` ist unbekannt**, kein Formatter-Fall: Aufrufer rendert
  i18n-Placeholder (wie §4.2 Regel 4). Bewusst anders als
  `formatModelSize`, das `"Unknown size"` hart codiert zurückgibt — genau
  solche hardcoded Strings verbietet AGENTS.md/i18next für UI-Texte; der
  Meeting-Seam wiederholt diesen Fehler nicht.
- Flächen: Chunk-/Assetgrößen im Audio-Tab, Speicherhinweis im Startdialog
  (Plan §12: ~230 MB/h dual-track), Exportmanifest-Anzeige.
- **Exportinhalt enthält nie formatierte Größen:** Manifeste und JSON
  tragen Integer-`bytes`-Felder (EPD §4.4); Formatierung ist reine
  Anzeigenaht. TXT/Markdown-Exporte nennen Größen gar nicht.

## 8. Export-Sprachregel (Entscheidung)

**Entschieden: Der Export folgt der Session-Sprache, nicht der UI-Locale.**

Präzisierung:

1. `export_language = primary_subtag(meeting_sessions.language)` (BCP-47);
   leer/unbekannt → `en`.
2. Alle vom Serializer erzeugten Rahmenwörter (TXT-Headerzeilen `Titel`,
   `Datum`, `Dauer`, `Quelle-ID`, `Export-Version` laut EPD §4.2; Markdown-
   Gliederungstitel laut Plan §10 Standardoutline) stammen aus einer
   **geschlossenen, versionsbasierten Label-Tabelle**
   `EXPORT_LABELS[export_language]` im Exportdienst. v1-Einträge: `en`
   (Fallback, vollständig) und `de` (primäre Entwicklersprache, I18N §8.2).
   Jede weitere Sprache ist ein Tabellen-Eintrag, kein Codepfad.
3. **Inhaltsprache unverändert:** Transkripttext und Summary-Prosa bleiben
   exakt so, wie STT/LLM sie produziert haben; der Export übersetzt nichts.
4. Begründungen gegen die Alternative „UI-Locale“:
   - Der Serializer lebt im Backend (`meeting_export.rs`, EPD §2) außerhalb
     jedes i18next-Kontexts; UI-Locale müsste erst als neuer Parameter über
     Commands gereicht werden — zusätzliche Grenzfläche ohne Nutzwert.
   - Exporte sind Archiv-/Teile-Artefakte: Re-Export unter geänderter
     UI-Sprache würde byte-anders aussehen und die Reproduzierbarkeits-
     Absicht des Provenance-`content_hash` (EPD §5) unterlaufen.
   - Die Session-Sprache ist pro Meeting deterministisch persistiert.
5. Zahlen, Zeitmarker (`[mm:ss]`), Zeitangaben (R3) und `bytes`-Felder in
   Exporten sind **vollständig locale-invariant** (ASCII, §3.2/§7) — die
   Label-Tabelle ist der einzige sprachabhängige Anteil.
6. Adjudikation zu EPD §4.2: Die dort notierten deutschen Headerliterale
   (`Titel`, `Datum`, …) sind Beispieleinstanz der Tabelle (`de`), keine
   Festlegung auf Deutsch; der Tabellenmechanismus hier ist normativ.

Clipboard-Pfade (EPD §6) folgen derselben Regel — sie nutzen dieselben
Serializer-Stufen.

## 9. RTL/Bidi-Verhalten

1. **Clocks und Marker sind bidi-neutral:** ASCII-Ziffern + `:` erzeugen in
   ar/he-kontexten keine Richtungswechsel, können aber als schwach
   richtungs Token falsch einordnen. Verbindlich: Komponenten wickeln
   formatter Outputs, die in RTL-Satz eingebettet werden, in
   `<span dir="ltr">…</span>` bzw. `<bdi>` — Fortsetzung der
   I18N §10.2-Pflicht, jetzt ausdrücklich auch für Zeit-/Größentoken.
2. **Intl-Outputs sind eigenständig lokalisiert:** `RelativeTimeFormat`/
   `DateTimeFormat`-Ergebnisse („vor 2 Stunden“, „قبل ساعتين“) sind
   vollständige locale-correcte Strings und brauchen keine Isolation.
3. **Kein Richtungs-Glyph in formatter Output:** Der Seam emittiert nie
   Pfeile oder Richtungswörter (I18N §10.1).
4. **Tray:** bekommt ausschließlich `{{duration}}` der Klasse A
   (ziffernbasiert, I18N §10.2); Gerätenamen/Titel bleiben verboten.

## 10. Enforcement und Testpflichten

| Prüfung                                                                        | Mechanismus                                                                                                     | Status                                                  |
| ------------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------- |
| Import-Disciplin (kein zweites Formatieren in Meetingflächen)                  | Review-Pflicht gegen §4.1; ESLint kann dies nicht ausdrücken                                                    | Pflicht ab Phase 4                                      |
| Klassen-A-/B-Algorithmen inkl. >24 h, 0, negativ, NaN, Minuten-/Stundenpolster | Unit-Tests über `meetingFormat.ts` (reine Funktionen, §4.2 Regel 1)                                             | Vorgeschrieben bei Implementierung; Referenzfälle in §5 |
| Zeitzonenregeln R1/R2/R3                                                       | Unit-Tests mit fixer `timeZone`-Injection (R1 via Intl-Options-Override im Test) und fixem `utc_offset_minutes` | Vorgeschrieben vor Phase-7-Export                       |
| Export-Sprachtabelle Vollständigkeit (`de` ⊆ `en`-Schlüsselmenge)              | Erweiterung des Label-Tests im Exportdienst                                                                     | Vorgeschrieben vor Phase 7                              |
| Relativzeit-Schwellen (7-Tage-Grenze, Zukunftsfall)                            | Unit-Tests mit injiziertem `now`                                                                                | Vorgeschrieben bei Phase 4                              |

Sketch-Verifikation der Algorithmusfixierungen §5: ausgeführt mit Node v24
(`node`, temporäres Skript mit den §5.2/§5.1-Regeln, 14/14 Fälle bestanden):
Klasse A: 0 s → `00:00:00`; −5 → `00:00:00` (Clamp); NaN und ±∞ →
`00:00:00`; 2549 s → `00:42:29`; 2520 s (Plan-Pill) → `00:42:18`; 3600 s →
`01:00:00`; 94309 s → `26:11:49`; 360005 s → `100:00:05`; 259320 s →
`72:02:00`. Klasse B: 0 ms → `00:00`; 245000 ms → `04:05`; 3600000 ms →
`1:00:00`; 94309000 ms → `26:11:49`. Die Skizze bestätigt, dass die
Fixierung implementierbar und widerspruchsfrei ist; sie ist keine
implementierte Behavior-Aussage über den Bestand.

## 11. Nicht-Ziele und Komplexitätsbudget

- **NG-1:** Keine Datumsbibliothek (date-fns, dayjs, luxon, Temporal-Polyfill)
  — ausschließlich `Intl.*`.
- **NG-2:** Keine nicht-gregorianischen Kalender, keine `calendar:`-Optionen.
- **NG-3:** Kein lokales Pinning von Ziffernsystemen für Bytegrößen
  (`numberingSystem`-Override); Clocks sind ohnehin handgebaut ASCII (§4.2
  Regel 3).
- **NG-4:** Kein ICU-MessageFormat und keine zweite i18n-Infrastruktur
  (kongruent zu I18N NG-7).
- **NG-5:** Kein Multi-Offset-Modell für DST-/Reise-Sessions und keine
  Zeitzonen-Auswahl-/Override-Einstellung pro Oberfläche (§3.1).
- **NG-6:** Keine Migration der Diktatpfad-Formatter (`dateFormat.ts`,
  `format.ts`, Overlay-Timer, AudioPlayer, LiveLogViewer) in den Seam — der
  Diktierpfad bleibt unangetastet (Plan Phase-2-Abnahme).
- **NG-7:** Keine Subsekundenanzeige, keine Millisekunden-Marker in
  Nutzerflächen (Persistenz bleibt ms-genau).
- **NG-8:** Keine Übersetzung von Transkript-/Summary-Inhalten im Export
  (§8 Regel 3) und keine weiteren Exportlabel-Sprachen über en/de hinaus in
  v1.
- **NG-9:** Kein automatisches Refresh-Framework für Relativzeiten über die
  60-s-Kadenz hinaus; kein Sekunden-Ticker außer der NAV-Pille.

## 12. Gate-Selbstprüfung

| Gate                                                                                                      | Nachweis                                                                                                                                       |
| --------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------- |
| G1 — Zeitzonenentscheidung explizit für Anzeige, Export-Dateinamen und Export-Inhalt, je genau eine Regel | §3.2 Matrix R1/R2/R3 mit Grenzfalltafel; R2 adjudiziert die offene Zone in EPD §4.3, R3 fixiert Session-Offset + Suffix.                       |
| G2 — Dauerformat behandelt >24-h- und Nulllängen-Kante                                                    | §5.2 Algorithmus, §5.3 Tabelle (26 h/100 h/72 h), §5.4 Falltabelle unbekannt vs. genuine 0 inkl. Negativ-Clamp; Sketch-Verifikation §10.       |
| G3 — Formatter-Seam ist ein einzelnes benanntes Modul                                                     | §4.1 kanonischer Pfad `src/lib/utils/meetingFormat.ts`, geschlossene API §4.2, Import-Disciplin und Seed-Migration §4.3.                       |
| G4 — Nicht-Ziele-Abschnitt; Dokumentationsindex aktualisiert                                              | §11 (NG-1–NG-9); Tabellenzeile + Lesereihenfolge-Eintrag Nr. 23 + Kurzregel in `docs/meeting-mode-documentation-index.md` im selben Changeset. |
