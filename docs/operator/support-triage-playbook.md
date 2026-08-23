# Support-Triage-Playbook — Beschwerdeklassen, Evidenzanforderungen und Eskalation

Status: **Playbook (lokaler Prozess)** — dieses Dokument beschreibt einen
internen Triage-Prozess und macht selbst keine Verhaltensaussagen über
implementierten Code. Jede Verhaltens-/Evidenzfrage wird an die unten
verlinkten Operator-Dokumente delegiert, die ihre Aussagen mit Codebeleg
tragen (Indexregel §1,
[docs/meeting-mode-documentation-index.md](../meeting-mode-documentation-index.md),
Abschnitt 1).
Autor-Arbeitsstand: **HEAD-Commit `d85a34b861398aee9f20160477181a50ec56ea3b`**
(verifiziert am 22.08.2026 via `git rev-parse HEAD`; `git status --porcelain`
war vor der Erstellung dieses Dokuments leer).
Umfang: Klassifikation eingehender Berichte in Triage-Lanes, je Lane die
exakt anzufordernde Diagnose-Evidenz, Eskalationskriterien sowie die
Aufbereitung der Lane-Signale als strukturierte Eingabe für den
Post-Boss-Discovery-Zyklus (Beschwerde-Mining mit
Evidenz/Frequenz/Nutzerwert/Komplexitäts-Matrix).
Plan: [docs/meeting-mode-plan.md](../meeting-mode-plan.md)
Index: [docs/meeting-mode-documentation-index.md](../meeting-mode-documentation-index.md)
Aufgenommen am: 22.08.2026

---

## 1. Grundsätze

