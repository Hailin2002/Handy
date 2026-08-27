# i18n-Namespace- und Locale-Resource-Vertrag — Meeting-Modus (Meeting & Projekte)

Status: **spec contract — keine Implementierung vorhanden; ändert keinen Quellcode.**
Dieser Vertrag fixiert den Key-Namespace der Meeting-/Projekt-UI in
`src/i18n/locales/en/translation.json` und die Konventionen für Interpolation,
Pluralisierung, Locale-Parität, Umbenennung, RTL-Sicherheit und Extraktion.

Plan-Anker: [docs/meeting-mode-plan.md](../meeting-mode-plan.md) — §5 (Zeilen
335–337: „Alle sichtbaren Texte werden zuerst in der englischen i18n-Quelldatei
angelegt … Es gibt keine hart codierten JSX-Texte“), §6.2 Regel 8, §6.16–§6.18
(Projekte-Seite, Detailansicht, Zuordnung), Phase-8/10-Abnahme („alle neuen
sichtbaren Texte verwenden i18n“, Zeile 1578).

Autoritätsordnung (wie im Dokumentationsindex): Plan schlägt Verträge,
Verträge schlagen Kataloge, nichts schlägt den tatsächlichen Code. Wo dieser
Vertrag Key-**Pfade** festlegt, bleibt der UX-Copy-Katalog Autorität für die
**Texte**; wo der Katalog einen Pfad abweichend von diesem Vertrag notiert,
gilt §4.2 (Normalisierungsregeln), nicht eine stille Abweichung.

Verwandte Dokumente:
[UX-Copy-Katalog](../meeting-mode-ux-copy-catalog.md) (COPY, `proposal-only`,
Autorität aller Textentwürfe),
[Error-Taxonomie](error-taxonomy-contract.md) (TAX, §6 NEW-Copy-Key-Familien),
[Dokumentationsindex](../meeting-mode-documentation-index.md).

Arbeitsstand geprüft am 22.08.2026 im Worktree dieses Tasks.

---

## 1. Verifizierungsstand gegen den Code (ehrliche Kennzeichnung)

Alle Aussagen dieses Vertrags über bestehende Infrastruktur wurden direkt im
Code geprüft:

| Prüfung                                | Methode                                                                   | Ergebnis                                                                                                                                                                                                                                                                                                                                                  |
| -------------------------------------- | ------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| ESLint-Hardcoded-String-Regel          | Lesezugriff `eslint.config.js`                                            | **Existiert.** Regel `i18next/no-literal-string` (Plugin `eslint-plugin-i18next`, Zeilen 1, 16, 20–35), Modus `markupOnly: true`, `ignoreAttribute`: `className`, `style`, `type`, `id`, `name`, `key`, `data-*`, `aria-*`. Es gibt keine Regel namens `no-hardcoded-string`; dieser Name ist die Umgangssprache aus AGENTS.md („ESLint enforces this“).  |
| Paritätsprüfung der Locales            | Lesezugriff `scripts/check-translations.ts`                               | **Existiert.** Referenzsprache `en`; prüft fehlende **und** zusätzliche Leaf-Keys je Sprache (`missing`/`extra`, Zeilen 124–133), Exit-Code 1 bei Abweichung (Zeile 211). Aufruf über `bun run check:translations` = `node scripts/check-translations.ts` (package.json). Folgerung: **jeder** Key-Must in **allen** Sprachordnern gleichzeitig ankommen. |
| Locale-Ordner                          | `Get-ChildItem src/i18n/locales -Directory`                               | Genau 24 Ordner: ar, bg, cs, da, de, en, es, fr, he, hi, it, ja, ko, ne, nl, pl, pt, ru, sv, tr, uk, vi, zh, zh-TW.                                                                                                                                                                                                                                       |
| Top-Level-Abschnitte der en-Quelldatei | PowerShell `ConvertFrom-Json` über `src/i18n/locales/en/translation.json` | Genau 16 Abschnitte: `tray`, `sidebar`, `nav`, `meetings`, `onboarding`, `modelSelector`, `settings`, `footer`, `whatsNew`, `common`, `accessibility`, `errors`, `appLanguage`, `theme`, `overlay`, `secureInput`. **Ein Abschnitt `meetings` existiert bereits** (4 Keys: `emptyBenefit`, `newMeeting`, `importAudio`, `privacyLocal`) — siehe §2.2.     |
| Bestehende Plural-Konvention           | grep `_one\|_other` über `src/i18n/locales/en/translation.json`           | Genau eine Familie nutzt i18next-Plural-Suffixe: `accessibility.shortcutConflicts.blockedWithCulprit_one/_other`, `blockedNoCulprit_one/_other` (Zeilen 798–801) mit `{{count}}`.                                                                                                                                                                         |
| Plural-Suffixe in anderen Locales      | grep in `ru/translation.json`, `ar/translation.json`                      | Auch Russisch und Arabisch tragen **nur** `_one`/`_other` (keine `_few`/`_many`/`_zero`). Die Paritätsprüfung würde zusätzliche Suffixe sonst als `extra` zurückweisen.                                                                                                                                                                                   |
| Tray-i18n                              | `src-tauri/src/tray_i18n.rs` Zeilen 1–13                                  | TrayStrings wird **zur Build-Zeit von `build.rs` aus den Locale-Dateien generiert**; en/translation.json ist „single source of truth“. Neue Tray-Keys erfordern zusätzlich ein `TrayStrings`-Feld und Nutzung in `tray.rs`.                                                                                                                               |
| Meeting-i18n-Keys                      | grep `meeting` über `src/i18n/locales/en/translation.json`                | Nur der Azure-Beschreibungstext erwähnt „meetings“ im Diktierkontext. Kein einziger Key des COPY-Katalogs existiert.                                                                                                                                                                                                                                      |
| RTL-Infrastruktur                      | `src/lib/utils/rtl.ts`, `src/i18n/index.ts` Zeilen 127–132                | **Existiert.** `isRTLLanguage`/`getLanguageDirection` über `LANGUAGE_METADATA.direction`; `languageChanged` setzt `<html dir>`/`<html lang>`. RTL-Sprachen im Repo: u. a. `ar`, `he` (beide als Locale-Ordner vorhanden).                                                                                                                                 |

