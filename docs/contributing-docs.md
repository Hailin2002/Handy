# Contributing Documentation — Meeting-Mode Docs

- **Status:** Konventionsdokument (dieses Dokument regelt, wie
  meeting-mode-Dokumentation geschrieben und gepflegt wird; es macht selbst
  keine Verhaltensaussage über Produktcode und definiert nichts am Produkt).
- **Autor-Arbeitsstand:** HEAD-Commit `33c6e0fd7effef94daf13e6277ee4ca7e6d00e96`
  („company(task-0295): ActiveMeetingNavItem live-tick and pin-appearance
  behavior test"; verifiziert am 23.08.2026 via `git rev-parse HEAD` in einem
  sauberen Arbeitbaum; `git status --porcelain` war bei der
  Verifikation leer).
- **Geltungsbereich:** alle meeting-mode-Dokumente unter `docs/`, die in der
  Dokumenttabelle des
  [Dokumentationsindex](meeting-mode-documentation-index.md) (Abschnitt 3)
  geführt werden — und jede Person oder jeder Agent, die/der ein solches
  Dokument anlegt oder ändert.
- **Nicht Geltungsbereich:** der allgemeine Contribution-Flow (Environment
  aufsetzen, forken, Branches, Commits, PRs, Issue/Discussion-Regeln,
  Verhaltenskodex, Lizenz). Dafür gilt ausschließlich
  [CONTRIBUTING.md](../CONTRIBUTING.md) — dieses Dokument dupliziert ihn
  bewusst nicht. Für Repo-weite Assistenten-Regeln gilt
  [AGENTS.md](../AGENTS.md).

---

## 1. Die eine Grundregel: Implementiertes Verhalten nur mit Codebeleg

Der Dokumentationsindex normiert dies in seinem Abschnitt 1. Die Regel gilt
wörtlich; hier steht sie unverändert zitiert:

> **Nur Dokumente, die ihr Aussagen mit konkreten Codebelegen versehen
> (`datei.rs`/`datei.tsx` mit Fundstelle oder Commit-Hash), dürfen beschreiben,
> was implementiert ist.** Alle übrigen meeting-mode-Dokumente beschreiben
> Planung, Vorschläge oder Verträge — niemals lauffähiges Verhalten. Ein
> Statuswert „implemented behavior" ohne Codebeleg in der Tabelle unten ist
> ein Dokumentationsfehler und zu korrigieren.

Konsequenzen für Autoren:

- Jede Aussage der Form „die App tut X" / „der Code macht Y" braucht eine
  Fundstelle (`datei.rs:123-130`) oder einen Commit-Hash — und die Fundstelle
  muss zum Zeitpunkt des Schreibens gegen den gepinnten HEAD geprüft worden
  sein (siehe Abschnitt 4).
