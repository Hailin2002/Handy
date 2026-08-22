# Speak Meeting-, Projekt- und Wissenssystem — finaler Umsetzungsplan

Status: **finale Planung, noch keine Implementierung**  
Basis: Branch `feat/custom-stt-pipeline`, geprüft am 22.08.2026  
Zielplattform für v1: **Windows x64, lokal-first**

## 1. Ziel

Speak wird im bestehenden Tauri-/React-Frontend um zwei eigenständige
Produktbereiche erweitert:

1. einen Meeting-Modus neben dem unveränderten Diktier-Modus;
2. projektbasierte Wissens- und Aufgabenverwaltung über Meetings, WhatsApp-Exporte,
   manuelle Notizen und Dateien.

Der Meeting-Modus zeichnet unabhängig von der verwendeten Kommunikations-App
gleichzeitig auf:

- das ausgewählte Mikrofon;
- den Ton des ausgewählten Windows-Ausgabegeräts über WASAPI-Loopback.

Die Aufnahme wird während des Meetings sicher auf die Festplatte geschrieben und
abschnittsweise transkribiert. Für jedes Meeting kann unabhängig vom Diktier-Modus
zwischen lokaler STT-Verarbeitung, Azure Speech und dem vorhandenen Custom-STT-Pfad
gewählt werden. Anschließend kann Speak lokal oder über einen ausgewählten
Cloud-LLM-Anbieter eine Zusammenfassung erstellen. Ein Meeting kann vor dem Start
einem Projekt zugeordnet oder zunächst in einem Eingang ohne Projekt gespeichert
werden. Nach der Verarbeitung darf die KI ein passendes Projekt vorschlagen, aber
nie ohne sichtbare Bestätigung umsortieren.

Der Meeting-Modus muss mit Teams, WhatsApp, Zoom, Google Meet, Discord,
Browser-Audio und anderen Anwendungen funktionieren, ohne an deren APIs gebunden zu
sein.

Ein Projekt bündelt seine Quellen und zeigt daraus einen belegbaren aktuellen
Status, Entscheidungen, offene Fragen und priorisierte Aufgaben. Als erster externer
Quellentyp wird ein WhatsApp-Export als `.zip` oder `.txt` unterstützt; enthaltene
Bilder und unterstützte Dokumente können nach ausdrücklicher Auswahl mit einem
multimodalen Modell verarbeitet werden.

Die v1 bleibt vollständig auf einem Windows-Gerät nutzbar. Zentrale
Server-Synchronisation, iPhone-App, MCP-Zugriff sowie direkte WhatsApp- und
E-Mail-Anbindungen sind Zukunftsfunktionen. Datenmodell und Services werden jetzt so
geschnitten, dass diese Erweiterungen keine Ablösung der lokalen Datenhaltung
erfordern.

## 2. Festgelegter Umfang

### Bestandteil von v1

- Windows-Systemaudio über den kompletten ausgewählten Output-Endpunkt;
- Mikrofon und Systemaudio als technisch getrennte Quellen;
- stundenlange, speicherbegrenzte Aufnahme auf Festplatte;
- lokale oder Azure-basierte Abschnittstranskription;
- optional Custom STT mit demselben segmentbasierten Auftragssystem;
- schrittweise sichtbares Transkript, sobald Abschnitte fertig verarbeitet sind;
- Verarbeitung darf während der Aufnahme zurückliegen und nach dem Stoppen
  weiterlaufen;
- persistente Jobs mit Wiederaufnahme nach Netzwerkfehler oder App-Neustart;
- Zusammenfassung, Entscheidungen, Aufgaben und offene Punkte;
- Meeting-Bibliothek, Suche, Kopieren, Export und kontrollierte Löschung;
- Projekte mit Übersicht, Quellen, Meetings und Aufgaben;
- optionales Projekt beim Meetingstart sowie manuelle Zuordnung danach;
- KI-gestützter, bestätigungspflichtiger Projektvorschlag;
- projektübergreifender Eingang für noch nicht zugeordnete Inhalte;
- WhatsApp-Import aus `.zip` und `.txt`, einschließlich zugeordneter Medien;
- lokales Parsen, Vorschau, Deduplizierung und belegbare Projektaktualisierung;
- optionales multimodales Verstehen geeigneter Bilder und Dokumente über einen
  ausdrücklich gewählten, kompatiblen OpenRouter-Anbieter;
- projektbezogene Statusübersicht und vorgeschlagene Aufgaben mit Quellenbelegen;
- Vollbild-/Maximieren-Nutzung und responsives Layout für große und kleine Fenster;
- klarer sichtbarer Aufnahmestatus und Cloud-Hinweis.

### Bewusst nicht Bestandteil von v1

- akustische Diarisierung;
- Erkennung oder Speicherung von Sprechernamen;
- Zuordnung einzelner Remote-Teilnehmer;
- Meeting-Bots oder Teams-/Zoom-spezifische Integrationen;
- UI- oder Accessibility-Scraping anderer Apps;
- automatische oder heimliche Aufnahme;
- Per-Prozess-Loopback;
- echtes Wort-für-Wort-Livetranskript mit geringer Latenz;
- macOS- und Linux-Systemaudio.
- zentrale Cloud-Synchronisation oder Mehrgerätebetrieb;
- iPhone-App;
- lokaler oder entfernter MCP-Server;
- direkte WhatsApp-, E-Mail-, Teams- oder Kalender-Schnittstellen;
- automatische Schreibzugriffe externer Agenten;
- vollautomatische Übernahme von KI-Aufgaben ohne menschliche Kontrolle;
- semantische Vektorsuche als Pflichtbestandteil der ersten Version.

Intern trägt jeder Audio- und Textabschnitt weiterhin die sichere Quelleninformation
`microphone` oder `system`. Das ist keine Sprechererkennung. Die normale Ansicht
zeigt standardmäßig ein chronologisches Transkript ohne erfundene Personenzuordnung.

### Jetzt bauen und später ermöglichen

| Jetzt verbindlich bauen | Jetzt nur architektonisch vorbereiten |
| --- | --- |
| Windows-Aufnahme, lokale/Azure/Custom-STT | macOS, Linux und iPhone-Aufnahme |
| lokale SQLite-Datenbank und lokaler Dateispeicher | Server-Datenbank, Objektspeicher und Synchronisation |
| Meetings, Projekte, Aufgaben, Status und Quellenbelege | gemeinsamer Zugriff von mehreren Geräten |
| WhatsApp-ZIP/TXT als manueller Import | direkte WhatsApp- und E-Mail-Connectoren |
| lokale oder bewusst gewählte Cloud-KI | zentrale serverseitige Verarbeitung |
| lokale Volltextsuche mit SQLite FTS5 | Embeddings und Vektorsuche |
| interne Service-Schnittstellen | authentifizierter Remote-MCP-Server für Codex |

Die rechte Spalte erzeugt in v1 weder Servercode noch ungenutzte UI-Schalter. Sie
bestimmt nur stabile IDs, Datenbeziehungen und Schnittstellengrenzen.

## 3. Was aus dem zusätzlichen Vorschlag übernommen wird

Folgende Punkte verbessern den bestehenden Plan technisch deutlich und werden
übernommen:

1. **Zwei getrennte Aufnahmequellen.** Das reduziert Echo- und Pegelprobleme und
   erlaubt eine unabhängige VAD- und Fehlerbehandlung.
2. **Gemeinsame monotone Zeitachse.** Mikrofon und Output-Gerät besitzen getrennte
   Hardware-Clocks. Zeitstempel, Sample-Counter, Gap-Filling und kontrollierte
   Resampling-Korrektur sind deshalb Pflicht.
3. **Eine lokale STT-Engine mit persistenter Queue.** Dadurch entstehen keine zwei
   konkurrierenden Modellinstanzen mit doppeltem RAM-/VRAM-Bedarf.
4. **Per-Prozess-Loopback nur später.** Windows stellt dafür eine offizielle API ab
   Windows 10 Build 20348 bereit, aber sie ist für den app-unabhängigen MVP nicht
   erforderlich.
5. **Getrennte Audioquellen für Azure als Stereo-Upload.** Azure Fast Transcription
   kann zwei Kanäle unabhängig transkribieren. So bleiben beide Quellen sauber,
   obwohl nur ein Upload nötig ist.
6. **Strukturierte Daten statt einer Sammlung loser Textdateien.** Rohimporte und
   Exporte bleiben portabel, aber Beziehungen, Aufgabenstatus, Belege und
   Wiederholungsjobs liegen in einer transaktionalen Datenbank.
7. **Generische Quellen-Pipeline.** Meeting, WhatsApp-Export, Datei und spätere
   Connectoren werden nach dem Einlesen auf dasselbe Quellenmodell normalisiert.
8. **Belege vor KI-Automatik.** Jede KI-generierte Statusaussage und Aufgabe verweist
   auf Meeting-Zeitstempel, Chatnachricht oder Datei. Vorschläge sind sichtbar als
   solche markiert und müssen angenommen, bearbeitet oder verworfen werden.
9. **Sync-fähige Identitäten ohne Sync in v1.** Stabile IDs, Revisionen,
   Änderungszeiten, Löschmarker und Inhalts-Hashes verhindern, dass eine spätere
   Serverversion eine vollständige Migration benötigt.

Nicht übernommen werden:

- Diarisierung und Namenserkennung, weil sie ausdrücklich nicht gewünscht sind;
- Parakeet V3 als ungeprüfter Standard. Speak besitzt Parakeet V3 und mehrere
  Whisper-Modelle, aber das beste deutsche Meeting-Modell wird gemessen;
- ein einzelnes, stundenlang offenes Stereo-WAV als primäre Persistenz. Rotierende,
  atomar abgeschlossene Dateien sind absturzsicherer;
- Live-STT als Pflicht. Für diesen Anwendungsfall hat Robustheit und Genauigkeit
  Vorrang vor Sekundenlatenz.

## 4. Verifizierter Ausgangszustand im Repository

- `speech_pipeline.rs` nutzt für Azure derzeit ausschließlich den Short-Audio-
  Endpunkt und blockiert Audio über 60 Sekunden.
- `actions.rs` sendet Cloud-STT erst nach dem Stoppen die vollständige Aufnahme.
- `AudioRecorder::stop()` liefert ein vollständiges `Vec<f32>` zurück.
- `recorder.rs` sammelt alle resampelten Samples in `processed_samples`.
- Beim Speichern wird dieser Vektor in `actions.rs` zusätzlich geklont.
- `list_output_devices()` existiert bereits.
- `selected_output_device` wird momentan für Feedback-Töne verwendet. Diese
  Einstellung darf nicht stillschweigend als Meeting-Loopback-Gerät umgedeutet
  werden.
- `TranscriptionManager` besitzt eine gemeinsam geschützte geladene Engine und ist
  daher für eine serialisierte Meeting-Queue geeignet.
- Die bisherige History-Tabelle besitzt nur einen flachen Eintrag pro Diktat. Sie
  kann Meeting-Zustände, Audioabschnitte und Wiederholungsjobs nicht abbilden.
- `HistoryManager` speichert bereits in `history.db` unter dem von `portable.rs`
  aufgelösten App-Datenverzeichnis. Projekt- und Assetspeicherung muss dieselbe
  portable-aware Pfadlogik verwenden.
- OpenRouter ist bereits als Post-Processing-Provider in `settings.rs` und
  `llm_client.rs` vorhanden. Der Projektpfad erweitert diese Infrastruktur um
  Capability-Prüfung und multimodale Inhalte, statt einen zweiten Key-/Provider-
  Mechanismus einzuführen.
- Die uncommittete Upstream-Branch `origin/chunking` enthält verwertbare VAD-
  Chunking-Ideen, sammelt aber ebenfalls die Gesamtaufnahme im RAM und wird nicht
  vollständig übernommen.
- Die lokale CPAL-Version 0.16 setzt für ein Render-Gerät beim Aufbau eines
  Input-Streams das WASAPI-Flag `AUDCLNT_STREAMFLAGS_LOOPBACK`.

Folgerung: Der Diktierpfad bleibt erhalten. Meeting-Aufnahme, Persistenz und
Jobsteuerung werden als eigener Lebenszyklus gebaut und verwenden nur geeignete
bestehende Bausteine wieder.

## 5. Zielarchitektur

```text
Mikrofon ──> Capture-Frames ──> gemeinsame Timeline ──> rollierende Mic-Dateien ─┐
                                                                                ├─> STT-Jobs
Output ────> WASAPI Loopback ─> Gap/Drift-Korrektur ─> rollierende System-Dateien ┘
                                                                                     │
                          ┌──────────────────────────────────────────────────────────┘
                          v
              Local Queue | Azure Fast | Custom STT
                          v
              zeitlich sortierte Transkriptsegmente
                          v
             lokale oder Cloud-Zusammenfassung
                          v
             normalisierte, belegbare Quelle
                          v
       Projektstatus / Aufgaben / Suche / Meeting-Bibliothek
```

Die Architektur besitzt drei bewusst getrennte Ebenen:

```text
React-Oberfläche
  -> Tauri-Commands und Events
    -> Domain-Services für Meetings, Projekte, Import und KI
      -> Repository-Schnittstellen
        -> SQLite + lokaler AssetStore (v1)
        -> Server-API + Objektspeicher (später)
```