**Folgerung:** Dieser Vertrag legt nichts fest, was es schon gibt, sondern
normalisiert die Naht zwischen drei bestehenden Mechanismen (ESLint-Regel,
Paritätsskript, build.rs-Tray-Generierung) und dem geplanten Key-Bestand.

## 2. Namespace-Adjudikation

### 2.1 Neue Top-Level-Bäume

Für den Meeting-Modus werden genau **zwei neue Top-Level-Abschnitte** in
`translation.json` angelegt:

- `meeting` — alles, was eine einzelne Meeting-Session betrifft: Session-
  zustände, Verarbeitung, Fehler/Warnungen, Consent/Cloud/Retention/Löschung,
  Recovery, Aktionen (entspricht 1:1 dem COPY-Katalog, Abschnitte 2–6).
- `projects` — alles, was die Projekt-Domain betrifft: Projekte-Home,
  Projekterstellung, Projekt-Detail, Zuordnung von Meetings/Importen
  (Plan §6.16–§6.18). Der COPY-Katalog enthält hierfür noch keine Einträge;
  die Keys in §4.4 sind daher **Namespace-Festlegung mit Platzhaltercharakter**,
  nicht Textfreigabe — Wortlaut-Vorschläge gehen an den Katalog-Eigner.

Beide Bäume sind **flach beginnend, semantisch gruppiert**: maximal vier
Pfad-Ebenen (`meeting.errors.micLost.title` ist die Tiefengrenze; tiefe Pfade
bedürfen eines Vertrags-Amendments).

### 2.2 Kollision `meetings.*` (existiert) vs. `meeting.*` (neu)

Der vorhandene Abschnitt `meetings` (4 Keys, Empty-State der Meeting-Landingpage)
bleibt unverändert bestehen und behält seine Bedeutung:
**Landing-/Bibliotheksfläche**. Der neue Baum `meeting` (Singular) bedeutet
ausschließlich **Session-/Ablaufkontext** gemäß COPY. Die Regel:

| Baum                     | Bedeutung                                                 | Beispiele                                                       |
| ------------------------ | --------------------------------------------------------- | --------------------------------------------------------------- |
| `meetings.*` (bestehend) | Listen-, Landing- und Bibliothekstexte                    | `meetings.newMeeting`, `meetings.emptyBenefit`                  |
| `meeting.*` (neu)        | Ablauf einer Session: Zustände, Fehler, Consent, Löschung | `meeting.session.recording.title`, `meeting.actions.endMeeting` |
| `projects.*` (neu)       | Projekt-Domain                                            | `projects.home.title`, `projects.assign.toProject`              |

Kein Key darf zwischen den Bäumen umziehen, ohne die Umbenennungsregel §8 zu
durchlaufen. Begründung der Beibehaltung beider Bäume: `meetings.*` ist in
24 Locale-Dateien ausgeliefert; eine Zusammenlegung wäre eine reine Kosmetik-
Migration ohne Nutzwert und verletzt das Komplexitätsbudget (§12 NG-1).

### 2.3 Key-Grammatik (bindend für beide neuen Bäume)

1. Top-Level: Singular-Substantiv (`meeting`, `projects` als Domänenname).
2. Zweite Ebene: Funktionsgruppe in camelCase (`session`, `errors`, `warnings`,
   `cloud`, `consent`, `retention`, `stopConfirm`, `delete`, `conflicts`,
   `recovery`, `notify`, `processing`, `actions`; bei `projects`: `home`,
   `create`, `detail`, `assign`, `common`).
3. Blatt-Keys: camelCase, semantisch (**nie positional**: verboten sind
   `title2`, `bodyNew`, `text3`).
4. Rollen-Suffixe am Blatt nur für die in §5–§7 definierten Muster:
   Struktur-Suffixe (`title`, `body`, `label`, `hint`, `badge`, `item*`),
   Selektor-Suffixe (`body.mic`/`body.system`, `provider.local`/`provider.cloud`)
   und Plural-Suffixe (`_one`/`_other`, §6).
5. Reservierte Präfixe, die **nicht** unter `meeting`/`projects` liegen dürfen:
   bestehende Abschnitte der en-Datei (§1, Liste der 16) sowie `tray`
   (Tray-Erweiterungen bleiben im bestehenden `tray`-Abschnitt, weil
   `build.rs`/`TrayStrings` dort koppeln — COPY §2.2 und §8 Offener Punkt 2).
   Neu vorgeschlagene Tray-Keys: genau `tray.meetingRecording`,
   `tray.meetingPaused`, `tray.meetingProcessing`.

### 2.4 Fehlercode ↔ Copy-Key-Naht

TAX normiert bereits, dass die Grenze Frontend↔Backend nur `{code, params}`
überschreitet und jede Code-Zeile mindestens einen Copy-Key trägt. Dieser
Vertrag ergänzt die Pfadseite: alle in TAX §5 als „existing“ bzw. „NEW §6“
genannten Copy-Keys liegen unter `meeting.errors.*`, `meeting.warnings.*`
bzw. `meeting.recovery.*` wie in §3 aufgeführt. Das Unknown-Code-Fallback
`meeting.errors.generic.*` (TAX §2.3) ist Pflichtbestandteil der ersten
Key-Anlage — es darf nie fehlen, solange irgendein `meeting.errors.*`-Key
existiert.

## 3. Der `meeting`-Baum (1:1 zum COPY-Katalog)

Die folgende Baumskizze führt **jeden** im COPY-Katalog vorgeschlagenen Key
auf; Klammern nennen die Katalogquelle (Abschnitt/Entry). Textautorität bleibt
durchgängig COPY; diese Liste fixiert nur Pfade.