1. **Lokales Artefakt only.** Dieses Playbook erzeugt und verarbeitet
   ausschließlich lokale Triage-Aufzeichnungen. Es enthält bewusst **keine**
   Schritte zum Posten, Publizieren oder Melden nach außen (GitHub Issues,
   Discussions, E-Mail, Foren). Externe Meldekanäle sind gemäß `AGENTS.md`
   (Abschnitt „GitHub workflow") menschlichen Contributorn vorbehalten und
   nicht Teil dieses Prozesses.
2. **Evidenz vor Hypothese.** Ein Bericht verlässt seine Lane nicht als
   „bestätigter Befund", bevor nicht die in Abschnitt 3 geforderte
   Mindest-Evidenz vorliegt. Fehlt sie, ist der einzige zulässige nächste
   Schritt die Evidenzanfrage (Abschnitt 3), keine Ursachenvermutung im
   Berichtstext.
3. **Kein Restatement.** Dieses Playbook wiederholt keine Symptom-Ursache-
   Tabellen der Troubleshooting-Dokumente; es verlinkt sie und benennt nur,
   *welche* Zeilen/Evidenzfelder aus ihnen anzufordern sind.
4. **Discovery-Feed.** Das Ziel jeder Triage ist ein strukturierter
   Lane-Datensatz (Abschnitt 5), der ohne Nachbearbeitung in die
   Evidenz/Frequenz/Nutzerwert/Komplexitäts-Matrix des Discovery-Zyklus
   übernommen werden kann.

## 2. Triage-Lanes

Jeder eingehende Bericht wird in genau eine Primär-Lane einsortiert. Bei
Mehr-Klassen-Berichten gilt die Reihenfolge unten (erste zutreffende Klasse
gewinnt); die Sekundärklasse wird im Datensatz vermerkt.

| Lane | Klasse | Typische Erkennungssignale | Primäre Referenz |
|---|---|---|---|
| **T1** | Aufnahme schlägt fehl / Capture-Failure | Kein Ton, leere Aufnahme, Gerät verschwindet, Berechtigungsfehler, Recording startet/stopped nicht | [troubleshooting-audio-devices.md](troubleshooting-audio-devices.md) |
| **T2** | Ausgabe falsch | Falscher/fehlender Text, falsche Sprache, Halluzinationen, Formatierungsfehler, Text landet nicht im Zielfeld | [troubleshooting-models.md](troubleshooting-models.md) (Modell-/Sprachwahl) + [troubleshooting-audio-devices.md](troubleshooting-audio-devices.md) (Resampler-/Rate-Fälle R12–R15) |
| **T3** | Performance | Hohe Latenz, CPU/GPU-Last, Stottern während Aufnahme, langsamer Modellstart | [troubleshooting-models.md](troubleshooting-models.md) (Modellgröße/-Variante) + [meeting-mode-known-limitations.md](../meeting-mode-known-limitations.md) |
| **T4** | Installation / Modell-Setup | Download bricht ab, Prüfsummenfehler, Offline-Setup, VAD-Modell fehlt, portable Installation | [troubleshooting-models.md](troubleshooting-models.md) |
| **T5** | Shortcut / CLI / Fernsteuerung | Hotkey registriert nicht, Doppelbelegung, PTT-Verhalten, `--toggle-*`-Flags wirkungslos, Single-Instance | [troubleshooting-shortcuts-cli.md](troubleshooting-shortcuts-cli.md) |

**Lane T0 (Vorsortierung, keine End-Lane):** Bericht ohne erkennbares Signal
oder mit mehreren gleichwertigen Klassen. T0-Berichte erhalten die
Basis-Evidenz aus Abschnitt 3.7 und werden nach deren Eingang in T1–T5
umsortiert. T0 zählt **nicht** als Discovery-Signal.

## 3. Evidenzanforderungen je Lane

Für jede Lane gilt: erst die **Basis-Evidenz** (§3.7), dann die
**lane-spezifische Evidenz**. Die Feldbezeichnungen folgen dabei den Spalten
„Zu sammelnde Evidenz" der referenzierten Dokumente.

### 3.1 T1 — Capture-Failure

Anfordern (gemäß Evidenzspalten von
[troubleshooting-audio-devices.md](troubleshooting-audio-devices.md)):

1. Betroffene Symptomzeile R1–R15 (Nutzer beschreiben lassen bzw. zuordnen):
   welches Symptom, welche Quelle (Mikrofon/Systemaudio).
2. Debug-Log: Anwendung mit `--debug` gestartet, Logausschnitt vom
   Fehlerzeitpunkt (Start der Aufnahme bis Fehlermeldung).
3. Gerätekontext: Audio-Gerätename, Default-Gerät zum Zeitpunkt des Fehlers,
   ob das Gerät während der Aufnahme gewechselt/an-/abgesteckt wurde
   (Hot-Unplug-Fälle).
4. Plattform + Berechtigungsstatus (Mikrofonzugriff des Systems).

### 3.2 T2 — Falsche Ausgabe

Anfordern:

1. Gewünschter vs. tatsächlicher Output (wörtliches Zitat beider; bei
   „Text landet nicht im Zielfeld": Ziel-Anwendung und Eingabemethode).
2. Gewähltes Modell und Sprache (Modell-ID wie im Modellselektor angezeigt;
   Ablageorte siehe
   [troubleshooting-models.md](troubleshooting-models.md), Abschnitt 2).
3. Debug-Log des Transkriptionslaufs.
4. Bei Verdacht auf Audio-Seite (abgeschnittene Wörter, Rauschen): zusätzlich
   die vollständige T1-Evidenz (Resampler-/Rate-Fälle R12–R15).

### 3.3 T3 — Performance

Anfordern:

1. Modellvariante und -größe, Hardwareklasse (GPU vorhanden? Welche?), OS.
2. Beobachtete Kennzahl (Zeit bis Text, CPU-%/RAM, geführt vs. gemessen) und
   Sitzungslänge.
3. Debug-Log über mindestens einen reproduzierbaren Langsam-Fall.
4. Abgleich gegen die registrierten Ist-Grenzen in
   [meeting-mode-known-limitations.md](../meeting-mode-known-limitations.md)
   — bereits registrierte Grenzen sind **keine** neuen Discovery-Signale,
   sondern werden nur gezählt.

### 3.4 T4 — Installation / Modell-Setup

Anfordern (gemäß
[troubleshooting-models.md](troubleshooting-models.md)):

1. Installationsart (Installer/portabel) und App-Version.
2. Download-Symptomzeile (HF-Retry/Mirror, Stall-Watchdog,
   SHA256-/Größenfehler, Archivfehler) inklusive des Events
   `model-download-failed` aus dem Log (nicht nur der transiente Toast).
3. Speicherplatz-Bedarf vs. freier Platz am Models-Verzeichnis
   (`<App-Datenverzeichnis>/models`; Ablageort-Tabelle Abschnitt 2 dort).
4. Bei Offline-Setup: exakte Schritte des Drop-ins inkl.
   `silero_vad_v4.onnx`-Beschaffung; bei ONNX-Runtime-Fehlern: gesetzte
   Umgebungsvariablen (`ORT_LIB_LOCATION`, `ORT_PREFER_DYNAMIC_LINK`).

### 3.5 T5 — Shortcut / CLI

Anfordern (gemäß
[troubleshooting-shortcuts-cli.md](troubleshooting-shortcuts-cli.md)):

1. Exakter Shortcut-/Flag-Aufruf (Binding, PTT vs. Toggle) und erwartetes vs.
   beobachtetes Verhalten; Symptomzeile R1–R20 zuordnen.
2. Debug-Log von App-Start bis Reproduktion (Registrierungsfehler stehen am
   Start).
3. Bei Fernsteuerungs-Flags: vollständige Kommandozeile, App war vor Aufruf
   bereits gestaltet (Kaltstart-No-op ist dokumentiertes Verhalten, kein Bug).
4. macOS: Accessibility-Berechtigungsstatus; Linux: Fensterumgebung
   (X11/Wayland).

### 3.6 Geplante Begleitdokumente (offengelegte Abhängigkeit)

Der Task-Umfang sieht zwei Begleitdokumente vor, die zum Autor-Zeitpunkt in
diesem Arbeitbaum **nicht existieren** (verifiziert per glob/`Get-ChildItem`
am 22.08.2026; dieselbe Lage ist im Diagnostics-Vertrag als Abhängigkeit
G3-M3 offengelegt):

- `docs/operator/diagnostics-capture-guide.md` — sobald vorhanden, ersetzt
  er die Log-Sammelhinweise in §3.1–§3.5 als primäre Anforderungsreferenz;
  die Lanes verweisen dann auf seine Erfassungsbefehle statt auf die
  Kurzform „`--debug` starten".
- Support-Bundle-Dokument — sobald vorhanden, definiert es das
  zusammengefasste Evidenzpaket; bis dahin gilt: die je Lane angeforderten
  Elemente werden lokal in einem Ordner pro Bericht abgelegt (Logausschnitt,
  Gerätekontext, Repro-Schritte), ohne dass etwas übertragen wird.

Beide Verweise sind Zusagen über Pfade, keine Inhaltsbehauptungen; die
inhaltliche Autorität bleibt bei den verlinkten Ist-Stand-Dokumenten und dem
[Diagnostics-Vertrag](../meeting-mode-architecture/diagnostics-contract.md)
(Bundle-Artefakte B1–B7, Mapping-Regeln G3-M1/M2 für künftige
Operations-Guides).

### 3.7 Basis-Evidenz (jede Lane)

1. App-Version und Installationsart, OS + Version.
2. Reproduktionsschritte (minimal, nummeriert) und Häufigkeit (immer/
   manchmal/einmalig).
3. Debug-Log-Ausschnitt (`--debug`, siehe `AGENTS.md`,
   „CLI Parameters"; Definition in `src-tauri/src/cli.rs`).

## 4. Eskalationskriterien

Eskalation bedeutet: Übergabe an die Entwicklungs-Dokumentation/den
Limitations-Register-Prozess — **kein** externer Post. Eine Lane wird
eskaliert, wenn mindestens ein Kriterium erfüllt ist:

| # | Kriterium | Eskalationsziel |
|---|---|---|
| E1 | Mindest-Evidenz (§3) liegt vollständig vor und widerlegt alle dokumentierten Ursachen der Lane-Matrix | Neuer Kandidat für [meeting-mode-known-limitations.md](../meeting-mode-known-limitations.md) (nur mit Beleg, Registerregel dort) |
| E2 | Dasselbe Symptomsignal tritt in ≥3 unabhängigen Lane-Datensätzen auf | Discovery-Matrix (Frequenz-Spalte), Abschnitt 5 |
| E3 | Datenverlust oder Wiederherstellungsversuch betroffen | [Backup/Restore-Drill](backup-restore-drill.md) zur Verifikation des Restore-Pfads + E1 |
| E4 | Verhalten widerspricht einer belegten Aussage eines Operator-Troubleshooting-Dokuments (Dokument-Drift) | Eigentümer des jeweiligen Troubleshooting-Dokuments (Drift-Fix, keine Feature-Deutung) |
| E5 | Bericht betrifft Meeting-Mode-Zielverhalten aus dem Plan | Discovery-Zyklus als Nutzerbedarfs-Signal; **keine** Umsetzungsannahme (Feature-Freeze, Plan §1/§2 Nicht-Ziele) |

**Explizit keine Eskalation:** Einzelvorkommen dokumentierten Verhaltens
(z. B. Kaltstart-No-op der Fernsteuerungsflags), fehlende Evidenz (→
Nachanfordern statt eskalieren), Wunschkriterien ohne reproduzierbares
Signal (→ T0, zählt nicht).

## 5. Discovery-Feed: strukturierter Lane-Datensatz

Je abgeschlossenem Bericht wird lokal ein Datensatz mit genau diesen Feldern
gepflegt (Freitext nur in „notes"):

```
lane:            T1..T5
symptom_ref:     <R-Zeilen-Nr. aus dem Lane-Referenzdokument>
evidence_state:  pending | complete | refuted-documented | confirmed-new
frequency_count: <Zählung unabhängiger Berichte je symptom_ref>
severity:        blocker | major | minor
user_value_note: <ein Satz: welcher Arbeitsfluss ist betroffen>
complexity_hint: <ein Satz, falls E1: wo der Verdacht sitzt (Pfad/Symbol)>
escalated_to:    <E1..E5 oder leer>
```

Diese Felder sind die direkte Eingabe für die
**Evidenz/Frequenz/Nutzerwert/Komplexitäts-Matrix** des
Post-Boss-Discovery-Zyklus: `confirmed-new`-Datensätze (E1) liefern die
Evidenz-Spalte, `frequency_count` die Frequenz-Spalte, `user_value_note` die
Nutzerwert-Spalte, `complexity_hint` die Komplexitäts-Spalte. Nur Datensätze
mit vollständigem `evidence_state ∈ {confirmed-new}` sind stimmberechtigt
für die Implementierungs-Auswahl; alles andere bleibt Signal.

## 6. Komplexitätsbudget / Non-Goals

- Kein Ticket-System, keine Datenbank, kein Tooling: der Datensatz (§5) ist
  ein Textformat, gepflegt neben diesem Playbook.
- Kein externer Melde-/Upload-Kanal (siehe §1.1); auch künftige
  Diagnostics-Bundles bleiben lokale Artefakte.
- Keine Ursachenanalyse in diesem Dokument: jede Ursache lebt ausschließlich
  in den belegten Troubleshooting-Dokumenten (§1.3).
- Kein SLA, keine Priorisierungsformel hier — die Gewichtung geschieht nur
  in der Discovery-Matrix gegen die vier genannten Dimensionen; Low-Value-
  Features werden dort explizit abgelehnt (Non-Goal: SAP-artiger Scope).