Die React-Oberfläche liest weder SQLite noch lokale Dateien direkt. Dadurch können
später Desktop-App, iPhone-App und MCP dieselben fachlichen Services verwenden.

### Neue Backend-Komponenten

#### `MeetingManager`

Zentrale Zustandsmaschine und einzige öffentliche Koordinationsstelle. Sie startet
und stoppt Capture, erzeugt Sessions, verwaltet Pause/Fortsetzen, veröffentlicht
Tauri-Events und stößt Transkription sowie Zusammenfassung an.

Vorgesehener Ort: `src-tauri/src/managers/meeting.rs`.

#### `MeetingRecorder`

Eine eigenständige Aufnahmeimplementierung für zwei Quellen. Der bestehende
`AudioRecorder` wird nicht mit Meeting-Sonderfällen überladen. Wiederverwendet werden
Geräteauflistung, Sample-Konvertierung, `FrameResampler` und VAD-Bausteine.

Vorgesehener Ort:
`src-tauri/src/audio_toolkit/audio/meeting_recorder.rs`.

#### `MeetingStore`

SQLite-Zugriff für Sessions, Chunks, STT-Jobs, Transkriptsegmente und
Zusammenfassungen. Schreiboperationen erfolgen transaktional.

Vorgesehener Ort: `src-tauri/src/managers/meeting_store.rs`.

#### `MeetingTranscriptionQueue`

Persistente, abbrechbare Queue mit genau einem lokalen Engine-Lease oder begrenzten
Cloud-Requests. Audioaufnahme darf niemals auf STT warten.

Vorgesehener Ort:
`src-tauri/src/managers/meeting_transcription.rs`.

#### `MeetingSummaryService`

Erstellt aus dem vollständigen, sortierten Transkript eine strukturierte
Zusammenfassung. Verwendet die bestehende Provider- und Secret-Infrastruktur, aber
eigene Meeting-Prompts und eine eigene Jobpersistenz.

Vorgesehener Ort: `src-tauri/src/managers/meeting_summary.rs`.

#### `ProjectManager`

Verwaltet Projekte, Zuordnungen, Aufgaben, Projektstatus und den Eingang für noch
nicht zugeordnete Quellen. Manuelle Nutzeränderungen besitzen Vorrang vor späteren
KI-Läufen.

Vorgesehener Ort: `src-tauri/src/managers/project.rs`.

#### `KnowledgeStore`

Transaktionale Repository-Schicht für Projekte, Quellen, normalisierte Inhalte,
Assets, Aufgaben, Status-Snapshots und Änderungsrevisionen. Die bestehende
`transcription_history` wird nicht zweckentfremdet.

Vorgesehener Ort: `src-tauri/src/managers/knowledge_store.rs`.

#### `SourceIngestionService`

Ein gemeinsamer Auftragspfad für manuelle Dateien und spätere Connectoren. Jeder
Importer implementiert `inspect`, `preview`, `normalize`, `deduplicate` und
`commit`; nachgelagerte Suche und KI kennen das ursprüngliche Transportformat nicht.

Vorgesehener Ort: `src-tauri/src/managers/source_ingestion.rs`.

#### `WhatsAppExportImporter`

Liest `.zip` und `.txt` defensiv, erkennt Formatvarianten, verknüpft Medien und
erzeugt vor dem Commit eine verständliche Vorschau. Das Original bleibt entsprechend
der Aufbewahrungsregel als belegbare Quelle erhalten.

Vorgesehener Ort: `src-tauri/src/importers/whatsapp.rs`.

#### `ProjectIntelligenceService`

Erstellt aus neuen oder geänderten Quellen inkrementelle, versionierte
Projekt-Snapshots und Aufgabenvorschläge. Er kapselt lokale und Cloud-Modelle,
Prompt-Versionen, Kosten-/Datenschutzhinweise und Quellenbelege.

Vorgesehener Ort: `src-tauri/src/managers/project_intelligence.rs`.

#### `AssetStore`

Speichert Audio, Bilder, Dokumente und Rohimporte über stabile Asset-IDs und
Inhalts-Hashes. v1 implementiert einen lokalen Store im von `portable.rs`
ermittelten App-Datenverzeichnis; eine spätere Objektspeicher-Implementierung ändert
die fachlichen Aufrufer nicht.

Vorgesehener Ort: `src-tauri/src/storage/asset_store.rs`.

### Frontend-Komponenten

- `src/components/shell/AppShell.tsx`
- `src/components/shell/PrimaryNavigation.tsx`
- `src/components/dictation/DictationHome.tsx`
- `src/components/dictation/DictationStatusCard.tsx`
- `src/components/meetings/MeetingSetup.tsx`
- `src/components/meetings/MeetingPreflight.tsx`
- `src/components/meetings/MeetingRecorderView.tsx`
- `src/components/meetings/MeetingProcessingView.tsx`
- `src/components/meetings/MeetingDetail.tsx`
- `src/components/meetings/MeetingList.tsx`
- `src/components/meetings/MeetingRecoveryBanner.tsx`
- `src/components/projects/ProjectsHome.tsx`
- `src/components/projects/ProjectCreateDialog.tsx`
- `src/components/projects/ProjectDetail.tsx`
- `src/components/projects/ProjectOverview.tsx`
- `src/components/projects/ProjectTasks.tsx`
- `src/components/projects/ProjectSources.tsx`
- `src/components/projects/UnassignedInbox.tsx`
- `src/components/import/WhatsAppImportWizard.tsx`
- `src/components/settings/ShortcutRecorder.tsx`
- `src/components/settings/dictation/DictationSettings.tsx`
- `src/components/settings/meeting/MeetingSettings.tsx`

Alle sichtbaren Texte werden zuerst in der englischen i18n-Quelldatei angelegt und
danach in die übrigen Sprachdateien übertragen. Es gibt keine hart codierten
JSX-Texte.

## 6. Frontend-, Zustands- und Bedienlogik

### 6.1 Erkenntnisse aus der Meetily-Prüfung

Geprüft wurden das offizielle Meetily-Repository, die dort veröffentlichten
Screenshots sowie die Komponenten für Hauptseite, Aufnahme, Gerätewahl, Einstellungen,
Transkript, Meeting-Details und Onboarding.

Meetily löst folgende Dinge gut, die Speak übernimmt:

- eine große und jederzeit erkennbare zentrale Aufnahmeaktion;
- eine klare Laufzeitanzeige mit sichtbarem Recording-Status;
- Geräte, Modell und Sprache vor der Aufnahme schnell erreichbar;
- Zeitstempel und einen eindeutigen Listening-/Processing-Zustand im Transkript;
- eine Meeting-Historie in der Primärnavigation;
- nach dem Meeting eine gemeinsame Arbeitsfläche für Transkript und Ergebnis;
- Pause und Stop als unterschiedliche, dauerhaft sichtbare Aktionen;
- Recovery als sichtbaren Nutzerfluss statt als stillen Hintergrundprozess.

Speak kopiert bewusst nicht:

- mehrere gleichwertige Aufnahmebuttons an verschiedenen Stellen;
- rote Farbe bereits für den noch nicht gestarteten Primärbutton;
- Navigation, die nur aus unbeschrifteten Icons besteht;
- Audio-Backend-Begriffe wie WASAPI oder Core Audio in der normalen Geräteauswahl;
- versteckte Edit-/Delete-Aktionen, die erst beim Hover sichtbar werden;
- Modell-, Geräte- und Sprachänderungen während einer laufenden Aufnahme ohne klare
  Aussage, wann sie wirksam werden;
- eine dauerhaft geteilte Transkript-/Editoransicht als einzige Detailansicht. Sie
  ist für Power-User nützlich, überfordert aber viele Nutzer beim ersten Öffnen.

### 6.2 Verbindliche UX-Prinzipien

1. **Diktieren und Meetings sind zwei Produkte im selben App-Rahmen.** Sie haben
   getrennte Startaktionen, Zustände, Shortcuts, Einstellungen, Provider-Snapshots
   und Verläufe.
2. **Eine Bildschirmseite besitzt genau eine offensichtliche Hauptaktion.** Weitere
   Aktionen sind sekundär oder in einem klar beschrifteten Menü.
3. **Text und Icon werden gemeinsam verwendet.** Ein Icon allein reicht nur für
   allgemein bekannte Aktionen wie Schließen, wenn zusätzlich ein zugänglicher Name
   vorhanden ist.
4. **Technik bleibt hinter verständlichen Entscheidungen verborgen.** Der Nutzer
   wählt „Auf diesem Gerät“ oder „Azure Cloud“, nicht WASAPI, Multipart oder
   Engine-Lease.
5. **Fortschritt ist ehrlich.** `Aufnahme läuft`, `Audio sicher gespeichert`,
   `Transkription läuft`, `Zusammenfassung läuft` und `Fertig` sind getrennte
   Zustände.
6. **Keine stille Automatik bei sensiblen Aktionen.** Cloud-Upload, Start einer
   Langzeitaufnahme, Änderung des aktiven Geräts und Löschen benötigen sichtbares
   Feedback.
7. **Standardfall zuerst, Details bei Bedarf.** Erweiterte Modell-, Chunk- und
   Provideroptionen sind nicht Teil des normalen Startflusses.
8. **Fehler erklären die nächste Handlung.** Keine rohen Backendfehler und kein
   „etwas ist fehlgeschlagen“ ohne Button wie `Erneut versuchen`, `Gerät wechseln`
   oder `Einstellungen öffnen`.
9. **Aufnahmezustand ist überall konsistent.** Hauptfenster, Overlay, Tray und
   Shortcut reagieren auf dieselbe Backend-Zustandsmaschine.
10. **Die App darf geschlossen werden.** Wenn Aufnahme oder Verarbeitung im Tray
    weiterläuft, sagt Speak das ausdrücklich und verhindert einen versehentlichen
    vollständigen Prozess-Exit.

### 6.3 Neue Informationsarchitektur

Die heutige Sidebar besteht fast vollständig aus Einstellungsbereichen. Für zwei
Werkzeuge wird sie zu einer Produktnavigation umgebaut:

```text
Speak

  Diktieren
  Meetings
  Projekte
  Diktatverlauf

  ─────────────
  Einstellungen
  Hilfe & Über
```

- **Diktieren** ist die Startseite für den schnellen Speech-to-Text-Workflow.
- **Meetings** enthält die Meeting-Liste und die Aktion `Neues Meeting`.
- **Projekte** bündelt Status, Aufgaben, Meetings und importierte Quellen. Ein
  sichtbarer Zähler markiert nur den Eingang mit noch nicht zugeordneten Elementen,
  nicht jede neue KI-Aktivität.
- **Diktatverlauf** bleibt der bestehende kurze Transkriptionsverlauf.
- **Einstellungen** öffnet eine eigene Seite mit sekundären Tabs, statt jeden
  Einstellungsbereich in der Primärnavigation anzuzeigen.
- Während ein Meeting läuft, erscheint direkt unter `Meetings` ein angehefteter
  Eintrag `Aufnahme läuft · 00:42:18` mit rotem Statuspunkt.

Sekundäre Einstellungstabs:

```text
Diktieren | Meetings | Modelle & Provider | Allgemein | Erweitert
```

Debug-Funktionen bleiben wie heute nur im aktivierten Debug-Modus sichtbar.

### 6.4 Diktieren-Seite

Die Diktieren-Seite verändert den bewährten globalen Arbeitsablauf nicht. Sie zeigt
nur den aktuellen Zustand und die wichtigsten Einstellungen:

- Status `Bereit`, `Aufnahme`, `Transkription` oder `Fehler`;
- aktuellen Diktier-Shortcut als anklickbares Keycap;
- Modus `Drücken zum Umschalten` oder `Gedrückt halten`;
- STT-Ziel `Lokal`, `Azure` oder `Custom`;
- Sprache und bei lokaler STT das ausgewählte Modell;
- einen ungefährlichen Button `Diktat testen`, dessen Ergebnis nur in einer
  Vorschau erscheint und nicht automatisch in eine andere App eingefügt wird;
- Link `Diktier-Einstellungen` für alle weiteren Optionen.

Auf dieser Seite gibt es keinen Meeting-Startbutton. Dadurch kann ein Nutzer die zwei
Werkzeuge nicht als unterschiedliche Modi desselben Aufnahmebuttons missverstehen.

### 6.5 Meetings-Seite

Leerer Zustand beim ersten Öffnen:

- klare Überschrift `Meetings`;
- ein Satz Nutzenbeschreibung;
- eine große Hauptaktion `Neues Meeting`;
- sekundär `Audiodatei importieren`, falls diese Funktion später freigegeben ist;
- kurzer Datenschutzstatus `Standard: lokal auf diesem Gerät`;
- keine technische Konfiguration und keine leere Tabelle.

Nach vorhandenen Meetings zeigt die Seite:

- oben weiterhin `Neues Meeting` als einzige Primäraktion;
- Suchfeld;
- Filter `Alle`, `In Verarbeitung`, `Fertig`, `Fehler`;
- Meeting-Karten mit Titel, Datum, Dauer, Provider, Fortschritt und einer kurzen
  Zusammenfassung;
- sichtbares Drei-Punkte-Menü für Export, erneute Verarbeitung und Löschen;
- keine Löschaktion ausschließlich beim Hover.

Ein aktives Meeting steht immer oben und kann nicht durch Suche oder Filter
verschwinden.