```text
meeting
├── actions                      # COPY §2.3 + inline vorgeschlagene Buttons
│   ├── tryAgain | switchDevice | openSettings | openSystemSettings
│   ├── reconnect | recordMicOnly | continueWithoutSystemAudio
│   ├── keepRecording | endMeeting | pause | resume | workInBackground
│   ├── retryFailedSegments | finishDictation | cancelMeetingStart
│   ├── openMeeting | deleteRecording | cancel | close | confirmDelete
│   ├── startMeeting                     # COPY S1
│   ├── testSystemAudio                  # COPY E4
│   ├── recover | decideLater            # COPY E12
│   ├── resumeWithDevices                # COPY E14
│   └── useLocal | useCloudSummary | disableSummary   # COPY P3/P5
├── session
│   ├── draft.hint                                   # S1
│   ├── starting.{title,body,countdown,escHint}      # S2
│   ├── recording.{title,saved,provider.local,provider.cloud}  # S3
│   ├── paused.{title,body}                          # S4
│   ├── stopping.{title,body,done}                   # S5
│   ├── micOnly.badge                                # E5
│   ├── completed.{title,saved}                      # S8
│   └── cancelled.{title,body}                       # S10
├── processing
│   ├── title | closeOk                              # S6
│   ├── step.saved                                   # S6
│   ├── step.prepared_one / step.prepared_other      # S6 (normalisiert, §6.4)
│   ├── step.transcribed_one / step.transcribed_other# S6 (normalisiert, §6.4)
│   ├── backlog_one / backlog_other                  # S6 (normalisiert, §6.4)
│   └── summarizing                                  # S7
├── errors                 # COPY E1,E2,E5,E6,E8,E9,E10,E13,E16,E17,S9 (+ TAX §6 NEW)
│   ├── generic.{title,body}                          # TAX §2.3 Fallback (Pflicht)
│   ├── recoverable.{title,detail}                    # S9
│   ├── micLost.{title,body}                          # E1
│   ├── systemLost.{title,body}                       # E2
│   ├── defaultChanged.{title,body,followed}          # E3
│   ├── noLoopback.{title,body}                       # E5
│   ├── exclusiveMode.{title,body}                    # E6
│   ├── diskFull.{title,body}                         # E8
│   ├── cloudUnreachable.{title,body,waiting}         # E9
│   ├── cloudConfig.{title,body}                      # E10
│   ├── slept.{title,body}                            # E13
│   ├── missingKey.{title,body.cloud,body.local,body.summary}  # E16
│   ├── micPermission.{title,body}                    # E17
│   └── … NEW-Familien aus TAX §6: deviceUnavailable.{title,body.mic,body.system},
│         storageWriteFailed.*, actionUnavailable.*, captureStoppedUnexpectedly.*,
│         consentRequired.*, segmentsFailed.{title,body}, audioPackageRejected.*,
│         languageUnsupported.*
├── warnings
│   ├── silence.{title,body}                          # E4
│   ├── diskLow.{title,body}                          # E7
│   ├── backlog.{title,body}                          # E11
│   ├── devicesChanged.{title,body}                   # E14
│   └── degradedCapture.*                             # TAX §6
├── conflicts.dictation.{title,body}                  # E15
├── recovery.{title,body}                             # E12
├── consent
│   ├── beforeStart.{title,body,acknowledge}          # P1
│   └── indicator.label                               # P2/S3
├── cloud
│   ├── notice.azure.{title,body,detail}              # P3
│   ├── notice.llm.{title,body}                       # P5
│   └── active.{azure,azureProcessing,llm}            # S3/S6/P4/P5
├── retention
│   ├── question.{title,body}                         # P6
│   └── option.{deleteNow,7days,30days,forever}       # P6 (Blattnamen bewusst
│                                                     #  wertstabil, nicht i18n-Inhalt)
├── stopConfirm.{title,body,timeout}                  # P7
├── notify.completed.{title,body}                     # S7 (notification-Surface)
└── delete
    ├── level1.{label,body}                           # P8
    ├── level2.{label,body}                           # P9
    └── level3.{label,body,itemAudio,itemTranscript,itemSummary,
                irreversible,confirm}                  # P10
tray                                                   # bestehender Abschnitt
└── meetingRecording | meetingPaused | meetingProcessing   # COPY S3/S4/S6
```

Anmerkungen mit Normcharakter:

1. **E12 (`meeting.recovery.*`)** steht absichtlich nicht unter `errors`:
   Crash-Recovery ist ein Angebots-, kein Fehlerzustand (COPY listet ihn in
   §5 E12; TAX mappt ihn kombiniert mit `recoverable.*`).
2. **`meeting.retention.option.7days`/`30days`**: Ziffern im Key-Namen sind
   hier erlaubt, weil sie den Policy-Wert identifizieren (Retention-Rule),
   nicht die Übersetzung. Ändert sich der Policy-Wert, gilt §8 (Rename).
3. **Tray-Keys** brauchen zusätzlich das `TrayStrings`-Feld in
   `src-tauri/src/tray_i18n.rs` (Build-Kopplung, §1); der ESLint-Extraktions-
   check (§11) greift für sie nicht — die Kopplung ist compile-time und daher
   selbstprüfend (fehlendes Feld = Buildfehler).

## 4. Der `projects`-Baum (Namespace-Festlegung, Wortlaut offen)

### 4.1 Status

Anders als §3 gibt der COPY-Katalog für Projekte **noch keine Texte** her.
Dieser Vertrag legt deshalb nur die Gruppenstruktur fest, damit spätere
Katalog-Einträge nicht gegen bereits ausgelieferte Keys entscheiden müssen.
Jeder Blatt-Key in §4.4 ist als _proposed namespace slot_ zu behandeln; die
erste Key-Anlage erfolgt gemeinsam mit Katalog-Einträgen (und damit mit allen
24 Locale-Dateien, §7).

### 4.2 Gruppierung

