# Retention & automatische Löschung — Nutzer-FAQ (vertraglich begründet)

Status: **Erläuterung eines Spec Contracts — kein implementiertes Verhalten.**
Dieses Dokument erklärt in einfacher Sprache, was der
Retention-/Quota-/Change-Log-Vertrag über automatisches Löschen festlegt. Es
beschreibt **Zielverhalten eines noch nicht implementierten Vertrags**, keinen
heutigen Ist-Zustand. Der Meeting-Modus selbst ist im Repository noch nicht
implementiert (verifiziert im
[Dokumentationsindex](../meeting-mode-documentation-index.md), Abschnitt 1);
auch der heutige Diktiermodus besitzt keine der hier beschriebenen Mechaniken.

Autorität: Diese FAQ ist **rein erklärend**. Sie definiert nichts und hat im
Konfliktfall keine eigene Autorität. Gültig ist ausschließlich der Vertrag
selbst; bei Konflikten gilt die Index-Ordnung „Plan schlägt Verträge,
Verträge schlagen Kataloge, nichts schlägt den Code“.

Quelle (im Folgenden **RQC** genannt):
[docs/meeting-mode-architecture/retention-quota-contract.md](../meeting-mode-architecture/retention-quota-contract.md)
— Status dort: „spec contract — no implementation yet (Phase 3 prerequisite)“.
Weitere zitierte Verträge: Asset-Store-Vertrag (**AS**, Abschnitts-Kürzel
„AS-§n“), Disk-Guard-Vertrag (**DG**),
[Plan](../meeting-mode-plan.md) §11/§12.

**Wann gilt das alles? (Gültigkeitsbedingung)** Keine Aussage dieser FAQ ist
vor dem Abschluss der Phase-3-Speicherarbeit wirksam. Erst wenn der Vertrag
implementiert und gemäß Indexregel §1 gegen Code belegt ist, beschreiben diese
Antworten tatsächliches Verhalten. Bis dahin gilt umgekehrt dasselbe wie
immer: Es wird nichts automatisch gelöscht, weil es den Mechanismus gar nicht
gibt.

---

## F1. Was kann überhaupt jemals automatisch gelöscht werden?

Nur eine einzige Art von Ding: **die Roh-Audiodateien eines Meetings**
(`audio/*.wav` plus `export/*` dieses Meetings) — und zwar ausschließlich als
„Delete-Level 1“ durch die zwei Regeln R1 (Aufbewahrungszeitraum abgelaufen)
und R2 (Speicherdruck) (RQC §4.1, §6.1 Entscheidungsmatrix, §8 Regel 1).
Dazu kommt eine zweite, rein buchhalterische Automatik: R3 löscht alte Zeilen
in der internen `change_log`-Datenbanktabelle (älter als 180 Tage, max. 1000
Zeilen pro Durchlauf) — das sind Protokollzeilen, keine Dateien, und für den
Nutzer bewusst unsichtbar (RQC §4.1 R3, §9).

Alles andere — Transkripte, Zusammenfassungen, importierte Dateien und Medien,
ganze Meetings — wird **nie automatisch** gelöscht. Die Löschstufen 2 und 3
(„Transkript und Zusammenfassung löschen“, „gesamtes Meeting endgültig
löschen“) sind für die Automatik technisch unerreichbar und existieren nur
hinter nutzerbestätigten Dialogen (RQC §6.1, §8 Regel 1; Plan §11).

## F2. Wann entsteht überhaupt Speicherdruck, und was passiert dann?

Die Schwelle liegt bewusst sehr hoch. Die harte Quote entspricht **100 Stunden
Doppelstreckenaudio** (Mikrofon + Systemaufnahme zusammen, 230.400.000 Bytes
pro Stunde) ≈ 23.040.000.000 Bytes (~21,5 GiB); darunter passiert nie etwas
(RQC §4.2 `AUDIO_QUOTA_HARD_BYTES` inklusive Herleitung). Ein einzelnes
maximal modelliertes 10-Stunden-Meeting belegt nur ein Zehntel dieser
Schwelle.

Erst wenn auch nach allen ohnehin fälligen Aufräum-Löschungen (Regel R1) mehr
als diese harte Quote belegt wäre, greift Regel R2: Sie löscht Audio der
**ältesten fertigen, nicht geschützten Meetings zuerst**, bis etwa **80
Doppelstreckenstunden** (weiche Quote) erreicht sind oder keine geeigneten
Kandidaten mehr übrig sind (RQC §4.1 R2, §5.1). Ein Meeting wird pro Durchlauf
höchstens einmal gezählt; die Projektion rechnet bereits ausgewählte Bytes
genau einmal ab (RQC §5.1, Test t04 in Appendix B).

Wichtig und ehrlich gesagt: Unter hartem Speicherdruck darf R2 ein Meeting
löschen, dessen selbst gewähltes Zeitfenster (z. B. „30 Tage“) **noch nicht
voll abgelaufen ist** — aber niemals ein Meeting ohne beantwortete Auswahl,
mit „dauerhaft behalten“, mit Projektschutz oder mit Festplatten-Vorfall
(RQC §4.4, normative Klarstellung). Die Alternative wäre, die Platte still
volllaufen zu lassen, bis mitten in einer Aufnahme die Aufzeichnung scheitert
(DG-Notfallpfad, RQC §4.4).