### 6.6 Meeting-Ersteinrichtung

Die bestehende Diktier-Onboardingstrecke wird nicht verlängert. Erst beim ersten
Öffnen von `Meetings` erscheint eine kurze, eigene Einrichtung:

1. **Verarbeitung wählen**
   - `Lokal auf diesem Gerät` ist die datenschutzfreundliche Empfehlung;
   - `Azure Cloud` erklärt knapp, dass Audio hochgeladen wird;
   - `Custom` liegt unter `Weitere Optionen`;
   - die Auswahl kann später für jedes Meeting geändert werden.
2. **Audio prüfen**
   - Mikrofon mit Livepegel und Button `Mikrofon testen`;
   - Systemaudio mit Livepegel und Button `Systemaudio testen`;
   - der Systemtest spielt einen kurzen Speak-Testton über das ausgewählte
     Output-Gerät und bestätigt, dass Loopback ihn empfängt;
   - verständliche Korrekturaktionen bei fehlendem Gerät oder Berechtigung.
3. **Meeting-Shortcut einrichten**
   - kann übersprungen werden;
   - empfohlener Kandidat unter Windows ist `Ctrl+Alt+Space`, weil der vorhandene
     Diktier-Shortcut `Ctrl+Space` und AI-Diktat `Ctrl+Shift+Space` bleiben;
   - der Kandidat wird vor Anzeige tatsächlich registriert und nur angeboten, wenn
     er auf dem Gerät verfügbar ist;
   - andernfalls werden konfliktfrei registrierbare Alternativen angeboten.

Nach Abschluss landet der Nutzer direkt im kompakten Startdialog. Das Onboarding
wird nicht bei jedem Meeting wiederholt.

### 6.7 Kompakter Startdialog für wiederkehrende Nutzer

Der normale Startdialog bleibt auf einen Bildschirm begrenzt. Er besitzt vier
sichtbare Statuszeilen:

```text
Mikrofon       Standardmikrofon                 ✓ funktioniert
Systemaudio    Lautsprecher / Headset           ✓ funktioniert
Transkription  Lokal · Meeting-Modell           Auf diesem Gerät
Sprache        Deutsch                          Ändern
```

Darunter:

- optionaler Titel, vorausgefüllt mit Datum und Uhrzeit;
- optionales Projekt als kompakter Chip; `Kein Projekt` ist ein gültiger Zustand und
  landet im Eingang;
- Summary-Schalter `Nach dem Meeting zusammenfassen`;
- kompakter Speicherhinweis;
- genau ein Primärbutton `Meeting starten`;
- sekundärer Link `Weitere Einstellungen`.

Jede Statuszeile ist komplett anklickbar und öffnet eine kleine, auf diese eine
Entscheidung begrenzte Auswahl. Providerkeys, Modell-Downloads oder fehlende Geräte
werden dort behoben, ohne den gesamten Dialog zu verlieren.

Der Startdialog merkt sich die letzte funktionierende Konfiguration, speichert aber
für jede Session einen Snapshot. Änderungen an globalen Einstellungen verändern kein
bereits laufendes Meeting.

### 6.8 Farb- und Aktionslogik

- `Meeting starten` verwendet die normale Marken-/Primärfarbe.
- Rot erscheint erst, wenn wirklich aufgenommen wird.
- `Pause` ist neutral und klar beschriftet.
- `Meeting beenden` ist rot und besitzt ein Stop-Symbol plus Text.
- Orange bedeutet unterbrochen oder Aufmerksamkeit erforderlich.
- Grün bedeutet vollständig und sicher abgeschlossen.
- Blau bedeutet Verarbeitung oder Information.

Farbe ist niemals der einzige Informationsträger. Jeder Zustand besitzt Text und
ein passendes Symbol.

### 6.9 Shortcut-Konzept für beide Werkzeuge

| Aktion                        | Standard/Empfehlung Windows                     | Verhalten                                                      |
| ----------------------------- | ----------------------------------------------- | -------------------------------------------------------------- |
| Diktat starten/stoppen        | bestehend `Ctrl+Space`                          | unverändert; Toggle oder Push-to-talk nach Diktier-Einstellung |
| Diktat mit AI-Nachbearbeitung | bestehend `Ctrl+Shift+Space`                    | nur aktiv, wenn Nachbearbeitung eingeschaltet ist              |
| Meeting starten/beenden       | empfohlen `Ctrl+Alt+Space`, explizit aktivieren | startet nach Countdown; beim Beenden Bestätigung               |
| Meeting pausieren/fortsetzen  | standardmäßig unbelegt                          | UI und Tray; optional für Power-User                           |
| Aktuelles Diktat abbrechen    | dynamisch `Escape`                              | gilt nicht als globaler Meeting-Stop                           |

Regeln des Shortcut-Editors:

- jede Aktion besitzt einen eigenen, klar benannten Eintrag;
- Klick auf die Keycap startet die Erfassung; `Escape` bricht die Änderung ab;
- `Zurücksetzen`, `Deaktivieren` und `Shortcut testen` sind sichtbar;
- Meeting-Shortcuts müssen mindestens aus Modifikator plus Taste bestehen;
- identische Speak-Bindings werden blockiert und beide Konfliktstellen angezeigt;
- Registrierung beim Betriebssystem wird vor dem Speichern getestet;
- ein nicht registrierbarer Shortcut bleibt nicht scheinbar erfolgreich gespeichert;
- während der Erfassung werden die übrigen Speak-Shortcuts wie im bestehenden
  System vorübergehend ausgesetzt;
- der aktuelle Shortcut steht im Startbutton, Tray-Menü und Aufnahme-Overlay;
- Änderungen während eines laufenden Meetings gelten erst nach dessen Ende.

Meetingstart per Shortcut:

1. Letzte funktionierende Meeting-Konfiguration vorprüfen.
2. Kompaktes Overlay mit drei Sekunden Countdown anzeigen.
3. `Escape` bricht nur den Countdown ab.
4. Aufnahme startet erst nach erfolgreichem Öffnen beider verlangten Quellen.
5. Eigenes Meeting-Startsignal und eindeutige Systembenachrichtigung ausgeben.

Meetingende per Shortcut:

1. Erster Druck zeigt `Meeting wirklich beenden?` im Overlay.
2. Derselbe Shortcut oder `Enter` bestätigt; `Escape` setzt die Aufnahme fort.
3. Nach kurzer Zeit verschwindet die Bestätigung ohne Zustandsänderung.
4. Erst nach sicherer Chunk-Finalisierung wird `Aufnahme beendet` angezeigt.

Damit kann ein versehentlicher Tastendruck keine stundenlange Aufnahme unbemerkt
beenden.

### 6.10 Gleichzeitige Nutzung beider Werkzeuge

`Unabhängig` bedeutet für den Nutzer:

- Meeting-Einstellungen verändern keine Diktier-Einstellungen und umgekehrt;
- ein laufendes Meeting verhindert kein kurzes Diktat;
- ein Diktat pausiert oder beendet die Meetingaufnahme nicht;
- Meeting-Transkription erhält keine Priorität vor dem interaktiven Diktat;
- Status und Fehler beider Werkzeuge werden getrennt angezeigt.

Technische Bedienregel:

- Läuft bereits ein Meeting, verwendet ein gestartetes Diktat einen zeitlich
  begrenzten Tap des vorhandenen Meeting-Mikrofonstreams, statt dasselbe Gerät ein
  zweites Mal zu öffnen.
- Ist das Meeting pausiert, verwendet Diktat seinen normalen Recorder.
- Läuft gerade ein Diktat und der Nutzer startet ein Meeting, fordert Speak zuerst
  eine klare Entscheidung `Diktat abschließen` oder `Meetingstart abbrechen`.
- Die lokale Meeting-STT-Queue pausiert an einer Segmentgrenze, solange ein Diktat
  dieselbe Engine benötigt, und setzt danach automatisch fort.
- Azure-/Custom-Meetingjobs laufen unabhängig weiter, solange sie keine Capture-
  Ressourcen blockieren.

### 6.11 Laufende Meetingansicht

Die Aufnahmeansicht besitzt drei Ebenen:

1. **Fixierte Statusleiste**
   - `Aufnahme läuft` oder `Pausiert`;
   - Dauer;
   - `Lokal` oder Cloud-Provider;
   - `Audio gespeichert` beziehungsweise konkrete Warnung;
   - minimieren/zum Tray wechseln.
2. **Hauptbereich**
   - zuletzt fertiggestellte Transkriptsegmente mit Zeitstempeln;
   - klarer Zustand `Hört zu`, `Abschnitt wird transkribiert` oder
     `Verarbeitung liegt zurück`;
   - keine automatisch springende Ansicht, sobald der Nutzer nach oben scrollt;
   - Button `Zum aktuellen Text` erscheint dann separat.
3. **Fixierte Steuerung**
   - `Pause/Fortsetzen`;
   - `Meeting beenden`;
   - getrennte kleine Mic-/System-Pegel mit Textlabel;
   - Anzahl wartender Abschnitte nur dann, wenn tatsächlich ein Backlog besteht.

Provider, Modell, Sprache und Geräte sind während der Aufnahme lesbar, aber nicht
direkt veränderbar. `Für nächstes Meeting ändern` führt in Einstellungen, ohne den
aktuellen Session-Snapshot anzutasten.

### 6.12 Nach dem Stoppen

Direkt nach dem Stoppen zeigt Speak keine fertige Zusammenfassung vor, solange Jobs
noch laufen. Die Processing-Seite verwendet eine klare Checkliste:

```text
✓ Aufnahme sicher gespeichert
✓ 42 von 42 Audioabschnitten vorbereitet
● 37 von 42 Abschnitten transkribiert
○ Zusammenfassung wartet
```

Dazu:

- `Im Hintergrund weiterarbeiten`;
- Aussage `Du kannst dieses Fenster schließen`;
- `Fehlerhafte Abschnitte erneut versuchen`, falls nötig;
- keine unbestimmte Prozentanzeige, wenn die Gesamtmenge unbekannt ist;
- Systembenachrichtigung nach Abschluss.

### 6.13 Meeting-Detailansicht

Standardmäßig öffnet die Detailseite den leicht verständlichen Tab `Übersicht`:

- editierbarer Titel, Datum, Dauer und Verarbeitungsstatus;
- Kurzfassung;
- Entscheidungen;
- Aufgaben;
- offene Fragen;
- sichtbare Aktionen `Kopieren` und `Exportieren`;
- weitere Aktionen in einem beschrifteten Menü.

Weitere Tabs:

- `Transkript`: chronologische Segmente, Suche, Zeitstempel und Audioposition;
- `Audio`: Wiedergabe und Aufbewahrung, nur solange Audio vorhanden ist;
- `Verarbeitung`: Provider, Modell, Jobs, Fehler und Wiederholungen.

Auf breiten Bildschirmen kann der Nutzer optional `Geteilte Ansicht` aktivieren, um
Transkript und Zusammenfassung wie in Meetily nebeneinander zu bearbeiten. Sie ist
nicht der unerklärte Standard. Summary-Modell, Template und Neu-Generierung befinden
sich in einem fokussierten Dialog `Zusammenfassung neu erstellen`, nicht als Reihe
technischer Toolbarbuttons.

### 6.14 Zugänglichkeit und Fehlbedienungsschutz

- interaktive Ziele mindestens 44 × 44 CSS-Pixel;
- vollständige Tastaturbedienung und sichtbarer Fokus;
- sinnvolle Tab-Reihenfolge und Fokus-Rückgabe nach Dialogen;
- Screenreader-Namen für alle Icons, Pegel und Statusänderungen;
- Statusänderungen über zurückhaltende Live-Regions statt permanentem Fokuswechsel;
- Textkontrast nach WCAG AA;
- reduzierte Bewegung respektieren;
- Tooltips ergänzen Beschriftungen, ersetzen sie aber nicht;
- Lösch- und Stopaktionen bleiben auch ohne Hover erreichbar;
- irreversible Löschung nennt konkret Audio, Transkript und Zusammenfassung;
- während Aufnahme und Verarbeitung gehen keine Aktionen durch Navigation verloren.

### 6.15 Anfänger- und Usability-Gates

Vor Phase 3 wird ein klickbarer Prototyp mit mindestens fünf Personen getestet, die
Speak und Meetily nicht kennen. Zusätzlich testen erfahrene Diktiernutzer, ob der
bestehende schnelle Workflow langsamer oder verwirrender wird.

Pflichtaufgaben ohne Anleitung:

1. ein normales Diktat starten und beenden;
2. ein lokales Meeting mit Mikrofon und Systemaudio starten;
3. für ein einzelnes Meeting Azure auswählen und den Cloud-Hinweis verstehen;
4. Meeting pausieren und sicher beenden;
5. fertige Zusammenfassung kopieren;
6. Meeting-Shortcut ändern, ohne den Diktier-Shortcut zu überschreiben;
7. einen Geräte- oder Netzwerkfehler beheben;
8. eine unterbrochene Verarbeitung nach Neustart fortsetzen.

Release-Gates:

- kein Tester verwechselt Diktat- und Meetingstart;
- kein Tester glaubt bei Azure, dass die Verarbeitung lokal bleibt;
- kein Meeting wird wegen eines unklaren Stop-/Pause-Buttons versehentlich beendet;
- der Aufnahmestatus ist aus Hauptfenster, Overlay und Tray eindeutig;
- jede kritische Fehlermeldung führt zu einer erkennbaren nächsten Aktion;
- kritische Usability-Probleme werden vor Implementierung der visuellen Details
  behoben und anschließend erneut getestet.

### 6.16 Projekte-Seite

Die Projekte-Seite ist kein Administrationsformular, sondern eine schnelle
Arbeitsübersicht. Sie funktioniert sowohl im normalen Fenster als auch maximiert
oder im Vollbildmodus.

Leerer Zustand:

- Überschrift `Projekte` und ein kurzer Satz zum Nutzen;
- eine Primäraktion `Projekt erstellen`;
- eine sekundäre Aktion `WhatsApp-Export importieren`;
- Erklärung, dass Meetings auch zunächst ohne Projekt aufgenommen werden können.

Mit bestehenden Projekten:

- Suche über Projektnamen, Status, Aufgaben und normalisierte Quellentexte;
- Karten oder kompakte Zeilen mit Name, kurzem Status, nächster dringender Aufgabe,
  letzter Aktualisierung und Anzahl noch ungeprüfter Vorschläge;
- eigener Eintrag `Nicht zugeordnet` für Meetings und Importe ohne Projekt;
- Sortierung `Zuletzt aktiv`, `Dringendste Aufgabe`, `Name`; Standard ist
  `Zuletzt aktiv`;
- eine einzige Primäraktion `Projekt erstellen`; Import bleibt sekundär.

Ein Projekt wird mit genau einem Pflichtfeld `Name` erstellt. Beschreibung, Farbe
und weitere Metadaten sind optional und werden nicht im ersten Dialog erzwungen.

### 6.17 Projekt-Detailansicht

Die Kopfzeile zeigt Projektname, belegbaren Kurzstatus, Zeitpunkt des letzten
KI-Laufs und die Aktionen `Quelle hinzufügen` sowie ein Überlaufmenü. Darunter:

```text
Übersicht | Aufgaben | Meetings | Quellen
```

- **Übersicht:** Status in wenigen Sätzen, Entscheidungen, offene Fragen, dringende
  Aufgaben und letzte Aktivität. Jede Aussage besitzt einen Link zu mindestens
  einer Quelle.
- **Aufgaben:** `Vorgeschlagen`, `Offen`, `Erledigt` und `Verworfen`; Bearbeiten,
  Annehmen und Erledigen sind direkt erreichbar.
- **Meetings:** dieselben Meeting-Datensätze wie in der globalen Bibliothek, nur auf
  das Projekt gefiltert. Es gibt keine duplizierten Meeting-Kopien.
- **Quellen:** WhatsApp-Exporte, Dateien, Notizen und spätere Connectoren mit
  Verarbeitungsstatus, Zeitraum, Datenschutzmodus und Importdatum.

Die Aufgabenreihenfolge ist nachvollziehbar und nicht nur ein undurchsichtiger
KI-Score:

1. überfällige Aufgaben;
2. frühestes bestätigtes Fälligkeitsdatum;
3. vom Nutzer gesetzte Priorität;
4. danach KI-Dringlichkeit und Confidence;
5. bei Gleichstand zuletzt aktualisiert.

Bei KI-Priorität zeigt die UI `Warum dringend?` mit kurzer Begründung und Belegen.
KI-extrahierte Aufgaben beginnen als `Vorgeschlagen`. Der Nutzer kann sie annehmen,
bearbeiten, verwerfen oder mit einer existierenden Aufgabe zusammenführen. Erledigte
oder manuell bearbeitete Aufgaben werden bei einer Neuanalyse nie still
zurückgesetzt.

### 6.18 Projektzuordnung von Meetings und Quellen

- Der Startdialog merkt sich das zuletzt verwendete Projekt, wählt es bei einem
  neuen Meeting aber nur sichtbar vor; ein Klick entfernt die Auswahl.
- Nach genügend Transkript oder nach dem Stop kann Speak `Projekt BDNA Club
  vorgeschlagen` anzeigen. Der Vorschlag enthält Confidence und zwei kurze Belege.
- Standardaktion ist `Zuordnen`; `Anderes Projekt` und `Ohne Projekt lassen` sind
  gleichwertig erreichbar. Ohne Eingabe findet keine automatische Verschiebung
  statt.
- In v1 zeigt die UI genau ein Hauptprojekt. Das Datenmodell erlaubt dennoch mehrere
  Projektverknüpfungen, damit ein bereichsübergreifendes Meeting später nicht
  dupliziert werden muss.
- Neu-Zuordnen ändert nur die Verknüpfung. Transkript, Audio und Aufgabenbelege
  bleiben erhalten.
- Der Eingang erlaubt Stapelzuordnung, aber keine KI-Massenaktion ohne Vorschau.

### 6.19 WhatsApp-Importassistent

Der Import ist ein geführter Vier-Schritt-Dialog:

1. **Datei wählen:** `.zip` oder `.txt`; noch keine Cloudübertragung.
2. **Erkanntes prüfen:** Chatname, Teilnehmer, Zeitraum, Nachrichten, Medien,
   Warnungen und mögliche Dubletten.
3. **Projekt und Verarbeitung wählen:** Zielprojekt, nur lokal importieren oder
   kompatibles Cloud-Modell verwenden; exakt auflisten, welche Inhalte das Gerät
   verlassen und eine Kostenschätzung zeigen.
4. **Import bestätigen:** Fortschritt pro Stufe und ein Ergebnis mit importierten,
   übersprungenen und fehlerhaften Elementen.

Der Parser verlässt sich weder auf einen festen ZIP-Dateinamen noch auf eine einzige
Sprache oder ein einziges Datumsformat. Er unterstützt mehrzeilige Nachrichten,
Systemmeldungen, Gruppenteilnehmer, fehlende Anhänge und wiederholte, überlappende
Exporte. Inhalts-Hashes und stabile externe Schlüssel machen erneute Importe
idempotent.

Lokales Parsen, Deduplizieren und Textindexieren geschieht immer vor einem möglichen
Cloudaufruf. Ein gesamtes Archiv wird nie blind an einen Provider gesendet:

- Bilder gehen nur an ein Modell, das Bildeingaben ausweist;
- PDFs oder Dokumente nur an einen dafür kompatiblen Pfad;
- Sprachnachrichten werden als Assets importiert und nur nach ausdrücklicher Wahl
  durch die vorhandene STT-Pipeline verarbeitet;
- Video wird in v1 gespeichert und als Metadatum/Thumbnail angezeigt, nicht
  automatisch vollständig analysiert;
- nicht unterstützte Dateien bleiben verfügbar und werden klar als `nicht
  analysiert` markiert.

Der Nutzer kann einen Export vollständig lokal importieren und durchsuchen, selbst
wenn Bilder nicht interpretiert werden. Bei OpenRouter werden private lokale Bilder
erst nach Bestätigung als Base64-Dateninhalt über die vorhandene Secret- und
Providerlogik gesendet. Modellwahl wird nach benötigten Eingabemodalitäten gefiltert;
kein einzelnes OpenRouter-Modell wird hart codiert.

### 6.20 Fenster- und responsive Logik

- Das bestehende Speak-Fenster wird erweitert; es entsteht keine zweite
  Verwaltungs-App.
- Standard-Maximieren und Vollbild sind erlaubt. Fenstergröße, Position und
  maximierter Zustand werden pro Gerät gespeichert; ein separater
  `Vollbildmodus`-Schalter ist nicht Voraussetzung für die Bedienung.
- Ab breiter Desktopgröße nutzt die Projektansicht zwei Spalten: Navigation/Liste
  und Detail. Bei schmalen Fenstern werden diese Ebenen nacheinander gezeigt.
- Detailtabs bleiben horizontal scrollbar oder wechseln in ein zugängliches Menü;
  Inhalte werden niemals abgeschnitten.
- Das kleine Aufnahme-Overlay bleibt unabhängig vom Hauptfenster und zeigt auch bei
  geschlossenem Hauptfenster Meetingstatus, Dauer, Pause und Stop.
- Die wichtigste Aktion steht pro Seite oben rechts und optional am unteren Rand bei
  langen Ansichten; es gibt niemals zwei unterschiedlich benannte Buttons für
  dieselbe Aktion.

### 6.21 Erweiterte Usability-Gates

Zusätzlich zu Abschnitt 6.15 müssen neue Nutzer ohne Anleitung:

1. ein Projekt erstellen;
2. beim Meetingstart ein Projekt wählen und die Auswahl wieder entfernen;
3. ein fertiges, nicht zugeordnetes Meeting einem Projekt zuweisen;
4. einen WhatsApp-Export lokal importieren und die Vorschau verstehen;
5. erkennen, welche Bilder bei Cloud-Verarbeitung hochgeladen würden;
6. einen KI-Aufgabenvorschlag annehmen, ändern und verwerfen;
7. von einer Statusaussage zur belegenden Chatnachricht oder Meetingstelle springen;
8. dasselbe Projekt in kleinem Fenster und maximiert bedienen.

Release-Gates:

- kein Tester verwechselt Projektstatus mit ungeprüfter KI-Vermutung;
- kein Cloudimport beginnt vor einer expliziten, inhaltlich verständlichen
  Bestätigung;
- ein wiederholter WhatsApp-Export erzeugt keine sichtbaren Nachrichtendubletten;
- keine Hauptfunktion ist nur über ein Drei-Punkte-Menü oder Hover erreichbar;
- Projekt-, Meeting- und Diktiernavigation bleibt mit Tastatur und Screenreader
  verständlich.

### Session-Zustände

```text
draft
  -> starting
  -> recording <-> paused
  -> stopping
  -> transcribing
  -> summarizing (optional)
  -> completed

Jeder aktive Zustand kann zusätzlich einen recoverable_error besitzen.
Abbruch durch den Nutzer führt zu stopped_unprocessed oder cancelled,
nicht zu einem stillen Datenverlust.
```

Der persistierte Session-Zustand und die tatsächlichen Jobzustände werden beim
Programmstart abgeglichen. Eine Session bleibt nicht dauerhaft in `recording`, wenn
der Prozess abgestürzt ist. Offene Dateien werden als wiederherstellbar markiert,
finalisiert und anschließend zur Verarbeitung angeboten.

### Startvalidierung

Der kompakte Startdialog aus Abschnitt 6.7 zeigt nur verständliche Statuszeilen. Im
Hintergrund validiert der Startvorgang trotzdem vollständig:

- Mikrofon und Systemausgabegerät;
- freien Speicherplatz;
- STT-Provider und gewählte Sprache;
- lokales Meeting-Modell beziehungsweise Azure-/Custom-Konfiguration;
- optionalen Summary-Provider;
- Aufbewahrungsregel;
- erforderliche Berechtigungen.

Fehlende Providerkeys oder Modelle werden in einem fokussierten Reparaturdialog
behoben und nicht als zusätzliche Dauerfelder in den Startdialog eingebaut.

`selected_output_device` für Feedback-Sounds bleibt unangetastet. Der Meeting-Modus
erhält eigene Einstellungen für Mikrofon und Loopback-Output.

Der Startbutton ist erst aktiv, wenn die gewählten Geräte geöffnet werden können,
ausreichend Speicher verfügbar ist und die notwendigen Providerdaten vorhanden
sind. Der Nutzer kann bei nicht verfügbarem Loopback ausdrücklich auf
`Nur Mikrofon aufnehmen` wechseln; dieser Zustand wird deutlich im Meeting
gespeichert und angezeigt.

### Laufende Aufnahme

Die verbindliche Oberfläche steht in Abschnitt 6.11. Hauptfenster, Overlay und Tray
lesen denselben Zustand aus dem `MeetingManager`. Es gibt keinen zweiten lokalen
Recording-State im Frontend und keine voneinander abweichenden Start-/Stop-Handler.

### Pause

Pause schließt den gerade offenen Aufnahmeabschnitt sauber ab und erzeugt einen
Timeline-Marker. Während der Pause wird kein Audio gespeichert oder übertragen.
Beim Fortsetzen werden beide Geräte erneut validiert; ein Gerätewechsel wird als
neuer Quellenabschnitt dokumentiert.

### Stoppen

Stoppen beendet zuerst sicher beide Capture-Streams und finalisiert alle offenen
Dateien. Danach darf die UI geschlossen werden, während verbleibende STT- und
Summary-Jobs im Hintergrund weiterlaufen. Der Nutzer kann die Verarbeitung
abbrechen, ohne die bereits gespeicherte Aufnahme automatisch zu löschen.

### Fehlerverhalten

- **Mikrofon fällt aus:** Systemaudio läuft weiter; sichtbare Warnung und
  Wiederverbinden/Wechsel anbieten.
- **Systemgerät fällt aus:** Mikrofon läuft weiter; Loopback neu öffnen oder bewusst
  ohne Systemaudio fortfahren.
- **Standardgerät wechselt:** nicht unbemerkt wechseln. Bei aktivierter Option
  `Standardgerät folgen` wird ein neuer Quellenabschnitt begonnen, ansonsten bleibt
  das ausgewählte Gerät bindend.
- **Keine Systemlautstärke:** Stille ist kein Fehler. Die Setup-Testfunktion hilft zu
  unterscheiden, ob das Gerät falsch gewählt wurde.