| Gruppe              | Inhalt                                                                                                             | Plan-Anker                                    |
| ------------------- | ------------------------------------------------------------------------------------------------------------------ | --------------------------------------------- |
| `projects.common.*` | wiederverwendete Projektbegriffe (`noProject`, `unassigned`, Projekt-Chip-Label)                                   | §6.7 Zeile 520, §6.18                         |
| `projects.home.*`   | Überschrift, Nutzensatz, Suche, Primäraktion `Projekt erstellen`                                                   | §6.16                                         |
| `projects.create.*` | Ein-Feld-Dialog (Name pflichtlich)                                                                                 | §6.16                                         |
| `projects.detail.*` | Kopfzeile, Kurzstatus, Tab-Struktur der Detailansicht                                                              | §6.17                                         |
| `projects.assign.*` | Zuordnung/Vorschlag: `Zuordnen`, `Anderes Projekt`, `Ohne Projekt lassen`, KI-Vorschlag mit Confidence/Bestätigung | §6.18, Phase 8                                |
| `projects.inbox.*`  | Eingang „Nicht zugeordnet“, Batch-Zuordnung                                                                        | §6.16/§6.18, Navigation-Seam `{kind:"inbox"}` |

### 4.3 Abgrenzungen

- Aufgaben-/Snapshot-Texte (`project_tasks`, Snapshots) gehören **nicht** nach
  `projects.*`, solange sie nur in Meeting-Detail-/Aufgabenflächen erscheinen;
  sie erhalten bei Bedarf eigene Gruppen unter `meeting.*` oder einen
  Amendment-Antrag an diesen Vertrag. Begründung: der Katalog deckt sie nicht,
  und eine vorzeitige Doppelstruktur verletzt §12 NG-2.
- Such- und Beleg-Flächen (K1–K5 laut Evidence-Navigation) rendern überwiegend
  Dateninhalte (Titel, Zitate) — die dafür nötigen **strukturbasierten
  Screenreader-Namen** sind eigene Keys unter der jeweiligen Flächengruppe,
  nicht unter `projects.common`.

### 4.4 Vorgeschlagene Slots (nicht textfreigegeben)

```text
projects
├── common.noProject | common.unassigned
├── home.{title,subtitle,searchPlaceholder,createProject}
├── create.{title,nameLabel,nameRequired}
├── detail.{statusLabel,lastUpdated,sourcesTab,tasksTab,meetingsTab}
├── assign.{toProject,differentProject,keepUnassigned,suggestionTitle,
│           suggestionAccept,suggestionReject}
└── inbox.{title,batchAssign}
```

## 5. Interpolation

Bindende Regeln für alle Keys unter `meeting.*` und `projects.*` (und
Richtlinie für den Rest der Datei):

1. Syntax: i18next-Standard `{{param}}`; Parameter camelCase; EscapeValue
   bleibt `false` (React escaped, `src/i18n/index.ts` Zeile 95) — deshalb
   dürfen Parameter **niemals** Markup enthalten.
2. Parameternamen sind Teil des Key-Vertrags: gleiches Key ⇒ gleiche Param-
   menge in allen Locales. Übersetzungen dürfen Parameter weglassen? Nein —
   jeder Parameter muss im übersetzten String vorkommen (Ausnahme §5.4).
3. Geräte-, Provider- und Titelnamen werden ausschließlich als Parameter
   übergeben (`{{device}}`, `{{provider}}`, `{{title}}`); Anführungszeichen
   gehören **zum übersetzten Text**, nicht zum Parameter (COPY E3/E6/P10
   folgen diesem Muster bereits: `“{{device}}”`). Begründung: Zitatzeichen
   sind sprachabhängig (de: „…“, ja: 「…」).
4. Optionale Parameter: ein Parameter, der in einem Locale entfällt, muss am
   Key-Namen verhandelt sein (`…Optional`-Suffix ist verboten; stattdessen
   zwei Keys). i18next-`defaultValue`-Fallbacks im Code sind für
   `meeting.*`/`projects.*` verboten — fehlender Key ist ein Defect, keine
   Fallback-Aufgabe (Parität ist ohnehin CI-erzwungen, §1).
5. Zahlen außerhalb von Pluralkontexten (`{{freeGb}}` in E7, `{{seconds}}`
   in S2/E9) werden als **vom Aufrufer formatierte Strings** übergeben
   (Intl.NumberFormat mit aktueller App-Locale); der Key formatiert keine
   Zahlen nach.
6. Zeitpunkte (`{{time}}` in E13) ebenso: vom Aufrufer über
   Intl.DateTimeFormat lokalisiert übergeben; der Key enthält nur Rahmenwortlaut.

## 6. Pluralisierung

### 6.1 Konvention

Es gilt die im Repository bereits etablierte i18next-Intl-Konvention
(§1, Zeilen 798–801): Schlüssel mit `{{count}}`-Parameter bekommen die
Suffix-Paare `_one`/`_other` und werden mit `t('key', { count })` aufgerufen.
i18next wählt das Suffix über die Intl-Pluralregeln der aktiven Sprache.

### 6.2 v1-Einschränkung: nur `_one`/`_other`

Für v1 sind **ausschließlich** `_one`/`_other` zulässig. Begründung: (a) en
und de — die beiden im Entwicklungsfluss maßgeblichen Sprachen (§7) — haben
genau diese beiden Kategorien; (b) alle 23 ausgelieferten Locales nutzen heute
nur diese Suffixe (§1), und die Paritätsprüfung stuft abweichende Suffixe als
`extra` zurück; (c) Sprachen mit mehr Kategorien (ru, ar, pl, uk …) fallen auf
`_other` zurück und sind ggf. stilistisch unvollkommen, aber nicht kaputt.
Die Erweiterung des Paritätsskripts um sprachspezifische Suffixmengen ist
ausdrücklich Nicht-Ziel (§12 NG-3).

### 6.3 Beispiele Segmentanzahl

en:

```json
"step.transcribed_one": "{{count}} of {{total}} segments transcribed",
"step.transcribed_other": "{{count}} of {{total}} segments transcribed"
```

de:

```json
"step.transcribed_one": "{{count}} von {{total}} Segmenten transkribiert",
"step.transcribed_other": "{{count}} von {{total}} Segmenten transkribiert"
```