## F3. Was bedeutet die Aufbewahrungsfrage — und was passiert, wenn ich sie nicht beantworte?

Nach dem ersten vollständig erfolgreich verarbeiteten Meeting fragt Speak
einmal nach einer Standardregel: `sofort löschen`, `7 Tage`, `30 Tage` oder
`dauerhaft behalten` (Plan §12 Zeilen 1447–1449; Auslöser ist Trigger T2 direkt
nach dem Abschluss der Verarbeitung, RQC §3 T2, §8 Regel 6).

Solange die Frage **unbeantwortet** bleibt, ist für dieses Meeting nichts
ausgewählt (`Unanswered`, in der Datenbank `NULL`) — und dann wird **nichts
automatisch gelöscht, weder durch R1 noch durch R2** (Plan §12 Zeile 1450
„ohne beantwortete Auswahl wird nichts automatisch gelöscht“; RQC §4.3 Punkt
1, §5.1 `deletion_eligible`, §8 Regel 6). Nicht-Antworten ist also immer der
sichere Zustand. Zusätzlich startet die Löschuhr eines Meetings überhaupt erst
mit der **bestätigt abgeschlossenen Verarbeitung** (`completed_at`), nie beim
Aufnahmestart — Audio überlebt Aufnahme, Transkription und Zusammenfassung
unabhängig von jeder Regel (AS-§8.1; RQC §4.1 R1, §5.1 `completed_at_unix_ms`).

## F4. Welche Schutzflags gibt es, und was bedeuten sie?

Es gibt zwei Arten von Schutz: dauerhafte Befreiungen und Laufzeit-Blocker.

**Dauerhafte Befreiungen** (ein Meeting mit einem davon kommt für R1 _und_ R2
nie infrage, RQC §4.3):