- **Datenträger fast voll:** früh warnen; wenn sich kein neuer Chunk sicher anlegen
  lässt, Aufnahme kontrolliert stoppen und vorhandene Daten erhalten.
- **Azure/Custom nicht erreichbar:** Aufnahme läuft lokal weiter, Jobs wechseln in
  `retry_wait`; keine Abschnitte gehen verloren.
- **Lokales Modell zu langsam:** Backlog wächst nur auf Disk, nicht im RAM.
- **App-Absturz:** abgeschlossene Chunks und Jobs werden beim Neustart erkannt und
  fortgesetzt.

## 7. Windows-Audioaufnahme

### Loopback

Für v1 wird dem CPAL-Input-Stream das ausgewählte Output-Gerät übergeben. CPAL 0.16
aktiviert unter WASAPI dafür Loopback. WASAPI-Loopback funktioniert offiziell nur
im Shared Mode. Ein Exclusive-Mode-Fehler wird als verständlicher Gerätefehler an
die UI gegeben.

Vor der eigentlichen Implementierung beweist ein kleiner Spike auf der Zielhardware:

- Default- und manuell gewähltes Output-Gerät liefern Audio;
- Format, Kanalzahl und native Sample-Rate;
- Verhalten während echter Stille;
- Verhalten bei Kopfhörer- und Standardgerätewechsel;
- Verhalten nach Sleep/Wake;
- gemeinsame Aufnahme mit dem Mikrofon über mindestens zwei Stunden.

Erst dieser Spike entscheidet, ob CPAL für die Zielgeräte ausreicht. Falls nicht,
wird nur der Meeting-Loopback auf direkte WASAPI-Anbindung umgestellt; der
Diktierpfad bleibt unverändert.

### Gemeinsame Zeitachse

Beide Streams erhalten beim Eingang einen Zeitstempel aus derselben monotonen Uhr.
Pro Quelle werden zusätzlich die gelieferten Samples gezählt. Die gemeinsame
Zielzeitachse arbeitet mit 16 kHz Mono pro Quelle.

Die Synchronisierung erledigt drei Aufgaben:

1. Anfangsversatz zwischen den zwei gestarteten Streams durch Stille auffüllen.
2. Fehlende Zeitbereiche anhand Zeitstempel und Sample-Counter erkennen und mit
   Stille repräsentieren, statt die Spur zu verkürzen.
3. Langsame Clock-Abweichung über den vorhandenen Resampler kontrolliert ausgleichen,
   ohne Capture-Callbacks zu blockieren.

Die konkreten Toleranzen werden im Spike aus gemessenen Windows-Geräten festgelegt.
Sie werden anschließend als dokumentierte Konstanten und Tests hinterlegt, nicht
nach Gefühl gewählt.

### Persistenzformat

Primäre Speicherung ist eine Session-Struktur mit getrennten, rollierenden
PCM16-WAV-Dateien:

```text
meetings/<meeting-id>/
  manifest.json
  audio/
    000001-microphone.wav
    000001-system.wav
    000002-microphone.wav
    000002-system.wav
  export/
  recovery/
```

Jeder Chunk wird zunächst als temporäre Datei geschrieben, sein Header beim
Abschluss finalisiert und anschließend atomar umbenannt. Ein Prozessabsturz kann
damit höchstens den aktuell offenen Chunk beschädigen, nicht die gesamte Aufnahme.

Die initiale Rollzeit ist eine testbare Konstante. Als erster technischer Wert
werden 60 Sekunden verwendet, weil dies Recovery und Speicherbegrenzung vereinfacht;
Phase 1 prüft, ob ein anderer Wert messbar besser ist. Capture-Chunks und
STT-Sprachsegmente sind ausdrücklich nicht dasselbe.

Für Azure wird aus den synchronisierten Mic-/System-Chunks ein temporäres
Stereo-Paket erstellt: links Mikrofon, rechts Systemaudio. Lokal werden die Quellen
getrennt an die STT-Queue gegeben. Ein optionaler Stereo-WAV-Export kann später aus
denselben Quelldateien erzeugt werden.

## 8. Transkriptionspipeline

### Gemeinsames Segmentmodell

Ein STT-Segment enthält mindestens:

```text
meeting_id
source: microphone | system
timeline_start_ms
timeline_end_ms
audio_chunk_ids
language
provider
provider_settings_snapshot
attempt
state
```

VAD erzeugt aus abgeschlossenen Capture-Chunks sprachbasierte STT-Segmente. Als
Ausgangspunkt dienen die im geprüften `origin/chunking` vorhandenen Grenzen von
5 bis 30 Sekunden. Diese Werte sind kein Qualitätsdogma: Sie werden für deutsches
Meeting-Audio gegen längere Segmente getestet. Eine kleine Kontextüberlappung oder
providerabhängige Bündelung darf eingeführt werden, wenn der Benchmark weniger
Wortabbrüche zeigt. Doppelte Wörter an Segmentgrenzen werden beim Merge entfernt.

### Lokale Verarbeitung

- genau ein Meeting-STT-Worker least die vorhandene `TranscriptionManager`-Engine;
- Mic- und Systemsegmente werden nach Timeline und Fairness serialisiert;
- Aufnahme-Threads warten niemals auf Inferenz;
- Ergebnisse werden unmittelbar nach jedem Segment in SQLite geschrieben;
- das chronologische Transkript wird aus persistierten Segmenten abgeleitet;
- ein fehlgeschlagener Job kann einzeln wiederholt werden;
- nach Neustart werden `queued` und unterbrochene `running`-Jobs wieder aufgenommen.

Das Meeting-Modell ist eine eigene Einstellung und ändert nicht automatisch das
Diktiermodell. Welches Modell als Standard vorgeschlagen wird, entscheidet ein
Benchmark mit den bereits vorhandenen Kandidaten, insbesondere Parakeet V3,
Whisper Large, Whisper Turbo und weiteren installierbaren Modellen.

Bewertet werden:

- Wortfehlerrate auf repräsentativen deutschen Calls;
- Namen, Zahlen und projektspezifische Begriffe;
- Halluzinationen bei Stille und Störgeräuschen;
- Wortabbrüche an Segmentgrenzen;
- Echtzeitfaktor auf der tatsächlichen Zielhardware;
- RAM-/VRAM-Bedarf;
- Verhalten bei gemischtem Deutsch/Englisch.

Erst das Messergebnis setzt den UI-Standard. Nutzer können weiterhin ein anderes
Meeting-Modell auswählen.

### Ressourcenlogik zwischen Meeting und Diktat

Der Diktiermodus ist interaktiv und erhält Priorität:

1. Eine laufende Meetingaufnahme wird nie unterbrochen.
2. Ein lokaler Meeting-STT-Job darf an einer sicheren Segmentgrenze pausieren.
3. Diktat lädt/verwendet sein Modell und liefert das Ergebnis.
4. Danach setzt der Meeting-Worker seine Queue fort.

Wenn Meeting- und Diktiermodus verschiedene große Modelle nutzen, darf das System
nicht beide ungeprüft parallel laden. Modellwechsel, Leases und Abbruchpunkte werden
zentral im `TranscriptionManager` koordiniert. Die UI zeigt einen wachsenden Backlog,
statt Ressourcenengpässe zu verbergen.

### Azure-Verarbeitung

Der vorhandene Short-Audio-Pfad bleibt für kurze Diktate bestehen. Der Meeting-Modus
bekommt einen separaten `AzureMeetingClient` auf Basis der Fast-Transcription-API.

Gründe:

- der aktuelle Short-Audio-Endpunkt erlaubt maximal 60 Sekunden;
- Fast Transcription ist für Aufzeichnungen und Meeting-Notizen vorgesehen;
- es liefert Segment- und Wortzeitstempel;
- es unterstützt ein oder zwei Kanäle und kann Stereo-Kanäle unabhängig
  transkribieren;
- eine Datei muss unter fünf Stunden und unter 500 MB liegen.

Speak lädt nicht erst nach mehreren Stunden eine einzige Datei hoch. Abgeschlossene
Capture-Chunks werden zu begrenzten Azure-Auftragspaketen gebündelt. Paketdauer und
Parallelität sind konfigurierbare interne Werte, die anhand Latenz, Kontextqualität,
Uploadkosten und Retry-Verhalten getestet werden. Jedes Paket bleibt klar unter den
offiziellen Grenzen.

Azure-Jobs besitzen:

- idempotente lokale Job-ID;
- Timeout und begrenzte Exponential-Backoff-Wiederholung;
- persistierten Versuchszähler und letzte Fehlermeldung;
- Erkennung dauerhafter Authentifizierungs-/Konfigurationsfehler;
- Zeitstempelkorrektur relativ zur Meeting-Timeline;
- lokale Audiodaten bis zum bestätigten und gespeicherten Ergebnis.

Für v1 ist kein Azure-Echtzeitstream notwendig. Sollte später tatsächlich
Wort-für-Wort-Liveausgabe benötigt werden, wird ein eigener Spike für das offizielle
Speech SDK durchgeführt. Ein undokumentiertes WebSocket-Protokoll wird nicht zur
Basis der Anwendung gemacht.

### Custom STT

Der vorhandene Custom-Endpunkt wird nicht automatisch als langzeitfähig betrachtet.
Meeting-Jobs senden begrenzte Audiosegmente nach dem dokumentierten vorhandenen
Multipart-Vertrag. Timeout, Retry, Authentifizierung und Antwortvalidierung verwenden
dieselbe Joblogik wie Azure. Providerbesonderheiten bleiben hinter einem gemeinsamen
Meeting-STT-Interface.

## 9. Datenmodell

Die bestehende `transcription_history` bleibt für Diktate erhalten. Meetings erhalten
eigene Migrationen und Tabellen. Projekte und importierte Inhalte verwenden dasselbe
SQLite-Datenbankfile, aber eigene Repository- und Migrationsebenen.

Für alle synchronisierbaren Kerndatensätze gelten schon in v1:

- stabile UUIDs, die nicht von SQLite-Autoincrement abhängen;
- `created_at`, `updated_at`, optionale `deleted_at`-Löschmarker und eine monotone
  `revision`;
- Schema- und Parser-Versionen für reproduzierbare Migrationen;
- Inhalts-Hashes für Dateien und normalisierte Quellenelemente;
- Fremdschlüssel, Transaktionen und eindeutige Constraints statt nur in JSON
  versteckter Beziehungen;
- keine absoluten Dateipfade als dauerhafte Identität; Datenbank speichert Asset-ID
  und relativen Storage-Key.

Das ist noch keine Synchronisation und kein CRDT. Es schafft lediglich eine
eindeutige Änderungsbasis für den späteren Server.

### `meeting_sessions`

- `id` als stabile UUID;
- `title`;
- `started_at`, `ended_at`;
- `state`;
- `stt_provider` und Snapshot der nicht geheimen Providerkonfiguration;
- `meeting_model_id` für lokale Verarbeitung;
- `language`;
- verwendete Mikrofon- und Output-Geräte;
- `has_microphone_audio`, `has_system_audio`;
- `recording_retention_policy`;
- `summary_provider`, `summary_state`;
- `last_error`;
- `created_at`, `updated_at`.

### `meeting_audio_chunks`

- `id`, `meeting_id`, `sequence`;
- `source`;
- `timeline_start_ms`, `timeline_end_ms`;
- `path`, `sample_rate`, `sample_count`;
- `file_size`, Prüfsumme;
- `state`: `writing`, `ready`, `damaged`, `deleted`;
- Gerätekennung und Gap-/Drift-Metadaten.

### `meeting_stt_jobs`

- `id`, `meeting_id`;
- referenzierte Chunk-IDs;
- `source` oder Azure-Kanalzuordnung;
- `timeline_start_ms`, `timeline_end_ms`;
- `provider`, Konfigurationssnapshot ohne Secret;
- `state`: `queued`, `running`, `retry_wait`, `succeeded`, `failed`, `cancelled`;
- `attempt_count`, `next_attempt_at`, `last_error`;
- `created_at`, `updated_at`.

### `meeting_transcript_segments`

- `id`, `meeting_id`, `job_id`;
- `source`;
- `start_ms`, `end_ms`;
- `text`, optional Wortzeitstempel und Confidence;
- `provider`, `model_id`;
- `sequence`, `created_at`.

### `meeting_summaries`

- `id`, `meeting_id`;
- `provider`, `model`, Prompt-Version;
- `state`;
- `summary_markdown`;
- strukturierte `decisions`, `action_items`, `open_questions` als JSON;
- `last_error`, `created_at`, `updated_at`.

### `projects`

- `id`, `name`, optionale `description` und `color`;
- `state`: `active`, `archived`;
- `created_at`, `updated_at`, `deleted_at`, `revision`;
- optionale manuell gepflegte Statusnotiz, die KI-Ausgaben nicht überschreiben
  dürfen.

### `sources`

Generischer Ursprungsdatensatz für `meeting`, `whatsapp_export`, `manual_note`,
`file` und spätere Typen wie `email` oder `whatsapp_api`:

- `id`, `source_type`, `title`, `occurred_at`, optionaler Zeitraum;
- `ingestion_state`, `parser_version`, `content_hash`;
- `raw_asset_id`, Import- und Datenschutzmetadaten;
- `created_at`, `updated_at`, `deleted_at`, `revision`.