(de unterscheidet hier formal `ein Segment von…` vs. `Segmenten`; die
`_one`-Variante trägt dem Rechnung, wo gewünscht. Ob Copy beider Varianten
identisch ist, entscheidet der Katalog-Eigner — die Struktur ist Pflicht.)

Außerdem normiert: `segmentsFailed.body` (TAX NEW) erhält bei Aufnahme ins
JSON zwingend die Paarform `…_one`/`…_other`, da `failedCount` ein
`{{count}}`-Parameter ist.

### 6.4 Normalisierung gegenüber COPY

Der Katalog notiert drei Keys halbseitig pluralisch:
`meeting.processing.step.prepared_one`, `step.transcribed_one` (ohne `_other`)
und `meeting.processing.backlog` (mit `{{count}}`, ohne Suffix). Nach §6.1
werden diese bei Anlage ins JSON zu vollständigen Paaren
(`prepared_one/prepared_other`, `transcribed_one/transcribed_other`,
`backlog_one/backlog_other`) normalisiert. Dies ist eine Pfadkorrektur gemäß
Autoritätsordnung (Vertrag vor Katalog), keine Textänderung; der Katalog-Eigner
soll die Notation übernehmen.

### 6.5 Verbotene Muster

- `t(key, {count})` gegen einen Key **ohne** Plural-Suffixe (Ausnahme: der
  count ist rein deklarativ, z. B. „{{count}} segments waiting“ als
  Zwischenstand — auch dies wird als Paar geführt).
- String-Verkettung im Code zur Pluralbildung (`"…" + n + " …"`), handgestrickte
  `if (n === 1)`-Zweige mit zwei getrennten Keys ohne `{{count}}`.
- `_zero`-Sonderkeys (en/de kennen die Kategorie nicht).

## 7. Dauern

Zwei Klassen, strikt getrennt:

### 7.1 Countdown-Sekunden (kurzlebig, < 60 s)

Muster wie COPY S2/E9: `meeting.session.starting.countdown` =
`Recording starts in {{seconds}} s`. Regeln:

- `{{seconds}}` ist eine ganze Zahl als String (§5.5).
- Die **Einheit gehört zum übersetzten Text** und darf je Sprache variieren
  (de: `Aufnahme beginnt in {{seconds}} Sekunden`). Verboten, die Einheit im
  Code an den String zu hängen.
- Kein Plural-Zwang: Sekunden-Countdowns laufen als Einzelstring; wer
  grammatikalisch sauber sein will, darf `_one/_other` führen, ist aber nicht
  dazu verpflichtet (Countdown-Werte ≥ 2 dominieren real).

### 7.2 Verstrichene Dauer (Tray, Statusleiste)

- Format: `H:MM:SS` bzw. `HH:MM:SS`, vom Aufrufer erzeugt (eine einzige
  Hilfsfunktion, nicht pro Komponente), als **ein** Parameter `{{duration}}`
  übergeben. Beispiel COPY S3: `tray.meetingRecording` =
  `Meeting: recording ({{duration}})` / de: `Meeting: Aufnahme ({{
duration}})`.
- Begründung: `:`-getrennte Zifferngruppen sind lokalneutral lesbar, vermeiden
  Pluralprobleme von „h/min/s“ vollständig und sind bidi-stabil (§10).
- Verboten: Dauer in mehrere Parameter zerlegen (`{{hours}}h {{minutes}}m`),
  da dies pro Sprache andere Grammatik und Reihenfolge erzwingt und im
  Rust-Tray (kein i18next) ohnehin nicht verfügbar wäre.

## 8. en-Quelldatei und Locale-Parität

1. **en ist die Quelldatei.** Jeder neue Key wird zuerst in
   `src/i18n/locales/en/translation.json` angelegt (Plan Zeile 335; AGENTS.md;
   CONTRIBUTING_TRANSLATIONS.md; tray_i18n.rs „single source of truth“).
2. **de-Parität im selben Changeset.** Mit jedem en-Key wird unmittelbar die
   deutsche Übersetzung gepflegt — de ist die primäre Entwicklungssprache des
   Plans und muss niemals hinter en zurückfallen. Ein PR, der en-Keys ohne de
   Werte committet, ist unvollständig.
3. **Vollparität für alle Locales ist CI-pflichtig.**
   `bun run check:translations` erzwingt exakte Leaf-Key-Parität (fehlend und
   zusätzlich) zwischen en und allen 23 übrigen Ordnern (§1). Daraus folgt
   operational: Keys werden **immer als Satz über alle 24 Dateien** angelegt
   oder entfernt — nie „en erstmal, Rest später“. Für nicht übersetzte Werte
   gilt der CONTRIBUTING-Workflow (en-Text einstellen, Community übersetzt);
   der Key existiert dann in allen Dateien, der Wert darf provisorisch dem
   en-Text entsprechen.
4. Interpolationsparameter (§5.2) sind Teil der Parität: ein fehlender
   `{{param}}` in einer Übersetzung ist ein Review-Defect (automatisiert
   prüfbar als Folge-Verbesserung, §12 NG-4).

## 9. Umbenennen ohne Bruch der ausgelieferten Locales

Keys sind API gegenüber 24 Dateien und (für `tray.*`) gegenüber
`TrayStrings`. Bindendes Verfahren für Rename/Move/Delete:

1. **Alles-oder-nichts im selben PR:** Ein Key wird in allen 24 Dateien
   gleichzeitig umbenannt/gelöscht. Die Paritätsprüfung lässt jeden anderen
   Zustand gar nicht erst durch (extra/missing). Ein „Deprecation-Fenster“
   mit parallel gepflegten Alt- und Neu-Keys über mehrere Releases ist
   verboten (NG-5).
2. **Reihenfolge innerhalb des PRs:** (a) neuen Key in allen 24 Dateien
   anlegen, (b) alle `t()`-Aufrufe umstellen, (c) alten Key aus allen 24
   Dateien löschen. Zwischenstände sind nur innerhalb des PR sichtbar.
3. **Keine Wiederverwendung entfernter Pfade** für eine neue Bedeutung
   innerhalb desselben Minor-Releases (Übersetzungs-Caches, laufende
   Fenster-Instanzen).