| Flag                                                 | Bedeutung                                                                                                                                                                                                             |
| ---------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Unanswered` (keine beantwortete Aufbewahrungsregel) | Es wurde nichts versprochen, also wird nichts angetastet (F3).                                                                                                                                                        |
| `KeepPermanent` („dauerhaft behalten“ gewählt)       | Ein gegebenes Versprechen. Quotadruck hebt Versprechen nicht auf (RQC §4.3 Punkt 1).                                                                                                                                  |
| `projects.protect_meeting_audio`                     | Projektschutz-Schalter: Solange gesetzt, ist das Audio jedes Meetings dieses Projekts nicht löschbar (RQC §4.3 Punkt 2). Reichere Formen (Projekt-Quoten, Projekt-Zeitfenster) sind bewusst abgelehnt (RQC §11 NG-2). |

**Laufzeit-Blocker** (zustandsabhängig; ein blockiertes Meeting wird
übersprungen und im nächsten Durchlauf erneut geprüft):

- laufende oder offene Session (Session offen bzw. Uhr nie gestartet);
- unbestätigte STT-Jobs (wartend/laufend/wartend-auf-Wiederholung);
- fehlgeschlagene STT-Jobs (Plan §12 Zeilen 1451–1452 wörtlich);
- noch nicht vollständig abgeschlossene Verarbeitung;
- `ended_disk_full` — das Meeting ist mit Festplatten-Vorfall geendet und
  wird unabhängig von jeder Regel gesperrt (RQC §4.3 letzter Absatz, §4.5;
  DG-§6.2).

Diese Prüfung läuft doppelt: einmal beim Entschluss (reine
Momentaufnahmen-Auswertung, RQC §7.2 Gate 1) und ein zweites Mal unmittelbar
vor der allerersten Dateilöschung zur Laufzeit durch den `RetentionGuard`
(AS-§8.2; RQC §7.2 Gate 2). Kann sich der Zustand zwischen beiden Prüfungen
verschlechtert haben (z. B. neuer fehlgeschlagener Job), wird das Meeting
übersprungen und protokolliert, aber nicht gelöscht (RQC §7.2, Race-Test in
§7.3).

## F5. Wenn nur Audio gelöscht wird — warum bleibt die Zusammenfassung zitierfähig?

Weil „Delete-Level 1“ genau definiert ist als **nur Dateien**: Alle
Audiodateien und Exportdateien des Meetings werden entfernt, aber das
Transkript, die Zusammenfassung, die Quellen und der Session-Datensatz bleiben
in der Datenbank vollständig erhalten; die Chunk-Zeilen wechseln nur in den
Zustand `deleted` (Prüfsummen bleiben), und der Suchindex bleibt unverändert
(AS-§9.1 Spalte 1; RQC §8 Regel 1). Das Meeting bleibt dadurch als
durchsuchbare Textquelle voll nutzbar — Zusammenfassungen, Aufgabenbelege und
Statusaussagen verlieren ihre Grundlage nicht.

Der einzige sichtbare Unterschied: Sprünge von Belegen zu Audiopositionen
degradieren zu Textsprüngen, solange kein Audio vorhanden ist — Belege werden
nie stumm oder still entfernt (AS-§9.1 Zeile `task_evidence`; Beleg-
Navigationsvertrag, Invariante E2 über die Retentionsgrenze hinweg, RQC §6.1
und Index-Eintrag evidence-navigation-contract).

Und wenn man sich später fragt „warum ist mein Audio weg?“: Zu jeder
automatischen Löschung wird eine `change_log`-Zeile mit Meeting-ID, Regel,
freigegebenen Bytes und Schutzentscheidung geschrieben (AS-§8.2 Regel 5;
Reihenfolge laut Vertrag: erst Dateilöschung, dann Protokollzeile, RQC §8
Regel 2); frisch geschriebene
Zeilen können vom 180-Tage-Aufräumen nicht sofort wieder verschluckt werden
(RQC §8 Regel 5, §9).

## F6. Werden importierte Dateien, WhatsApp-Medien oder andere Assets je automatisch gelöscht?

**Nein, niemals.** Importierte Originale gelten als unersetzliche
Nutzerdaten; die Asset-Bilanz (Ledger L2) wird nur gemessen und berichtet,
aber von keiner Regel bearbeitet (RQC §2 „Why L2 carries no rule“, §6.1,
§11 NG-6). Auch Quarantäne-Inhalte aus der Wiederherstellung (`recovery/`)
und verwaiste Teildateien werden laut Startscan ausdrücklich **nie**
automatisch gelöscht — nur mit expliziter Nutzerbestätigung (AS-§7; RQC §6.1,
§7.1 U-P3/U-P4).

## F7. Wann prüft Speak überhaupt, ob aufzuräumen ist?

Zu vier festen Anlässen: beim App-Start (erst nachdem der
Wiederherstellungs-Scan alle Meeting-Ordner klassifiziert hat — Reihenfolge
Scan → Prüfung → UI), direkt nach bestätigtem Verarbeitungsabschluss, direkt
nach einem Import (der dabei nur einen Bericht erzeugt, nichts löscht) und
zusätzlich alle 24 Stunden bei laufender App (RQC §3, Trigger T1–T4). Läuft
eine Prüfung, wird eine gleichzeitig eintreffende weitere in dieselbe Prüfung
eingereiht statt parallel ausgeführt (RQC §3).

## F8. Erfahre ich, wenn automatisch gelöscht wurde?

Ja — aber nur, wenn wirklich etwas passiert ist. Nach jedem Durchlauf mit
mindestens einer echten Löschung erscheint genau eine Benachrichtigung mit den
tatsächlichen Zahlen (entfernte Megabytes, Anzahl betroffener Meetings), je
nach Grund als „Aufbewahrungszeitraum abgelaufen“ (N1) oder
„Speicher wurde knapp“ (N2) (RQC §6.2 N1/N2 samt Ehrlichkeitsbedingung:
gerendert aus den realen Löschberichten, keine Anzeige bei null Aktionen).
Ist die Quote überschritten, konnte aber sicher nichts gelöscht werden (alles
geschützt, dauerhaft oder in Verarbeitung), gibt es stattdessen eine
entsprechende Warnung (N4). Für Nicht-Ereignisse — blockierte Meetings,
unsichtbares Datenbank-Aufräumen — gibt es dagegen bewusst **keine**
Benachrichtigung (RQC §6.2 N3/N5, §11 NG-8); sie sind nur über die
`change_log`-Protokollzeilen nachvollziehbar.

## F9. Kann ich die Regeln konfigurieren oder erweitern?

Nein, und das ist Absicht. Genau drei Regeln existieren in v1 (R1, R2, R3)
als Code, nicht als Konfiguration; es gibt keine regelbasierte
Konfigurationssprache, keine Projekt-Quoten, keine Prognosen oder
Kaltstorage-Tiers (RQC §4.1, §11 NG-1–NG-4). Der bewusste Umfang: vier
gespeicherte Richtlinienwerte, zwei Quotaschwellen, ein 180-Tage-Aufräumwert,
ein Projektschutzschalter. Alles darüber hinaus wurde explizit als
komplexitätstreibende Funktion ohne Beleg verworfen (RQC §11 Komplexitätsbudget).

## F10. Was gilt heute, wenn ich dieselbe Frage zum jetzigen Produkt habe?

Heute existiert nur der Diktiermodus, und der löscht nichts automatisch — es
gibt schlicht keine der hier beschriebenen Mechaniken. Für Fragen zum heutigen
Datenbestand (Ablage, Backup, Restore des Diktatverlaufs) gilt der Operator-
Guide [docs/meeting-mode-data-migration-operator-guide.md](../meeting-mode-data-migration-operator-guide.md);
für den Meeting-Modus ist diese FAQ ausschließlich eine Erklärung des
Vertragsstands „spec contract — no implementation yet“. Sobald die Phase-3-
Speicherarbeit landet, ist jede Antwort hier gegen den Code neu zu prüfen
(Indexregel §1: Statuswechsel nur mit Codebeleg).