Jede `meeting_session` besitzt nach erfolgreicher Erstellung genau eine zugehörige
`source`. Der Source-Datensatz referenziert das Meeting fachlich, ohne Audio- und
STT-Tabellen zu duplizieren.

### `project_sources`

- `project_id`, `source_id`;
- `is_primary` für die in v1 sichtbare Hauptzuordnung;
- `origin`: `manual`, `ai_suggested`, `import_default`;
- `confidence`, `confirmed_at`, `created_at`;
- eindeutiger Schlüssel auf Projekt und Quelle.

Die Beziehung ist absichtlich viele-zu-viele. v1 zeigt normalerweise ein
Hauptprojekt, kann später aber bereichsübergreifende Meetings ohne Datenkopie
abbilden.

### `source_items`

Normalisierte, adressierbare Einheiten wie Chatnachricht, Transkriptsegment,
Dokumentabschnitt oder Notiz:

- `id`, `source_id`, `item_type`, `external_key`;
- `author_label`, `occurred_at`, `sequence`;
- `text`, strukturierte Metadaten und `content_hash`;
- `created_at`, `updated_at`, `deleted_at`, `revision`;
- eindeutige Deduplizierungs-Constraints pro Quelle und externem Schlüssel/Hash.

Meeting-Transkriptsegmente bleiben wegen ihrer Audio-Zeitachse in der spezialisierten
Tabelle und werden über eine stabile Referenz als `source_item` durchsuch- und
belegbar gemacht.

### `assets` und `source_item_assets`

- `assets`: `id`, Storage-Key, MIME-Typ, Dateigröße, Hash, ursprünglicher Dateiname,
  Breite/Höhe/Dauer soweit erkannt, Verarbeitungsstatus und Aufbewahrungsregel;
- `source_item_assets`: Zuordnung eines Mediums oder Dokuments zu einer oder mehreren
  normalisierten Nachrichten/Quellen;
- keine Binärdaten als große SQLite-BLOBs; SQLite enthält Metadaten, der AssetStore
  die Dateien.

### `project_tasks`

- `id`, `project_id`, `title`, optionale Beschreibung;
- `state`: in v1 `suggested`, `open`, `done`, `dismissed`; spätere Ausführungen
  ergänzen `in_progress`, `waiting_for_approval`, `failed` und `cancelled`;
- `priority`: `low`, `normal`, `high`, `urgent`;
- optionales Fälligkeitsdatum und Verantwortlicher;
- `owner_type`: `unassigned`, `human`, später `assistant`; die UI verwendet in v1
  nur `unassigned` und `human`;
- `origin`: `human` oder `ai`, `confidence`, `model`, Prompt-Version;
- `human_edited_at`, `completed_at`, `created_at`, `updated_at`, `deleted_at`,
  `revision`.

Ausführbarkeit wird nicht als dauerhaftes Wahrheitsfeld in der Aufgabe gespeichert.
Sie hängt später von aktuell installierten Fähigkeiten, verbundenen Konten,
Berechtigungen und möglichen Sicherheitsregeln ab und wird deshalb zur Laufzeit
bewertet.

### `task_evidence`

- `task_id`, `source_id`, optional `source_item_id` oder
  `meeting_transcript_segment_id`;
- optionaler Zeitstempelbereich, kurzer Belegauszug und Erklärungsgrund;
- ein KI-Vorschlag ohne mindestens einen gültigen Beleg wird nicht als Aufgabe
  veröffentlicht.

### `project_snapshots` und `snapshot_evidence`

- versionierter Projektstatus mit `summary_markdown`, Entscheidungen, Risiken und
  offenen Fragen;
- `covered_through`, beteiligte Quellenrevisionen, Provider, Modell und
  Prompt-Version;
- `snapshot_evidence` verbindet einzelne Aussagen mit Quellen oder
  Quellenelementen;
- nur ein Snapshot ist `current`, ältere bleiben für Nachvollziehbarkeit erhalten.

### `processing_jobs`

Generische persistente Aufträge für Import, Medienanalyse, Quellensummary,
Projekt-Snapshot und spätere Connector-Synchronisation. Die Zustands- und
Retry-Semantik entspricht `meeting_stt_jobs`; ein idempotency key verhindert
doppelte Ergebnisse.

### `change_log`

Kleine, transaktional geschriebene Änderungszeilen mit Entität, ID, Revision und
Zeitpunkt. In v1 dient das Log der inkrementellen Projektanalyse und Diagnose. Später
kann ein Sync-Client darauf aufbauen; es enthält keine Secrets und keine komplette
Kopie großer Inhalte.

### Suche

SQLite FTS5 indexiert Projektnamen, Source-Items, Transkripte, Aufgaben und
Snapshots. Suchtreffer liefern immer Entity-ID und Belegposition. Eine spätere
Embedding-/Vektorsuche liegt hinter einer `SearchService`-Schnittstelle, wird aber
nicht vorgezogen, solange Volltextsuche den realen Bedarf erfüllt.

Secrets werden ausschließlich über die vorhandene Secret-Verwaltung referenziert und
niemals in Meeting-, Projekt-, Importtabellen, Manifesten, Events oder Logs
geschrieben.

## 10. Zusammenfassung

Die Summary-Auswahl ist unabhängig von der STT-Auswahl. Zulässige Kombinationen sind
zum Beispiel:

- lokale STT + lokale Zusammenfassung;
- lokale STT + Cloud-Zusammenfassung;
- Azure STT + lokale Zusammenfassung;
- Azure STT + Cloud-Zusammenfassung;
- Zusammenfassung deaktiviert.

Für lange Meetings wird Map-Reduce verwendet:

1. Transkript in zeitlich zusammenhängende Textfenster teilen.
2. Pro Fenster Fakten, Entscheidungen, Aufgaben und offene Punkte extrahieren.
3. Zwischenergebnisse persistieren.
4. Abschließend deduplizieren und zu einem Gesamtprotokoll zusammenführen.

Das Modell erhält Zeitstempel und die technisch sichere Quelleninformation, aber keine
erfundenen Personennamen. Aufgaben ohne eindeutig genannten Verantwortlichen bleiben
`Verantwortlich: nicht erkannt`.

Transkripttext wird als nicht vertrauenswürdiger Inhalt klar vom Systemprompt
getrennt. Gesprochene Sätze wie „Ignoriere alle vorherigen Anweisungen“ dürfen die
Summary-Anweisungen nicht überschreiben.

Standardausgabe:

```markdown
# Zusammenfassung

## Kernaussagen

## Entscheidungen

## Aufgaben

## Offene Fragen

## Zeitlich gegliederter Verlauf
```

### Projektintelligenz

Projektstatus und Aufgaben werden nicht bei jeder neuen Nachricht über den gesamten
Datenbestand neu berechnet. Die Pipeline arbeitet inkrementell:

```text
neue/geänderte Source-Items
  -> Quellenauszug mit Belegen
    -> Abgleich mit aktuellem Projekt-Snapshot und offenen Aufgaben
      -> neuer versionierter Snapshot + Aufgabenvorschläge
```

- Die Oberfläche zeigt `Aktualisiert bis …` und den Verarbeitungszustand.
- Alte und neue Aufgaben werden über stabile IDs und semantische Merkmale
  abgeglichen; ein Retry erzeugt keine Duplikate.
- Nutzeränderungen sind geschützte Fakten. Die KI darf sie kommentieren, aber weder
  Abschlussstatus noch Priorität unbemerkt zurücksetzen.
- Widersprüchliche Quellen werden als Konflikt sichtbar gemacht und nicht zu einer
  scheinbar sicheren Wahrheit zusammengezogen.
- Importierter Text und Dateien sind nicht vertrauenswürdige Daten. Prompt
  Injection, versteckte Instruktionen in Bildern und Dateimetadaten dürfen keine
  Tools, Providerwechsel oder Datenfreigaben auslösen.
- Cloudmodelle erhalten nur die für den jeweiligen Auftrag nötigen Chunks und
  Belege, nicht automatisch das gesamte Projektarchiv.

## 11. Meeting-Bibliothek und Export

Die Seitenleiste erhält einen Bereich `Meetings`. Die Liste zeigt Titel, Datum,
Dauer, Projekt, Provider, Verarbeitungszustand und Fehlerstatus.

Die Detailansicht enthält:

- Zusammenfassung;
- vollständiges chronologisches Transkript;
- Suche im Transkript;
- Sprung zu Zeitpositionen, solange Audio vorhanden ist;
- erneute Transkription mit einem anderen Modell/Provider;
- erneute Zusammenfassung;
- Kopieren der Zusammenfassung oder des Transkripts;
- Export als Markdown, TXT und JSON;
- optionalen Audioexport;
- kontrolliertes Löschen.
- Projekt zuordnen, wechseln oder aus der Zuordnung lösen;
- von einer extrahierten Aufgabe zurück zur belegenden Zeitposition springen.

Beim Löschen unterscheidet die UI klar zwischen:

- nur Rohaufnahme löschen;
- Transkript und Zusammenfassung behalten;
- gesamtes Meeting endgültig löschen.

Vor irreversibler Löschung werden die konkreten Bestandteile angezeigt und bestätigt.
Wird ein Meeting aus einem Projekt gelöst, wird es nicht gelöscht, sondern erscheint
im Eingang. Verweise aus bereits bestätigten Aufgaben werden dabei als zu prüfende
Belege markiert und nicht still entfernt.

## 12. Speicher- und Datenschutzlogik

PCM16 mit 16 kHz benötigt pro Mono-Spur ungefähr 115 MB pro Stunde, bei zwei Spuren
ungefähr 230 MB pro Stunde. Temporäre Azure-Pakete und Exporte können den Bedarf
kurzzeitig erhöhen. Der Startdialog zeigt deshalb den Bedarf pro Stunde, den freien
Speicher und die daraus grob mögliche Aufnahmedauer, ohne den Nutzer vor jedem
Meeting nach einer geplanten Dauer zu fragen.

Standard-Aufbewahrung für neue Nutzer:

- Audio bleibt während Aufnahme, Transkription und Zusammenfassung erhalten;
- nach vollständig erfolgreicher Verarbeitung fragt Speak beim ersten Meeting nach
  der gewünschten Standardregel;
- Optionen: `sofort löschen`, `7 Tage`, `30 Tage`, `dauerhaft behalten`;
- ohne beantwortete Auswahl wird nichts automatisch gelöscht;
- fehlgeschlagene oder noch nicht bestätigte STT-Jobs verhindern automatische
  Audiolöschung.

Bei lokaler STT und lokaler Zusammenfassung verlässt kein Audio oder Transkript den
Rechner. Bei Azure oder einem Cloud-LLM zeigt die UI vor dem Start und während der
Verarbeitung klar, welche Daten an welchen Provider gesendet werden.

Eine Aufnahme nichtöffentlicher Gespräche erfordert die notwendige Einwilligung.
Speak zeigt daher während der gesamten Aufnahme einen sichtbaren Indikator und bietet
vor dem Start einen Consent-Hinweis. Das Feature wird nicht als Stealth-Recorder
gestaltet.

WhatsApp-Archive und alle importierten Dateien gelten als untrusted input. Vor dem
Entpacken werden Zielpfade normalisiert; Pfadtraversal, absolute Pfade und Symlinks
werden abgewiesen. Begrenzungen für Dateianzahl, Einzelgröße, Gesamtgröße und
Kompressionsverhältnis verhindern ZIP-Bombs. Beschädigte oder verschlüsselte Archive
werden ohne Teilimport erklärt. Erst nach erfolgreicher Prüfung werden Dateien aus
einem isolierten temporären Verzeichnis in den AssetStore übernommen.

Für jeden Import ist im Nachhinein sichtbar:

- welche Originaldatei importiert wurde;
- welche Elemente lokal verarbeitet wurden;
- welche Inhalte wann an welchen Cloudprovider gingen;
- welcher Parser, welches Modell und welche Prompt-Version verwendet wurden;
- welche Aufbewahrungsregel für Original und extrahierte Assets gilt.

## 13. Implementierungsphasen

Jede Phase endet mit Tests und einem eigenen konventionellen Commit. Die nächste
Phase beginnt erst, wenn ihre Abnahmekriterien erfüllt sind.

### Phase 0 — Arbeitsstand sichern und Testbasis schaffen

Aufgaben:

- vorhandene uncommittete Arbeit auf `feat/custom-stt-pipeline` prüfen und als
  Checkpoint sichern;
- aktuellen Diktierpfad mit Local, Azure Short Audio und Custom STT testen;
- relevante Lint-, Format- und Backendtests dokumentieren;
- Test-Audio und manuelle Windows-Testmatrix anlegen.

Abnahme:

- keine bestehenden Änderungen gehen verloren;
- Diktier-Baseline ist reproduzierbar;
- Meeting-Code kann später eindeutig vom aktuellen Speech-Pipeline-Umbau getrennt
  geprüft werden.

### Phase 1 — Windows-Loopback-Spike

Aufgaben:

- Output-Gerät über CPAL als Input-Stream öffnen;
- Mikrofon und Loopback parallel 120 Minuten mitschneiden;
- Zeitstempel, Paketlücken und Sample-Drift protokollieren;
- Stille, Gerätetausch, Sleep/Wake und Shared-Mode-Fehler testen;
- CPAL gegen direkte WASAPI-Anbindung bewerten.

Abnahme:

- Systemaudio wird auf der Zielhardware zuverlässig erfasst;
- gemessene Gap-/Drift-Werte und Korrekturgrenzen sind dokumentiert;
- die Technologieentscheidung CPAL oder direkte WASAPI ist durch Messdaten belegt;
- der bestehende Mikrofon-Diktierpfad wurde nicht verändert.

### Phase 2 — Dauerhafter Dual-Source-Recorder

Aufgaben:

- `MeetingRecorder` mit zwei unabhängigen Capture-Streams;
- gemeinsame monotone Zeitachse;
- Resampling, Gap-Filling und Drift-Korrektur;
- rotierende, atomar finalisierte Mono-WAV-Chunks;
- Pegel- und Zustandsereignisse;
- kontrolliertes Stoppen bei Disk-/Gerätefehlern;
- Recovery eines unterbrochenen offenen Chunks.

Abnahme:

- RAM-Verbrauch wächst nicht linear mit der Meetingdauer;
- beide Quellen bleiben über einen zweistündigen Test zeitlich stabil;
- Ausfall einer Quelle zerstört die andere nicht;
- nach erzwungenem Prozessabbruch sind alle abgeschlossenen Chunks nutzbar.

### Phase 3 — Domain-/Storage-Fundament und App-Shell

Aufgaben:

- klickbaren UX-Prototyp erstellen und die Anfängeraufgaben aus Abschnitt 6.15
  und 6.21 testen;
- App-Shell von einer reinen Einstellungsnavigation auf `Diktieren`, `Meetings`,
  `Projekte`, `Diktatverlauf` und `Einstellungen` umbauen;
- responsive Fensterlogik, Maximieren/Vollbild und Wiederherstellung der
  Fensterposition;
- SQLite-Migrationen, `MeetingStore`, `KnowledgeStore` und Repository-Grenzen;
- `AssetStore` mit relativen Storage-Keys, Hashing und atomarem Commit;
- stabile IDs, Revisionen, Löschmarker, `change_log` und FTS5-Suche;
- Migrationstests auf einer Kopie bestehender Nutzerdaten.

Abnahme:

- Diktieren, Meetings und Projekte sind klar getrennte Hauptbereiche derselben App;
- Layout ist in kleinem Fenster, maximiert und im Vollbild vollständig bedienbar;
- bestehende Diktat-History bleibt unverändert und lesbar;
- kein Frontendcode greift direkt auf SQLite oder Assetdateien zu;
- Schema und Servicegrenzen tragen lokale v1 und die beschriebenen späteren Clients,
  ohne dass Server- oder Sync-Code implementiert wird.

### Phase 4 — Meeting-Session und Aufnahme-UI

Aufgaben:

- `MeetingManager`-Zustandsmaschine;
- Tauri-Commands und Events;
- Setup-, Recording-, Processing-, Detail- und Listenansicht;
- getrennte Diktier-/Meeting-Einstellungen;
- kollisionsgeprüfter Meeting-Shortcut, Countdown, Stopbestätigung und eigener
  Tray-Eintrag;
- Geräte-, Speicher- und Consent-Prüfungen;
- Start, Pause, Fortsetzen, Stop und Crash-Recovery.

Abnahme:

- ein Meeting kann vollständig aufgenommen, pausiert, fortgesetzt und wieder geöffnet
  werden;
- UI und persistierter Zustand stimmen nach Neustart überein;
- alle neuen sichtbaren Texte verwenden i18n;
- Diktat und Meeting sind im Hauptfenster, Tray, Overlay und Shortcut-Editor nicht
  verwechselbar;
- alle Anfänger- und Usability-Gates aus Abschnitt 6.15 sind erfüllt;
- der Diktiermodus verhält sich unverändert.

### Phase 5 — Lokale Meeting-STT

Aufgaben:

- VAD-Segmentierung und persistente STT-Queue;
- genau ein Engine-Lease;
- Prioritätslogik zugunsten kurzer Diktate;
- Mikrofon-Tap für Diktate während einer laufenden Meetingaufnahme;
- segmentweise Speicherung und chronologischer Merge;
- Retry, Abbruch und Neustart-Wiederaufnahme;
- Benchmark der vorhandenen starken Modelle auf deutschem Meeting-Audio;
- Meeting-Modellwahl in UI und Settings.

Abnahme:

- ein zweistündiges Meeting wird ohne wachsenden Audio-RAM-Verbrauch verarbeitet;
- Transkriptsegmente erscheinen schrittweise und bleiben nach Neustart erhalten;
- Diktate bleiben während einer Meetingaufnahme nutzbar, ohne eine zweite
  Mikrofoninstanz zu öffnen oder das Meeting zu pausieren;
- der vorgeschlagene Default ist durch gespeicherte Benchmarkresultate begründet;
- Segmentgrenzen erzeugen keine systematischen Doppelungen oder Wortverluste.

### Phase 6 — Azure Fast Transcription und Custom STT

Aufgaben:

- separater `AzureMeetingClient`;
- Aufbau synchronisierter Stereo-Auftragspakete;
- unabhängige Kanaltranskription und Rückführung auf die Meeting-Timeline;
- persistente Cloud-Jobs, Timeouts, Backoff und Fehlerklassifikation;
- klare Cloud-Anzeige und Secret-Behandlung;
- Custom-STT-Adapter auf derselben Jobabstraktion;
- Netzwerkabbruch- und Authentifizierungstests.

Abnahme:

- Azure verarbeitet ein mehrstündiges Meeting abschnittsweise;
- temporärer Netzausfall verliert keine Audioabschnitte;
- permanente Konfigurationsfehler werden verständlich angezeigt und nicht endlos
  wiederholt;
- erfolgreich gespeicherte Antworten werden nicht doppelt in das Transkript
  eingefügt;
- Local, Azure und Custom können pro Meeting gewählt werden, ohne den Diktierprovider
  zu verändern.

### Phase 7 — Meeting-Zusammenfassung, Bibliothek und Export

Aufgaben:

- lokaler/Cloud-Summary-Provider;
- Map-Reduce und persistierte Zwischenergebnisse;
- strukturierte Entscheidungen, Aufgaben und offene Fragen;
- Detailansicht, Suche, Kopieren und Export;
- Neu-Transkription und Neu-Zusammenfassung;
- Aufbewahrungs- und Löschlogik.

Abnahme:

- lange Transkripte überschreiten nicht unkontrolliert das Kontextfenster;
- Neuversuche duplizieren keine Aufgaben oder Zwischenergebnisse;
- lokale Verarbeitung sendet nachweislich keine Nutzdaten an Cloud-Provider;
- Löschung entfernt exakt die in der Bestätigung genannten Dateien und Datensätze.

### Phase 8 — Projekte und Meeting-Zuordnung

Aufgaben:

- Projektliste, Ein-Klick-Projekterstellung und Projekt-Detailansicht;
- Eingang für nicht zugeordnete Meetings und Quellen;
- optionaler Projekt-Chip im Meetingstart;
- manuelle Zuordnung, Neuzuordnung und viele-zu-viele-Datenbeziehung;
- KI-Projektvorschlag mit Confidence, Belegen und Bestätigung;
- Aufgabenansicht mit deterministischer Sortierung und geschützten
  Nutzeränderungen;
- projektbezogene FTS5-Suche und Sprung zum Quellenbeleg.

Abnahme:

- ein Meeting kann vor dem Start, danach und aus dem Eingang einem Projekt
  zugeordnet werden;
- ein Meetingwechsel zwischen Teams, WhatsApp, Skype, Discord oder Browser während
  derselben laufenden Aufnahme erzeugt weiterhin genau eine Session und verliert
  keine Audioabschnitte;
- KI ordnet niemals ohne sichtbare Bestätigung um;
- Status- und Aufgabenansichten zeigen keine Aussage ohne erreichbaren Beleg;
- Projektverknüpfung dupliziert weder Meeting noch Transkript.

### Phase 9 — WhatsApp-Import und multimodale Verarbeitung

Aufgaben:

- sichere ZIP/TXT-Prüfung und isoliertes Entpacken;
- parserbasierte Erkennung von Sprach-, Datums- und Formatvarianten;
- Vorschau mit Teilnehmern, Zeitraum, Nachrichten, Medien und Warnungen;
- Normalisierung, Medienzuordnung, Hash-Deduplizierung und idempotenter Reimport;
- lokale Volltextindexierung ohne Cloudzwang;
- `SourceIngestionService` und persistente Import-/Analysejobs;
- OpenRouter-Modellfilter nach Eingabemodalität;
- explizite Datenfreigabe, Kostenschätzung und selektiver multimodaler Upload;
- Fehler- und Recovery-Tests mit beschädigten, großen und unvollständigen Archiven.

Abnahme:

- derselbe oder überlappende Export erzeugt keine sichtbaren Dubletten;
- Pfadtraversal, Symlinks und Dekompressionsbomben erreichen den AssetStore nicht;
- Text bleibt ohne Cloudkonto lokal importierbar und durchsuchbar;
- vor jedem Cloudlauf sieht der Nutzer Provider und betroffene Datentypen;
- nicht unterstützte Medien bleiben erhalten und werden nicht als analysiert
  ausgegeben.

### Phase 10 — Projektstatus und KI-Aufgaben

Aufgaben:

- inkrementelle Quellenauszüge und versionierte Projekt-Snapshots;
- Extraktion von Entscheidungen, Risiken, offenen Fragen und Aufgaben;
- Belegtabellen und Sprünge zu Meeting-/Chatstellen;
- Vorschlag-, Annahme-, Bearbeiten-, Verwerfen- und Merge-Workflow für Aufgaben;
- Schutz manueller Änderungen, Konflikterkennung und Prompt-Injection-Tests;
- lokaler/Cloud-Provider pro Projektlauf und persistierte Prompt-/Modellversion.

Abnahme:

- ein Projekt zeigt nachvollziehbar, bis zu welchem Stand es ausgewertet wurde;
- jede KI-Aussage und Aufgabe hat mindestens einen gültigen Beleg;
- Neuverarbeitung erzeugt keine Aufgabendubletten und überschreibt keine
  Nutzerentscheidung;
- lokale Verarbeitung sendet keine Projektinhalte an externe Provider;
- widersprüchliche Quellen werden sichtbar statt erfunden eindeutig dargestellt.

### Phase 11 — Härtung und Release-Gate

Aufgaben:

- mindestens zweistündige Soak-Tests mit Teams, WhatsApp und Browser-Calls;
- Gerätewechsel, Sleep/Wake, Stille, Lautsprecher-Echo und Benachrichtigungen;
- langsame Festplatte, wenig Speicher und Prozessabbruch;
- Offline-/Online-Wechsel bei Azure;
- CPU-, RAM-, GPU- und Disk-Profiling;
- Accessibility, Tastaturbedienung und i18n-Prüfung;
- Datenschutztexte und sichtbarer Aufnahmeindikator;
- große und überlappende WhatsApp-Exporte, viele Projekte und FTS-Suchlast;
- Tastatur-, Screenreader- und Responsive-Tests für alle Projektabläufe;
- dokumentierter Rollback-/Feature-Flag-Pfad.

Abnahme:

- keine bekannte lineare Speicherzunahme;
- keine still verlorenen Chunks oder Jobs;
- alle Fehlerzustände besitzen eine verständliche Nutzeraktion;
- der bestehende Diktierpfad besteht seine Baseline erneut;
- Meeting- und Projektfunktionen bleiben hinter getrennten Feature-Flags, bis ihre
  jeweiligen Gates erfüllt sind.

### Zukunftsroadmap — nicht Teil von v1

- Windows-Per-Prozess-Loopback;
- macOS-Systemaudio über CoreAudio Process Tap oder ScreenCaptureKit;
- Linux-Systemaudio über PipeWire/PulseAudio-Monitorquellen;
- echtes Live-STT, falls ein konkreter Bedarf die zusätzliche Komplexität rechtfertigt;
- weitere Exportformate;
- zentraler Server mit API, Datenbank und verschlüsseltem Objektspeicher;
- Desktop-Synchronisation mehrerer Geräte;
- iPhone-App für Aufnahme, Upload, Projektansicht und Aufgaben;
- direkte, autorisierte WhatsApp- und E-Mail-Connectoren über dieselbe
  Ingestion-Schnittstelle;
- integrierter Speak-Assistent mit Aufgabenübernahme, Fähigkeitenprüfung,
  Ausführungsplan und einzelnen Freigaben;
- lokaler und später Remote-MCP-Server für Codex und andere freigegebene Clients;
- optionale Embedding-/Vektorsuche, wenn FTS5 in realen Daten nicht ausreicht.

Diarisierung, Namenserkennung und Teilnehmer-Scraping bleiben auch hier außerhalb des
Plans, bis sie ausdrücklich neu beauftragt werden.

#### Spätere Server- und Sync-Grenze

Der Server wird nicht als gemeinsam genutztes Dateiverzeichnis modelliert. Er erhält
eine fachliche API über Projekte, Quellen, Aufgaben, Snapshots und Assets. Binärdaten
liegen in verschlüsseltem Objektspeicher; Metadaten und Beziehungen in einer
Serverdatenbank. Clients synchronisieren Änderungen über UUID, Revision,
`updated_at`, Löschmarker und Inhalts-Hash. Konfliktregeln und Ende-zu-Ende-
Verschlüsselung werden vor der Serverphase gesondert entschieden und nicht in v1
vorgetäuscht.