4. **Bedeutungsänderung = Rename:** Wird der Text eines Keys so geändert, dass
   sich seine Bedeutung verschiebt (nicht nur Orthografie/Ton), ist das
   ebenfalls ein Rename nach 1.–3. — Keys benennen Bedeutungen, keine Inhalte.
5. **Tray-Sonderweg:** `tray.*`-Renames ziehen die Feldumbenennung in
   `TrayStrings` (`tray_i18n.rs`/`tray.rs`) nach; die Build-Kopplung macht
   vergessene Umbenennungen zu Kompilierfehlern.
6. **Dokumentationspflicht:** Jeder Rename wird im UX-Copy-Katalog am
   betroffenen Entry nachgezogen (der Katalog ist die Key-Referenz für
   Reviewer), bei Fehlercode-relevanten Keys zusätzlich in TAX §5/§6.

## 10. RTL-Sicherheit

Gilt für alle neuen Layout-/Copy-Keys unter `meeting.*` und `projects.*`;
App-seitig existiert bereits die dir-Umschaltung über `rtl.ts` +
`languageChanged`-Hook (§1).

1. **Keine Richtungssemantik in Texten:** Copy-Keys dürfen keine Richtungs-
   wörter oder -glyphen enthalten, die Layout bedingen (`left`/`right`,
   `←`, `→`). Räumliche Bezüge heißen logisch (`ms-*`/`me-*`-Tailwind-Utilities
   bzw. `start`/`end`); Icons statt Pfeilzeichen. Beispiel-Verbot:
   `Zum aktuellen Text ↓` — der Hinweis aus Plan §6.11 wird als reiner Text
   ohne Richtungsglyph geführt.
2. **Bidi-Isolation für eingebettete LTR-Token:** Gerätename, Zeitstempel und
   Dauer in RTL-Copy (ar, he) können Richtungswechsel erzeugen. Regel:
   Komponenten wickeln solche Parameter in `<span dir="ltr">…</span>` bzw.
   Unicode-Isolate (U+2066/U+2069) — Entscheidung liegt an der Komponente,
   nicht in der Übersetzung. In Tray-Strings (Rust, kein Markup) werden keine
   Parameter gerendert, die isolierungsbedürftig sind: `tray.meetingRecording`
   bekommt ausschließlich `{{duration}}` (ziffernbasiert, bidi-neutral, §7.2);
   Gerätenamen oder Titel sind im Tray verboten.
3. **Satzzeichen und Klammern:** typographische Zeichen („…“, (), …) gehören
   zur Übersetzung und werden pro Sprache gesetzt; sie sind mirrored-safe.
   Verboten ist, öffnende/schließende Zeichen als separate Parameter
   (`{{open}}{{title}}{{close}}`) zu modellieren.
4. **Kein Layout über Textlänge:** Copy-Keys werden nicht gekürzt, um Layout
   zu retten (Abhängigkeit der Navigations-/Breakpoint-Verträge); Truncation/
   Wrapping regelt CSS. Umgekehrt darf UI keine Annahmen über Tokenzahl machen
   (de ist typisch länger als en; ar/kompakter).
5. **Neue Surface `overlay` (Meeting-Overlay):** übernimmt dieselben Regeln;
   da das Overlay ein eigenes Fenster ist, muss es beim Erzeugen `dir`/`lang`
   selbst setzen (Anschluss an overlay-Konventionen des Diktat-Overlays) —
   Hand-off an die Overlay-Implementierung, hier nur als Constraint dokumentiert.

## 11. Extraktions-Check (ESLint)

### 11.1 Was die Regel heute erzwingt

`i18next/no-literal-string` mit `markupOnly: true` (`eslint.config.js`,
Lauf via `bun run lint` = `eslint src`) flaggt **Literaltext in JSX-Children**
aller `src/**/*.{ts,tsx}`-Dateien. Für alle neuen Komponenten unter
`src/components/meetings/**` und `src/components/projects/**` gilt damit:
jeder sichtbare Text muss `t('…')` durchlaufen — Ausnahmen nur über die
konfigurierten `ignoreAttribute`s.

### 11.2 Bekannte Blinde Flecken (dokumentiert, nicht verschwiegen)

- **JSX-Attribute:** `markupOnly` prüft Attributwerte nicht; `title="…"`,
  `placeholder="…"` o. ä. würden nicht angemahnt. Konvention (reviewpflichtig):
  auch Attribute mit Nutzertext gehen durch `t()`. Eine Verschärfung der
  ESLint-Konfiguration (Abbau von `markupOnly` mit erweiterter Ignore-Liste)
  ist möglich, aber Nicht-Ziel dieses Vertrags (NG-6) — sie berührt den
  gesamten Bestand, nicht nur den Meeting-Modus.
- **Nicht-JSX-Strings:** Toast-/Event-Texte, die aus Stores oder TS-Modulen
  kommen, sieht die Regel nicht; für den Meeting-Modus ist jedoch normiert
  (TAX §2), dass die Grenze nur `{code, params}` passiert und Lokalisierung
  erst an der UI geschieht — d. h. Stores/Backend senden **nie** fertigen Text.
- **Key-Typos:** die Regel erzwingt _dass_ übersetzt wird, nicht _wohin_.
  Gegen Tippfehler in `t('meeting.…')`/`t('projects.…')` dient der
  Key-Existenztest: ein CI-Schritt (Erweiterung von `scripts/check-translations.ts`
  oder eigenes Skript) scannt `t('…')`-Literale und prüft ihre Auflösung in
  `en/translation.json`. TAX §8.3 nennt denselben Test als Gate. Bis zur
  Implementierung bleibt es bei Review-Pflicht (offener Punkt, kein Gate).

### 11.3 Enforcement-Summary