- Dokumente, die nur Planung, Verträge oder andere Dokumente beschreiben,
  behaupten keinen Ist-Zustand des Codes. Wo ein Ist-Stand erwähnt wird
  (z. B. „der Meeting-Modus ist noch nicht implementiert"), wird auch das
  mit einem Befehls-/Dateibeleg getragen, nicht nur behauptet.
- Ein Authority Statement in der Indextabelle fasst zusammen, wofür das
  Dokument autoritativ ist — und bei `implemented behavior` zwingend den
  Codebeleg.

## 2. Status-Enum: nur die vier verbindlichen Werte

Der Index normiert in Abschnitt 2 genau vier Statuswerte. Sie werden
unverändert zitiert; neue Werte werden nicht erfunden:

| Wert                   | Bedeutung                                                                                                                                                                      |
| ---------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `plan`                 | Planungsdokument; legt Ziele, Umfang, Phasen und Abnahme fest. Keine Verhaltensaussage über bestehenden Code außer mit explizitem Codebeleg.                                   |
| `proposal-only`        | unverbindlicher Vorschlag (z. B. Textentwürfe); nichts davon ist implementiert oder freigegeben.                                                                               |
| `spec contract`        | normativer Vertrag für zukünftige Implementierung (Interface, Datenformat, Zustände); ändert keinen Quellcode. Gültig erst mit Implementierung, dann gegen den Code zu prüfen. |
| `implemented behavior` | beschreibt tatsächlich vorhandenes, im Code belegbares Verhalten. Nur zulässig mit Code-/Commit-Beleg (siehe Regel in Abschnitt 1).                                            |

Praxis (wie in der bestehenden Tabelle):

- Einschränkungen des Status stehen in Klammern hinter dem Wert, z. B.
  `implemented behavior` (nur für den Diktierpfad) — ein Dokument trägt
  „implemented behavior" nie für einen größeren Umfang, als seine Belege
  tragen.
- Rein erklärende Begleitdokumente zu einem `spec contract` tragen selbst
  `spec contract` mit dem Zusatz, dass sie keine Ist-Verhaltensaussagen
  machen (Muster: `docs/operator/error-code-crosswalk.md`,
  `docs/operator/retention-faq.md`).

## 3. Pflicht: Registrierung in der Indextabelle

Der Index normiert in Abschnitt 5 (Pflegeregeln), hier unverändert zitiert:

> - Neues meeting-mode-Dokument → sofortige Ergänzung der Tabelle in Abschnitt 3
>   (Pfad, Plan-Abdeckung, Statuswert aus Abschnitt 2, einzeiliges Authority
>   Statement).
> - Statuswechsel (z. B. `spec contract` → `implemented behavior`) erfordert einen
>   Codebeleg im Authority Statement gemäß Regel in Abschnitt 1.

Das bedeutet konkret:

1. Jede neue `.md`-Datei unter `docs/` bekommt **im selben Changeset** genau
   eine Tabellenzeile im Index (Abschnitt 3): Pfad, abgedeckte
   Plan-Abschnitte, Statuswert aus Abschnitt 2, einzeiliges Authority
   Statement.
2. Zusätzlich gehört ein datierter Update-Absatz in den Prosateil über der
   Tabelle (Muster der bestehenden „Update (Task …)"-Absätze), der nennt:
   was neu aufgenommen wurde, mit welchem Status, den Belegstand-HEAD und
   die neu gezählten Bestandszahlen (Markdown-Dateien unter `docs/`,
   Tabellenzeilen, Ausnahmen).
3. Die Bestandszahlen werden nicht geschätzt, sondern gezählt, z. B.:
   `Get-ChildItem docs -Recurse -Filter *.md` bzw.
   `Select-String '^\| \`docs/'` auf dem Index.
4. Fehlende Zeilen fremder Dokumente (bekannt:
   `docs/operator/backup-restore-drill.md`,
   `docs/operator/meeting-mode-readiness-checklist.md`)
   trägt nicht jeder beliebige Task nach, sondern der Eigentümer des
   jeweiligen Dokuments — bestehende Tasks haben solche Nachträge bewusst
   offen gelassen und dies im Update-Absatz vermerkt.
5. Die Indexregistrierung ist eine offengelegte, minimal notwendige
   Erweiterung des eigenen Task-Konfliktumfangs, wenn der deklarierte Scope
   nur das eigene Dokument plus Read-Scopes umfasst — sie wird im
   Update-Absatz ausdrücklich als solche benannt (Muster: task-0368,
   task-0370, task-0371).

## 4. HEAD-Pinning-Praxis

Jedes Dokument, das Belege trägt (Codezitaten, Messstände, Audit-Stände),
pinnt den HEAD, gegen den die Belege verifiziert wurden:

1. Vor dem Schreiben: `git rev-parse HEAD` ausführen und den vollen Hash
   plus Commit-Subject notieren.
2. In einem sauberen Arbeitbaum verifizieren: `git status --porcelain` muss
   leer sein. Der Index trägt die Praxis, die Verifikation zweifach
   auszuführen; mindestens einmal mit leerem Porcelain-Status ist Pflicht.
3. Den Stand im Dokumentkopf nennen (Muster: „Autor-Arbeitsstand:
   HEAD-Commit `<hash>` („<subject>"; verifiziert am <Datum> via
   `git rev-parse HEAD` …)") und im Index-Update-Absatz den Kurz-Hash als
   Belegstand.
4. Zitierte Datei-/Zeilenbelege gelten relativ zu diesem gepinnten HEAD.
   Verschiebt sich der Code später, ist der Beleg nicht „falsch", aber das
   Dokument ist re-verifikationsbedürftig — der gepinnte Hash macht das
   prüfbar.
5. Nie einen HEAD-Pin behaupten, der nicht tatsächlich ausgeführt wurde
   („never claim a verification you did not run").

## 5. Flip-Regel des Known-Limitations-Registers

Das Register `docs/meeting-mode-known-limitations.md` normiert selbst, wann
ein Eintrag seinen Status wechseln darf. Unverändert zitiert aus seiner
Status-Legende:

> **Status legend:** `open` — the limitation/claim stands as written;
> `resolved` — a cited artifact or commit in the tree proves the limitation
> no longer applies. **No entry below may be flipped to `resolved` without
> a citation to the resolving artifact or commit.**

Konsequenzen:

- Ein Flip auf `resolved` ohne Zitat des auflösenden Artefakts/Commits ist
  ein Dokumentationsfehler.
- Der Flip wird mit Re-Verifikation dokumentiert und datiert (Muster:
  Re-Audit 2026-08-23 im Register, das jeden Eintrag gegen den jeweiligen
  Audit-HEAD `b4b0d344` re-prüfte und ausdrücklich vermerkte, dass kein
  Flip erfolgte).
- „Es sieht so aus, als sei das inzwischen behoben" ist kein Flip-Grund;
  nur ein benanntes, zitiertes Artefakt/Commit ist einer.

## 6. Autoritätsordnung bei Konflikten

Der Index (Abschnitt 4, Schluss) fixiert die Ordnung, hier sinngemäß
zitiert: **Plan schlägt Verträge, Verträge schlagen Kataloge, und nichts
schlägt den tatsächlichen Code** (der seinerseits nur durch code-zitierte
Dokumente wie die Phase-0-Baseline abgedeckt wird). Dokument-Autoren:

- Restatements nie widersprüchlich zum Vorranghöheren; wörtliche
  Restates markieren die Quelle (Muster: session-settings-policy restatet
  die Snapshot-Isolation „wörtlich … statt geändert").
- Abweichungen vom Code-Wirklichkeitsbild (z. B. der AGENTS.md-CLI-Tabelle
  gegenüber `lib.rs`) werden dokumentiert, nicht still verschwiegen
  (Muster: `docs/operator/troubleshooting-shortcuts-cli.md`).
- Evidenz-Artefakte (CSV-Rohdaten, Skripte) sind keine Dokumente und
  bekommen keine Tabellenzeile (Muster: Index Abschnitt 3,
  „Verwandte, aber keine .md-Dokumente").

## 7. Selbst-Check für Dokument-Autoren

Vor dem Abschluss eines Dokument-Changesets alle Punkte abhaken:

- [ ] Die neue/geänderte Datei liegt unter `docs/` und ist meeting-mode-Bezug
      im Sinne des Index (sonst gilt CONTRIBUTING.md, nicht dieses Dokument).
- [ ] Neue Datei: genau eine Tabellenzeile im Index-Abschnitt 3 ergänzt
      (Pfad, Plan-Abdeckung, Statuswert, Authority Statement) plus datierter
      Update-Absatz mit gezählten Bestandszahlen und offengelegter
      Scope-Erweiterung.
- [ ] Der Statuswert ist einer der vier Enum-Werte aus Abschnitt 2 —
      unverändert zitiert, kein neuer Wert, keine neue Bedeutung.
- [ ] Trägt das Dokument `implemented behavior` (auch eingeschränkt), hat
      **jede** Verhaltensaussage eine Fundstelle (`datei.rs:Zeilen`) oder
      einen Commit-Hash, und jede Fundstelle wurde gegen den gepinnten HEAD
      geprüft.
- [ ] HEAD gepinnt: `git rev-parse HEAD` ausgeführt und notiert;
      `git status --porcelain` war bei der Verifikation leer.
- [ ] Kein Known-Limitations-Eintrag wurde auf `resolved` geflippt ohne
      Zitat des auflösenden Artefakts/Commits.
- [ ] Der allgemeine Contribution-Flow wird auf CONTRIBUTING.md verlinkt,
      nicht dupliziert; AGENTS.md-Regeln (u. a. konventionelle
      Commit-Präfixe, hier: `docs:`) werden eingehalten.
- [ ] Kein Widerspruch zur Autoritätsordnung (Plan > Verträge > Kataloge;
      nichts schlägt den Code); wörtliche Restates nennen ihre Quelle.
- [ ] Evidenz-Artefakte (CSVs, Skripte, Logs) wurden nicht als Dokumente in
      die Tabelle aufgenommen.

## 8. Nicht-Ziele dieses Dokuments

- Keine neuen Regeln über den Index, das Limitations-Register und
  CONTRIBUTING.md hinaus — dieses Dokument kodifiziert bestehende,
  normative Praxis, es erfindet keine.
- Keine Änderung am allgemeinen Contribution-Flow, an Issue-/PR-Templates
  oder an der Community-Philosophie (alles in CONTRIBUTING.md und
  `.github/`).
- Keine inhaltlichen Aussagen über den Meeting-Modus oder den Diktierpfad —
  dafür sind die im Index verzeichneten Fachdokumente autoritativ.