#### Späterer MCP-Zugriff

MCP ist nur ein zusätzlicher externer Zugang. Der primäre Assistent lebt direkt in
Speak. MCP verwendet dieselbe Service-Schicht und startet read-only mit kleinen,
belegorientierten Werkzeugen:

- `list_projects`;
- `get_project_status`;
- `list_project_tasks`;
- `search_project_sources`;
- `get_meeting`.

Schreibwerkzeuge folgen erst mit Bestätigung, Audit-Log und minimalen Berechtigungen.
Ein entfernter HTTP-MCP-Server benötigt standardisierte Authentifizierung,
projektbezogene Scopes und widerrufbare Tokens. Kein Modell erhält standardmäßig
Zugriff auf alle Projekte oder Roh-Assets.

#### Späterer integrierter Speak-Assistent

Speak erhält in einer eigenen Zukunftsphase einen projektbezogenen Assistenten direkt
in der App. Er ist nicht bloß ein Chatfenster und nicht von MCP abhängig. Er kennt
das geöffnete Projekt, dessen freigegebene Quellen, Aufgaben und die aktuell
verfügbaren Fähigkeiten.

Beim Anklicken einer Aufgabe öffnet sich ein Detail-Drawer mit:

- vollständiger Beschreibung, Priorität, Termin und aktuellem Besitzer;
- den ursprünglichen Belegen aus Meeting, Chat oder Datei;
- verwandten Entscheidungen und offenen Fragen;
- Verlauf aller manuellen und KI-Änderungen;
- genau zwei primären Verantwortungsoptionen: `Ich übernehme` und – nur wenn
  tatsächlich möglich – `Speak übernehmen lassen`.

Die Aufgabenliste zeigt die Ausführbarkeit sofort, aber niemals ausschließlich durch
Farbe:

| Anzeige | Bedeutung |
| --- | --- |
| Blau + Person-Symbol + `Für mich` | sinnvoll oder zwingend durch den Nutzer zu erledigen |
| Violett + Spark-/Agent-Symbol + `Speak kann helfen` | Assistent kann einen Entwurf, Recherche oder Teilarbeit liefern |
| Grün + Agent-Symbol + `Speak kann erledigen` | vollständig ausführbar; erforderliche Fähigkeiten sind verbunden |
| Orange + Schloss-Symbol + `Freigabe/Verbindung nötig` | grundsätzlich möglich, aber Konto, Berechtigung oder Bestätigung fehlt |
| Grau + Fragezeichen + `Erst klären` | Ziel oder Erfolgskriterium ist nicht eindeutig genug |

Die Bewertung stammt aus einem `CapabilityRegistry`-Service und berücksichtigt zur
Laufzeit:

- welche internen Werkzeuge, Connectoren und Konten vorhanden sind;
- Lese- und Schreibrechte sowie Projektfreigaben;
- ob die Aufgabe nur vorbereitet oder wirklich abgeschlossen werden kann;
- notwendige Rückfragen, Kosten, Datenschutz und externe Nebenwirkungen.

`Speak übernehmen lassen` startet nie sofort eine unkontrollierte Aktion. Zuerst
zeigt der Assistent in Alltagssprache:

```text
Das kann ich übernehmen:
1. Informationen aus den Projektquellen zusammentragen
2. einen Antwortentwurf erstellen
3. nach deiner Freigabe die Nachricht versenden

Benötigt: Zugriff auf E-Mail · geschätzte Kosten · betroffene Daten
```

Der Nutzer kann den Plan ändern, nur vorbereiten lassen oder bestätigen. Externe
Schreibaktionen – etwa Nachricht senden, Termin anlegen, Datei teilen oder Daten
löschen – besitzen immer einen eigenen Approval-Schritt unmittelbar vor der Aktion.
Eine frühere allgemeine Zustimmung ersetzt diese konkrete Freigabe nicht.

Für die spätere Umsetzung werden eigene Entitäten ergänzt, nicht freie Chattexte als
Zustandsmaschine verwendet:

- `assistant_capabilities`: verfügbare Fähigkeit, Quelle, benötigte Berechtigungen
  und aktueller Verbindungsstatus;
- `task_execution_plans`: versionierter Vorschlag mit Schritten, erwarteten
  Ergebnissen, Kosten und Risiken;
- `task_execution_runs`: Zustand und Resultat eines konkreten Laufs;
- `task_execution_steps`: einzelne Toolaktion mit Input-/Outputreferenzen;
- `approvals`: präzise Freigabe, Umfang, Zeitpunkt und freigebender Nutzer;
- `assistant_audit_log`: unveränderbare, verständliche Historie der Aktionen.

Der Assistent kann daneben in einer Projektseitenleiste Fragen beantworten oder aus
einem Gespräch neue Aufgaben vorschlagen. Antworten müssen weiterhin Quellen nennen.
Er darf Projekte oder vertrauliche Daten nicht kontextübergreifend vermischen und
erhält standardmäßig nur Zugriff auf das aktuell geöffnete Projekt.

Diese Agenten-Ausführung ist nicht Teil von v1. v1 liefert bereits den Task-Drawer,
Belege und menschliche Übernahme; Capability-Bewertung, Connectoraktionen,
Ausführungspläne und Approvals werden erst implementiert, wenn die jeweiligen Tools
und Sicherheitsregeln konkret feststehen.

## 14. Testmatrix

### Automatisierte Tests

- Timeline-, Gap- und Drift-Berechnungen;
- Chunk-Rollover und atomare Finalisierung;
- Recovery beschädigter/offener Chunks;
- Session-Zustandsübergänge;
- Job-Claiming, Retry, Backoff und Idempotenz;
- chronologischer Merge und Segmentüberlappung;
- Azure-Antwortparsing und Kanalzuordnung;
- Map-Reduce-Deduplizierung;
- Retention und selektive Löschung;
- Migration bestehender Nutzer-Datenbanken;
- Serialisierung der neuen Settings und Specta-Bindings.
- Projekt-/Quellen-Fremdschlüssel, Revisionen und Löschmarker;
- FTS5-Indexaktualisierung und belegbare Suchtreffer;
- WhatsApp-Parser-Fixtures für Sprach-/Datumsvarianten und mehrzeilige Nachrichten;
- ZIP-Sicherheitsgrenzen, Medienzuordnung und idempotenter Reimport;
- Aufgabendubletten, Belegintegrität und Schutz manueller Änderungen;
- inkrementelle Projekt-Snapshots bei neuen oder gelöschten Quellen.

### Manuelle Windows-Matrix

- Teams, WhatsApp Desktop, Browser-Meet, Zoom und Discord;
- Headset, Laptoplautsprecher und Bluetooth-Gerät;
- Standardgerät sowie explizit ausgewähltes Gerät;
- 30 Sekunden Stille, lange Stille und wechselnde Gesprächsaktivität;
- Mikrofon hört Lautsprecher mit;
- Output-Gerät wird entfernt und neu verbunden;
- Standardgerät wird während der Aufnahme geändert;
- App wird hart beendet und neu gestartet;
- Azure-Key ungültig, Endpoint falsch, Netzwerk offline und Netzwerk langsam;
- lokales Modell langsamer als Echtzeit;
- Diktat wird während lokaler Meetingverarbeitung ausgelöst;
- Datenträger nähert sich der Kapazitätsgrenze.
- App klein, maximiert und Vollbild; Fensterzustand nach Neustart;
- Projekt erstellen, archivieren und Quelle neu zuordnen;
- `.txt`- sowie `.zip`-Export mit/ohne Medien und überlappendem Verlauf;
- beschädigtes, verschlüsseltes und bösartig konstruiertes ZIP;
- OpenRouter-Modell mit und ohne passende Bildeingabe-Fähigkeit;
- Cloudfreigabe abbrechen und denselben Import vollständig lokal fortsetzen;
- KI-Aufgabe bearbeiten/erledigen und danach Projekt erneut analysieren;
- Suche springt zur richtigen Chatnachricht oder Meeting-Zeitposition.

## 15. Reihenfolge der ersten tatsächlichen Arbeit

1. Aktuellen uncommitteten Branch als überprüfbaren Checkpoint sichern.
2. Phase-0-Baseline ausführen und Ergebnisse dokumentieren.
3. Nur den Windows-Loopback-Spike implementieren.
4. Spike gemeinsam bewerten; erst danach Capture-Technik festlegen.
5. Dual-Recorder und Recovery ohne STT bauen.
6. Domain-/Storage-Fundament und responsive App-Shell fertigstellen.
7. Aufnahme-UI und Meeting-Sessionlogik ergänzen.
8. Lokale STT-Queue integrieren und Modelle benchmarken.
9. Azure Fast Transcription und Custom STT integrieren.
10. Meeting-Zusammenfassung, Bibliothek und Export ergänzen.
11. Projekte, Eingang und Meeting-Zuordnung fertigstellen.
12. Sicheren WhatsApp-Import zunächst komplett lokal implementieren.
13. Capability-gefilterte multimodale OpenRouter-Verarbeitung ergänzen.
14. Projekt-Snapshots, Aufgaben und Beleglogik implementieren.
15. Gemeinsame Soak-, Security-, Accessibility- und Release-Gates abschließen.

Damit bleiben die Arbeitsschritte klein, prüfbar und einzeln rückgängig zu machen.
Kein späterer Cloud- oder UI-Teil entscheidet rückwirkend über die Sicherheit der
Audioaufnahme.

## 16. Primärquellen

- Meetily: offizielles Repository, UI-Komponenten und veröffentlichte Screenshots  
  <https://github.com/Zackriya-Solutions/meetily>
- Microsoft: WASAPI Loopback Recording  
  <https://learn.microsoft.com/en-us/windows/win32/coreaudio/loopback-recording>
- Microsoft: Application Loopback Audio Capture  
  <https://learn.microsoft.com/en-us/samples/microsoft/windows-classic-samples/applicationloopbackaudio-sample/>
- Microsoft: `AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS`  
  <https://learn.microsoft.com/en-us/windows/win32/api/audioclientactivationparams/ns-audioclientactivationparams-audioclient_process_loopback_params>
- Microsoft: Azure Short Audio REST  
  <https://learn.microsoft.com/en-us/azure/ai-services/speech-service/rest-speech-to-text-short>
- Microsoft: Azure Fast Transcription  
  <https://learn.microsoft.com/en-us/azure/ai-services/speech-service/fast-transcription-create>
- Microsoft: Azure Batch Transcription  
  <https://learn.microsoft.com/en-us/azure/ai-services/speech-service/batch-transcription>
- OpenRouter: Multimodale Eingaben und Modellfähigkeiten  
  <https://openrouter.ai/docs/guides/overview/multimodal/overview>
- OpenRouter: Bildverständnis über URL oder Base64-Daten  
  <https://openrouter.ai/docs/guides/overview/multimodal/image-understanding>
- Model Context Protocol: Autorisierung für entfernte HTTP-Server  
  <https://modelcontextprotocol.io/specification/2025-11-25/basic/authorization>
- Lokaler CPAL-0.16-Quellcode:
  `src/host/wasapi/device.rs`, Loopback-Flag für Render-Geräte.

## 17. Definition of Done für v1

v1 ist erst fertig, wenn ein Nutzer auf Windows:

1. ein mindestens zweistündiges Meeting mit Mikrofon und beliebigem Systemaudio
   aufnehmen und dabei zwischen Teams, WhatsApp, Skype, Discord oder Browser wechseln
   kann, ohne die Aufnahme neu zu starten;
2. die App während der Nachverarbeitung schließen und später ein vollständiges,
   persistiertes Transkript und optional eine Zusammenfassung öffnen kann;
3. Local, Azure oder Custom pro Meeting wählen kann und jederzeit erkennt, ob Daten
   hochgeladen werden;
4. Projekte erstellen, Meetings vorab oder nachträglich zuordnen und Unzugeordnetes
   im Eingang wiederfinden kann;
5. WhatsApp-Exporte mit Text und Medien sicher, dedupliziert und wahlweise vollständig
   lokal importieren kann;
6. ein kompatibles OpenRouter-Modell bewusst für ausgewählte multimodale Inhalte
   verwenden kann, ohne dass das gesamte Archiv ungefragt hochgeladen wird;
7. einen belegbaren Projektstatus und priorisierte KI-Aufgabenvorschläge prüfen,
   annehmen, bearbeiten oder verwerfen kann;
8. von jeder KI-Aussage zur zugrunde liegenden Meetingstelle, Chatnachricht oder
   Datei gelangt;
9. alle Kernabläufe im normalen Fenster, maximiert, im Vollbild sowie mit Tastatur
   bedienen kann;
10. bei Netzwerkfehler, kontrolliertem Neustart oder idempotentem Reimport keine
    bereits abgeschlossenen Daten verliert oder doppelt sieht.

Der Diktiermodus muss dabei funktional und hinsichtlich seiner Latenz unverändert
bleiben. Server, Mehrgerätesynchronisation, iPhone, direkte Connectoren und MCP sind
ausdrücklich keine versteckten v1-Abnahmekriterien. Dasselbe gilt für die autonome
Aufgabenausführung des integrierten Speak-Assistenten; v1 zeigt Aufgaben, Belege und
die menschliche Übernahme, führt aber keine externen Aktionen selbstständig aus.