| Prüfung                        | Mechanismus                                  | Status                                   |
| ------------------------------ | -------------------------------------------- | ---------------------------------------- |
| Harcoded JSX-Text              | `i18next/no-literal-string` (`bun run lint`) | existiert, sofort wirksam für neue Bäume |
| Key-Parität en ↔ 23 Locales    | `bun run check:translations`                 | existiert, CI-pflichtig                  |
| Tray-Key-Kopplung              | `build.rs` → `TrayStrings` Compile-Check     | existiert                                |
| Key-Existenz für `t()`-Aufrufe | Skript-Erweiterung                           | vorgeschlagen (TAX §8.3), offener Punkt  |

## 12. Nicht-Ziele und Komplexitätsbudget

- **NG-1:** Keine Migration/Zusammenlegung des bestehenden `meetings.*`-Baums
  mit dem neuen `meeting.*`-Baum.
- **NG-2:** Keine vorab definierten Namespaces für Aufgaben-/Snapshot-/
  Connector-Copy ohne Katalog-Entries (YAGNI, SAP-artiger Vorauseil verboten).
- **NG-3:** Keine sprachspezifischen Plural-Suffixmengen (`_few`/`_many`/
  `_zero`) und keine entsprechende Paritätsskript-Erweiterung in v1.
- **NG-4:** Keine automatisierte Interpolationsparameter-Paritätsprüfung im
  Paritätsskript in v1 (Review-Pflicht genügt; Skriptänderung folgt bei Bedarf
  als eigener kleiner Change).
- **NG-5:** Kein Deprecation-/Alias-Mechanismus für i18n-Keys (kein
  `keyAliases`, kein doppelt gepflegter Bestand).
- **NG-6:** Keine Verschärfung der globalen ESLint-Konfiguration (Attract-
  Mode, Namespace-unabhängig) in diesem Vertrag.
- **NG-7:** Keine ICU-MessageFormat-Migration, kein zweites i18n-Framework,
  keine Runtime-Switching-Infrastruktur beyond i18next.
- **NG-8:** Keine serverseitige/Backend-Lokalisierung von Meeting-Texten
  (TAX Non-Goal restated); Rust-Seite bleibt auf build.rs-generierte Tray-
  Strings beschränkt.

## 13. Gate-Selbstprüfung

| Gate                                                                               | Nachweis                                                                                                                                                                           |
| ---------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| G1 — Namespace-Baum bildet Katalog 1:1 ab, Coverage-Checkliste im Anhang           | §3 führt jeden COPY-Key mit Quell-Entry (S1–S10, E1–E17, P1–P10, §2.3-Aktionen, TAX-NEW) auf; Anhang A checklistiert jeden Katalogentry gegen den Baum inklusive Normalisierungen. |
| G2 — Plural-/Interpolationsmuster mit en- und de-Beispielen für Segmente und Dauer | §5 (Interpolation), §6.3 (Segmentzahlen, en+de), §7 (Countdown-Sekunden en+de-Muster, Dauerformat `H:MM:SS`).                                                                      |
| G3 — Renaming-Regel ohne Bruch ausgelieferter Locales                              | §9: Alles-oder-nichts über 24 Dateien (CI-erzwungen, §1/§8.3), Reihenfolge im PR, kein Path-Reuse, Bedeutungsänderung = Rename, Tray-Kopplung, Katalog-Nachzug.                    |
| G4 — Nicht-Ziele-Abschnitt; Dokumentationsindex aktualisiert                       | §12 (NG-1–NG-8); Tabellenzeile + Lesereihenfolge-Eintrag in `docs/meeting-mode-documentation-index.md` (im selben Changeset ergänzt).                                              |

---

## Anhang A — Coverage-Checkliste (Katalogentry ↔ Namespace)

Jede Zeile: Katalogentry → aufnehmende Gruppe in §3/§4. ✓ = Key-Pfad im Baum
vorhanden; † = Pfad gegenüber Katalognotation normalisiert (§6.4 bzw. §2.4).

| Catalog-Entry                                   | Keys                                                                                                                                                                                        | Baum-Ziel                                                                                                    | Check                              |
| ----------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------ | ---------------------------------- |
| §2.3 Aktionen (21 Keys)                         | `tryAgain` … `confirmDelete`                                                                                                                                                                | `meeting.actions.*`                                                                                          | ✓ (21/21)                          |
| S1 draft                                        | `session.draft.hint`, `actions.startMeeting`                                                                                                                                                | `meeting.session.draft`, `meeting.actions`                                                                   | ✓                                  |
| S2 starting                                     | `starting.{title,body,countdown,escHint}`                                                                                                                                                   | `meeting.session.starting`                                                                                   | ✓                                  |
| S3 recording                                    | `recording.{title,saved,provider.local,provider.cloud}`, `consent.indicator.label`, `cloud.active.azure`, `tray.meetingRecording`                                                           | `meeting.session.recording`, `meeting.consent`, `meeting.cloud.active`, `tray`                               | ✓                                  |
| S4 paused                                       | `paused.{title,body}`, `tray.meetingPaused`                                                                                                                                                 | `meeting.session.paused`, `tray`                                                                             | ✓                                  |
| S5 stopping                                     | `stopping.{title,body,done}`                                                                                                                                                                | `meeting.session.stopping`                                                                                   | ✓                                  |
| S6 transcribing                                 | `processing.{title,step.saved,step.prepared_one,step.transcribed_one,closeOk,backlog}`, `cloud.active.azureProcessing`, `tray.meetingProcessing`                                            | `meeting.processing`, `meeting.cloud.active`, `tray`                                                         | ✓ † (Pluralpaare)                  |
| S7 summarizing                                  | `processing.summarizing`, `notify.completed.{title,body}`, `cloud.active.llm`                                                                                                               | `meeting.processing`, `meeting.notify`, `meeting.cloud.active`                                               | ✓                                  |
| S8 completed                                    | `completed.{title,saved}`                                                                                                                                                                   | `meeting.session.completed`                                                                                  | ✓                                  |
| S9 recoverable_error                            | `errors.recoverable.{title,detail}`                                                                                                                                                         | `meeting.errors.recoverable`                                                                                 | ✓                                  |
| S10 stopped_unprocessed/cancelled               | `cancelled.{title,body}`                                                                                                                                                                    | `meeting.session.cancelled`                                                                                  | ✓                                  |
| E1 micLost                                      | `errors.micLost.{title,body}`                                                                                                                                                               | `meeting.errors.micLost`                                                                                     | ✓                                  |
| E2 systemLost                                   | `errors.systemLost.{title,body}`                                                                                                                                                            | `meeting.errors.systemLost`                                                                                  | ✓                                  |
| E3 defaultChanged                               | `errors.defaultChanged.{title,body,followed}`                                                                                                                                               | `meeting.errors.defaultChanged`                                                                              | ✓                                  |
| E4 silence                                      | `warnings.silence.{title,body}`, `actions.testSystemAudio`                                                                                                                                  | `meeting.warnings.silence`, `meeting.actions`                                                                | ✓                                  |
| E5 noLoopback                                   | `errors.noLoopback.{title,body}`, `session.micOnly.badge`                                                                                                                                   | `meeting.errors.noLoopback`, `meeting.session.micOnly`                                                       | ✓                                  |
| E6 exclusiveMode                                | `errors.exclusiveMode.{title,body}`                                                                                                                                                         | `meeting.errors.exclusiveMode`                                                                               | ✓                                  |
| E7 diskLow                                      | `warnings.diskLow.{title,body}`                                                                                                                                                             | `meeting.warnings.diskLow`                                                                                   | ✓                                  |
| E8 diskFull                                     | `errors.diskFull.{title,body}`                                                                                                                                                              | `meeting.errors.diskFull`                                                                                    | ✓                                  |
| E9 cloudUnreachable                             | `errors.cloudUnreachable.{title,body,waiting}`                                                                                                                                              | `meeting.errors.cloudUnreachable`                                                                            | ✓                                  |
| E10 cloudConfig                                 | `errors.cloudConfig.{title,body}`                                                                                                                                                           | `meeting.errors.cloudConfig`                                                                                 | ✓                                  |
| E11 backlog                                     | `warnings.backlog.{title,body}`                                                                                                                                                             | `meeting.warnings.backlog`                                                                                   | ✓                                  |
| E12 recovery                                    | `recovery.{title,body}`, `actions.{recover,decideLater}`                                                                                                                                    | `meeting.recovery`, `meeting.actions`                                                                        | ✓                                  |
| E13 slept                                       | `errors.slept.{title,body}`                                                                                                                                                                 | `meeting.errors.slept`                                                                                       | ✓                                  |
| E14 devicesChanged                              | `warnings.devicesChanged.{title,body}`, `actions.resumeWithDevices`                                                                                                                         | `meeting.warnings.devicesChanged`, `meeting.actions`                                                         | ✓                                  |
| E15 dictation conflict                          | `conflicts.dictation.{title,body}`                                                                                                                                                          | `meeting.conflicts.dictation`                                                                                | ✓                                  |
| E16 missingKey                                  | `errors.missingKey.{title,body.cloud,body.local,body.summary}`                                                                                                                              | `meeting.errors.missingKey`                                                                                  | ✓                                  |
| E17 micPermission                               | `errors.micPermission.{title,body}`                                                                                                                                                         | `meeting.errors.micPermission`                                                                               | ✓                                  |
| P1 beforeStart consent                          | `consent.beforeStart.{title,body,acknowledge}`                                                                                                                                              | `meeting.consent.beforeStart`                                                                                | ✓                                  |
| P2 indicator                                    | `consent.indicator.label`                                                                                                                                                                   | `meeting.consent.indicator`                                                                                  | ✓                                  |
| P3 azure notice                                 | `cloud.notice.azure.{title,body,detail}`, `actions.useLocal`                                                                                                                                | `meeting.cloud.notice.azure`, `meeting.actions`                                                              | ✓                                  |
| P4 active azureProcessing                       | `cloud.active.azureProcessing`                                                                                                                                                              | `meeting.cloud.active`                                                                                       | ✓ (mit S6 dedupliziert)            |
| P5 llm notice                                   | `cloud.notice.llm.{title,body}`, `cloud.active.llm`, `actions.{useCloudSummary,summarizeLocally,disableSummary}`                                                                            | `meeting.cloud.notice.llm`, `meeting.cloud.active`, `meeting.actions`                                        | ✓                                  |
| P6 retention                                    | `retention.question.{title,body}`, `retention.option.{deleteNow,7days,30days,forever}`, `actions.decideLater`                                                                               | `meeting.retention.*`                                                                                        | ✓                                  |
| P7 stopConfirm                                  | `stopConfirm.{title,body,timeout}`                                                                                                                                                          | `meeting.stopConfirm`                                                                                        | ✓                                  |
| P8 delete level 1                               | `delete.level1.{label,body}`                                                                                                                                                                | `meeting.delete.level1`                                                                                      | ✓                                  |
| P9 delete level 2                               | `delete.level2.{label,body}`                                                                                                                                                                | `meeting.delete.level2`                                                                                      | ✓                                  |
| P10 delete level 3                              | `delete.level3.{label,body,itemAudio,itemTranscript,itemSummary,irreversible,confirm}`                                                                                                      | `meeting.delete.level3`                                                                                      | ✓                                  |
| TAX NEW-Familien (§5-Zeilen „NEW“ + §6-Tabelle) | `deviceUnavailable`, `storageWriteFailed`, `actionUnavailable`, `captureStoppedUnexpectedly`, `consentRequired`, `segmentsFailed`, `audioPackageRejected`, `languageUnsupported`, `generic` | `meeting.errors.*`; zusätzlich aus TAX §5: `warnings.degradedCapture`, `warnings.localTranscriptionRetrying` | ✓                                  |
| Projekte (Plan §6.16–§6.18)                     | Slots ohne Wortlaut                                                                                                                                                                         | `projects.*` (§4)                                                                                            | ○ (Namespace fixiert, Texte offen) |

Coverage-Resultat: **alle** Katalogentries mit Key-Pfad sind im Baum
aufgenommen; Deduplizierungen (P4≡S6-Zeile, P2≡S3-Indikator) sind explizit
genannt; keine Katalog-Keys fehlen, keine erfundenen Katalog-Keys.
