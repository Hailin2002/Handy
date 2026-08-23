/* Speak Meeting-Mode Prototyp – Logik (statisch, kein Build-Schritt).
   Implementiert die Informationsarchitektur aus Plan §6.3 als klickbaren
   Mock: Hash-Routing, angehefteter Aufnahme-Eintrag, Startdialog,
   Laufendansicht, Verarbeitung-Checkliste. Alle Daten sind Mock-Daten. */
"use strict";

(function () {
  var $ = function (sel) { return document.querySelector(sel); };
  var liveRegion = $("#live-region");

  function announce(msg) { liveRegion.textContent = msg; }

  /* ---------- Zustand ---------- */
  var meetingSeq = 0;
  function nextMeetingId() { meetingSeq += 1; return "m" + meetingSeq; }
  var state = {
    sessionActive: false,      // läuft gerade ein Meeting? (steuert Pin in der Navigation)
    paused: false,
    seconds: 0,                // gemockte Aufnahmedauer, pro Tick inkrementiert
    meetings: [
      { id: nextMeetingId(),
        title: "Wochenplanung", date: "Gestern, 16:00", duration: "00:58:12",
        provider: "Lokal", status: "fertig",
        project: null,
        summary: "Team einigte sich auf Release-Termin und zwei Follow-up-Aufgaben.",
        decisions: ["Release-Termin wird um eine Woche verschoben.",
                    "Messestand-Budget bleibt unverändert."],
        tasks: [{ text: "Zahlen bis Donnerstag prüfen", done: false },
                { text: "Protokoll an das Team senden", done: true }],
        questions: ["Ist das Budget für die Support-Werkstudenten bewilligt?"],
        segments: [
          ["0:02", "Wir starten mit dem Tagesordnungspunkt Budget."],
          ["0:04", "Ich schlage vor, das Marketingbudget um zehn Prozent zu erhöhen."],
          ["0:06", "Einwand: Der Messestand braucht dieses Jahr mehr Mittel."],
          ["0:08", "Entscheidung: Wir verschieben die Entscheidung um eine Woche."],
          ["0:10", "Nächster Punkt: Personalplanung für das vierte Quartal."],
          ["0:12", "Vorschlag: Zwei zusätzliche Werkstudenten für den Support."],
          ["0:14", "Einwand: Das Budget dafür ist noch nicht bewilligt."],
          ["0:16", "Wir prüfen die Zahlen bis Donnerstag und melden uns."],
          ["0:18", "Abschließend: Das Protokoll geht heute Abend an alle."]
        ],
        audio: true,
        processing: { provider: "Lokal", model: "Whisper Small",
                      chunksTotal: 24, transcribed: 24, failedChunks: 0, retries: 0 } },
      { id: nextMeetingId(),
        title: "Kundengespräch Berger", date: "Mo, 10:30", duration: "00:31:40",
        provider: "Lokal", status: "verarbeitung",
        project: null,
        summary: null,
        decisions: [], tasks: [], questions: [],
        segments: [["1:04", "Herr Berger bittet um ein Angebot für zwanzig Lizenzen."]],
        audio: true,
        processing: { provider: "Lokal", model: "Whisper Small",
                      chunksTotal: 11, transcribed: 4, failedChunks: 0, retries: 0 } }
    ],
    filter: "alle",
    search: "",
    activeTitle: "",
    selectedId: null,          // Meeting in der Detailansicht (§6.13)
    lastProcessingId: null     // Meeting, dessen Checkliste gerade läuft (§6.12)
  };

  function meetingById(id) {
    return state.meetings.filter(function (m) { return m.id === id; })[0] || null;
  }

  function fmt(sec) {
    var h = String(Math.floor(sec / 3600)).padStart(2, "0");
    var m = String(Math.floor((sec % 3600) / 60)).padStart(2, "0");
    var s = String(sec % 60).padStart(2, "0");
    return h + ":" + m + ":" + s;
  }
  function elapsedSec() {
    return state.seconds;
  }

  /* ---------- Routing ---------- */
  var ROUTES = ["diktieren", "meetings", "meeting-aktiv", "verarbeitung",
                "projekte", "eingang", "projekt-detail", "import",
                "diktatverlauf", "einstellungen", "hilfe",
                "meeting-einrichtung", "meeting-detail"];

  function currentRoute() {
    var h = location.hash.replace(/^#\/?/, "").split("/")[0];
    return ROUTES.indexOf(h) !== -1 ? h : "diktieren";
  }

  function render() {
    var route = currentRoute();
    document.querySelectorAll(".view").forEach(function (v) {
      v.hidden = v.id !== "view-" + route;
    });
    document.querySelectorAll(".nav-item").forEach(function (a) {
      if (a.dataset.route === route) a.setAttribute("aria-current", "page");
      else a.removeAttribute("aria-current");
    });
    // Laufendes Meeting aktiviert den Pin und hält ihn sichtbar (Gate G2)
    $("#pinned-recording").hidden = !state.sessionActive;
    $("#active-meeting-card").hidden = !state.sessionActive;
    if (route === "meeting-aktiv" && !state.sessionActive) {
      location.hash = "#/meetings";
      return;
    }
    if (route === "meetings") renderMeetings();
    if (route === "meeting-einrichtung") initSetupView();
    if (route === "meeting-detail") renderDetail();
    if (route === "meeting-aktiv") renderRecordingStatus();   // Chip/Pin synchron halten
    if (route === "projekte") renderProjects();
    if (route === "eingang") renderEingang();
    if (route === "projekt-detail") renderProjectDetail();
    if (route === "import") initImportWizard();
    updateInboxBadge();
    updateClockDisplays();
  }

  window.addEventListener("hashchange", render);

  /* ---------- Meetings-Ansicht ---------- */
  function meetingMatches(m) {
    if (state.filter !== "alle" && m.status !== state.filter) return false;
    if (!state.search) return true;
    // summary kann während der Verarbeitung noch null sein – nicht als
    // String "null" in die Suche mischen
    return (m.title + " " + (m.summary || "")).toLowerCase()
      .indexOf(state.search.toLowerCase()) !== -1;
  }

  /* Fortschrittstext aus deterministischen Verarbeitungszahlen (§6.5/§6.12) */
  function meetingProgressText(m) {
    if (!m.processing) return "";
    var p = m.processing;
    if (m.status === "fertig") return "Verarbeitung abgeschlossen · " +
      p.transcribed + " von " + p.chunksTotal + " Abschnitten transkribiert";
    if (p.transcribed < p.chunksTotal)
      return "Transkription läuft · " + p.transcribed + " von " + p.chunksTotal + " Abschnitten";
    return "Zusammenfassung wird erstellt …";
  }

  /* Gemeinsame Löschbestätigung: nennt konkret Audio, Transkript und
     Zusammenfassung (§6.14, Gate G2) – im Listen-Menü wie im Detail-Menü */
  function confirmDeleteMeeting(m) {
    return window.confirm("„" + m.title + "“ endgültig löschen?\n\n" +
      "Es werden Audio, Transkript und Zusammenfassung dieses Meetings unwiderruflich entfernt.");
  }

  function renderMeetings() {
    var list = $("#meeting-list");
    list.innerHTML = "";
    var shown = state.meetings.filter(meetingMatches);
    $("#meetings-empty").hidden = state.sessionActive || state.meetings.length > 0;

    shown.forEach(function (m) {
      var li = document.createElement("li");
      li.className = "meeting-item";
      var chip;
      if (m.status === "fertig")
        chip = '<span class="status-chip status-ready">✓ Fertig</span>';
      else if (m.status === "fehler")
        chip = '<span class="status-chip status-recording">✗ Fehler</span>';
      else
        chip = '<span class="status-chip status-processing">⟳ In Verarbeitung</span>';
      var summaryText = m.summary !== null && m.summary !== undefined ? m.summary
        : "Zusammenfassung wird nach Abschluss der Verarbeitung angezeigt.";
      li.innerHTML =
        "<div class='row-between'>" +
        // Karte öffnet die Detailansicht (§6.13); eigenes, mit Tastatur
        // erreichbares Element – das Drei-Punkte-Menü bleibt separat (Gate G4)
        "<button type='button' class='card-open' data-id='" + m.id + "'" +
        " aria-label='Meeting „" + esc(m.title) + "“ öffnen'>" +
        "<strong>" + esc(m.title) + "</strong> · <span class='muted'>" + esc(m.date) +
        "</span> · " + esc(m.duration) + " · " + esc(m.provider) + "</button>" +
        "<div class='menu-row'><button type='button' class='btn btn-ghost btn-small menu-btn' " +
        "aria-haspopup='true' aria-expanded='false' aria-label='Aktionen für " + esc(m.title) + "'>⋯ Menü</button></div></div>" +
        chip + "<span class='muted small'>" + esc(meetingProgressText(m)) + "</span>" +
        "<p class='muted small'>" + esc(summaryText) + "</p>";
      li.querySelector(".card-open").addEventListener("click", function () {
        state.selectedId = m.id;
        location.hash = "#/meeting-detail";
      });
      // Sichtbares Drei-Punkte-Menü (§6.5): Export/Neu verarbeiten/Löschen ohne Hover
      li.querySelector(".menu-btn").addEventListener("click", function (ev) {
        toggleMenu(ev.currentTarget, m);
      });
      list.appendChild(li);
    });

    if (!shown.length && state.meetings.length) {
      list.innerHTML = "<li class='muted'>Keine Meetings für Suche/Filter.</li>";
    }
  }

  function toggleMenu(btn, m) {
    var existing = btn.parentElement.querySelector(".menu");
    closeMenus();
    if (existing) return;
    var menu = document.createElement("div");
    menu.className = "menu";
    menu.setAttribute("role", "menu");
    [["Exportieren …", ""], ["Erneut verarbeiten", ""], ["Löschen …", "menu-danger"]].forEach(function (item) {
      var b = document.createElement("button");
      b.type = "button";
      b.setAttribute("role", "menuitem");
      b.textContent = item[0];
      if (item[1]) b.className = item[1];
      if (item[0].indexOf("Löschen") === 0) {
        b.addEventListener("click", function () {
          // Irreversible Löschung nennt konkret, was entfernt wird (§6.14)
          if (confirmDeleteMeeting(m)) {
            state.meetings = state.meetings.filter(function (x) { return x !== m; });
            announce("Meeting gelöscht.");
          }
          closeMenus(); renderMeetings();
        });
      } else {
        b.addEventListener("click", function () { closeMenus(); announce(item[0] + " – im Prototyp nicht implementiert."); });
      }
      menu.appendChild(b);
    });
    btn.setAttribute("aria-expanded", "true");
    btn.parentElement.appendChild(menu);
  }
  function closeMenus() {
    // Nur dynamische Karten-Menüs entfernen; das statische Detail-Menü
    // (.menu-static) bleibt im DOM und wird lediglich zugeklappt
    document.querySelectorAll(".menu:not(.menu-static)").forEach(function (el) { el.remove(); });
    var more = $("#detail-more-menu");
    if (more) more.open = false;
    document.querySelectorAll(".menu-btn[aria-expanded]").forEach(function (b) {
      b.setAttribute("aria-expanded", "false");
    });
  }
  document.addEventListener("click", function (ev) {
    if (!ev.target.closest(".menu-row")) closeMenus();
  });

  /* Filter-Chips + Suche */
  document.querySelectorAll(".filter-group .chip").forEach(function (chip) {
    chip.addEventListener("click", function () {
      state.filter = chip.dataset.filter;
      document.querySelectorAll(".filter-group .chip").forEach(function (c) {
        c.classList.toggle("chip-selected", c === chip);
      });
      renderMeetings();
    });
  });
  $("#meeting-search").addEventListener("input", function (e) {
    state.search = e.target.value; renderMeetings();
  });

  /* ---------- Startdialog (§6.7) ---------- */
  var startDialog = $("#start-dialog");
  var lastFocus = null;

  /* Dialog-Zustand – überlebt jeden geöffneten Mini-Picker (Gate G1) */
  var dlg = {
    mic: { ok: true, name: "Standardmikrofon" },
    sys: { ok: true, name: "Lautsprecher / Headset" },
    provider: "lokal",          // lokal | azure | custom
    azureKey: false,
    lang: "Deutsch",
    project: null               // null = Kein Projekt (gültiger Zustand → Eingang)
  };
  var inputDevices = [{ name: "Standardmikrofon" }, { name: "USB-Headset" }];
  var outputDevices = [{ name: "Lautsprecher / Headset" }, { name: "HDMI-Monitor" }];
  var projects = ["BDNA Club", "Atlas GmbH"];
  var openPicker = null;        // 'mic' | 'sys' | 'provider' | 'lang' | 'project' | null

  function esc(s) {
    return String(s).replace(/[&<>"']/g, function (c) {
      return { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c];
    });
  }

  function resetDialogState() {
    // Provider-Auswahl aus der Ersteinrichtung (§6.6) bleibt erhalten
    var keepProvider = dlg.provider || "lokal";
    dlg.mic = { ok: true, name: "Standardmikrofon" };
    dlg.sys = { ok: true, name: "Lautsprecher / Headset" };
    dlg.provider = keepProvider;
    dlg.azureKey = false;
    dlg.lang = "Deutsch";
    dlg.project = null;
    openPicker = null;
    $("#row-picker").hidden = true;
    $("#project-picker").hidden = true;
    renderDialog();
  }

  function providerValueHtml() {
    if (dlg.provider === "azure") {
      return "Azure Cloud " + (dlg.azureKey
        ? '<span class="ok">✓ Schlüssel vorhanden</span>'
        : '<span class="warn-text">⚠ Schlüssel fehlt</span>');
    }
    if (dlg.provider === "custom") return 'Custom <span class="muted">Eigener Endpoint</span>';
    return 'Lokal · Meeting-Modell <span class="muted">Auf diesem Gerät</span>';
  }

  /* Gründe, warum „Meeting starten“ gerade deaktiviert ist (Gate G3) */
  function startBlockers() {
    var b = [];
    if (!dlg.mic.ok) b.push("Kein Mikrofon verfügbar – in der Zeile „Mikrofon“ beheben.");
    if (!dlg.sys.ok) b.push("Kein Systemaudio-Gerät – in der Zeile „Systemaudio“ beheben.");
    if (dlg.provider === "azure" && !dlg.azureKey)
      b.push("Azure-Schlüssel fehlt – in der Zeile „Transkription“ hinzufügen.");
    return b;
  }

  function renderDialog() {
    $("#row-mic-value").innerHTML = dlg.mic.ok
      ? esc(dlg.mic.name) + ' <span class="ok">✓ funktioniert</span>'
      : '<span class="warn-text">' + esc(dlg.mic.name) + ' ⚠ Aufmerksamkeit nötig</span>';
    $("#row-sys-value").innerHTML = dlg.sys.ok
      ? esc(dlg.sys.name) + ' <span class="ok">✓ funktioniert</span>'
      : '<span class="warn-text">' + esc(dlg.sys.name) + ' ⚠ Aufmerksamkeit nötig</span>';
    $("#row-provider-value").innerHTML = providerValueHtml();
    $("#row-lang-value").innerHTML =
      esc(dlg.lang) + ' <span class="link-like">Ändern</span>';

    var chip = $("#project-chip");
    chip.textContent = dlg.project || "Kein Projekt";
    chip.classList.toggle("chip-selected", !!dlg.project);
    chip.classList.toggle("chip-none", !dlg.project);

    var blockers = startBlockers();
    $("#btn-meeting-starten").disabled = blockers.length > 0;
    var bl = $("#start-blocker");
    bl.hidden = blockers.length === 0;
    bl.textContent = blockers.join(" ");
  }

  /* ---------- Mini-Picker (§6.7: eine Zeile = eine Entscheidung) ---------- */
  function optionButton(label, sub, selected, onClick, extraClass) {
    var b = document.createElement("button");
    b.type = "button";
    b.className = "picker-option" + (extraClass ? " " + extraClass : "");
    b.setAttribute("aria-pressed", String(!!selected));
    b.innerHTML = "<strong>" + esc(label) + "</strong>" + (sub ? '<span class="opt-sub">' + sub + "</span>" : "");
    b.addEventListener("click", onClick);
    return b;
  }
  function backButton(host) {
    var b = document.createElement("button");
    b.type = "button";
    b.className = "btn btn-ghost btn-small picker-back";
    b.textContent = "Zurück";
    b.addEventListener("click", function () { closePicker(true); });
    return b;
  }
  function pickerHead(title) {
    var head = document.createElement("div");
    head.className = "picker-head";
    var h = document.createElement("h3");
    h.textContent = title;
    head.appendChild(h);
    head.appendChild(backButton());
    return head;
  }
  function divider() {
    var d = document.createElement("div");
    d.className = "picker-divider";
    return d;
  }

  function devicePickerHtml(host, kind) {
    var key = kind === "mic" ? "mic" : "sys";
    var devices = kind === "mic" ? inputDevices : outputDevices;
    host.appendChild(pickerHead(kind === "mic" ? "Mikrofon wählen" : "Systemaudio-Gerät wählen"));
    devices.forEach(function (d) {
      host.appendChild(optionButton(
        d.name,
        kind === "mic" ? "Eingabe" : "Ausgabe · Loopback",
        dlg[key].ok && dlg[key].name === d.name,
        function () {
          dlg[key] = { ok: true, name: d.name };
          renderDialog(); refreshPicker(d.name);
          announce(d.name + " ausgewählt.");
        }));
    });
    host.appendChild(divider());
    if (!dlg[key].ok) {
      var note = document.createElement("p");
      note.className = "sim-note warn-text";
      note.textContent = "Kein Gerät verbunden – „Meeting starten“ ist deaktiviert.";
      host.appendChild(note);
    } else {
      host.appendChild(optionButton(
        "Simulation: Gerät entfernen",
        "Prototyp-Mock: zeigt den deaktivierten Start-Button (Gate G3)",
        false,
        function () {
          dlg[key] = { ok: false, name: "Kein Gerät" };
          renderDialog(); refreshPicker("Simulation: Gerät entfernen");
          announce("Simuliert: kein Gerät. Meeting starten ist deaktiviert.");
        }, "picker-sim"));
    }
  }

  function providerPickerInto(host) {
    host.appendChild(pickerHead("Verarbeitung wählen"));
    host.appendChild(optionButton(
      "Lokal auf diesem Gerät", "Datenschutzfreundliche Empfehlung · Auf diesem Gerät",
      dlg.provider === "lokal",
      function () { dlg.provider = "lokal"; renderDialog(); refreshPicker("Lokal auf diesem Gerät"); announce("Verarbeitung: lokal."); }));
    host.appendChild(optionButton(
      "Azure Cloud",
      dlg.azureKey ? "Schlüssel vorhanden ✓" : "Audio wird hochgeladen · Schlüssel fehlt ⚠",
      dlg.provider === "azure",
      function () { dlg.provider = "azure"; renderDialog(); refreshPicker(); announce("Verarbeitung: Azure Cloud."); }));
    if (dlg.provider === "azure" && !dlg.azureKey) {
      host.appendChild(optionButton(
        "Azure-Schlüssel hinzufügen (Mock)",
        "Setzt im Prototyp einen vorhandenen Schlüssel",
        false,
        function () { dlg.azureKey = true; renderDialog(); refreshPicker("Azure-Schlüssel hinzufügen (Mock)"); announce("Azure-Schlüssel gesetzt. Meeting starten aktiviert."); },
        "picker-sim"));
    }
    var more = document.createElement("details");
    more.className = "more-options";
    more.innerHTML = "<summary>Weitere Optionen</summary>";
    more.appendChild(optionButton(
      "Custom", "Eigener Provider mit Endpoint und Schlüssel",
      dlg.provider === "custom",
      function () { dlg.provider = "custom"; renderDialog(); refreshPicker("Custom"); announce("Verarbeitung: Custom."); }));
    host.appendChild(more);
  }

  function langPickerInto(host) {
    host.appendChild(pickerHead("Sprache wählen"));
    ["Deutsch", "English", "Français", "Español"].forEach(function (l) {
      host.appendChild(optionButton(l, "", dlg.lang === l, function () {
        dlg.lang = l; renderDialog(); refreshPicker(l); announce("Sprache: " + l);
      }));
    });
  }

  function projectPickerInto(host) {
    host.appendChild(pickerHead("Projekt wählen"));
    // „Kein Projekt“ ist gültig und sichtbar unterschieden (gestrichelt, Eingang-Hinweis)
    host.appendChild(optionButton(
      "📥 Kein Projekt", "Ohne Zuordnung – landet im Eingang",
      !dlg.project,
      function () { dlg.project = null; renderDialog(); refreshPicker("📥 Kein Projekt"); announce("Kein Projekt gewählt. Meeting landet im Eingang."); },
      "picker-none-opt"));
    projects.forEach(function (p) {
      host.appendChild(optionButton(p, "Projekt", dlg.project === p, function () {
        dlg.project = p; renderDialog(); refreshPicker(p); announce("Projekt: " + p);
      }));
    });
  }

  function buildPicker(host, row) {
    host.innerHTML = "";
    if (row === "mic" || row === "sys") devicePickerHtml(host, row);
    else if (row === "provider") providerPickerInto(host);
    else if (row === "lang") langPickerInto(host);
    else if (row === "project") projectPickerInto(host);
  }

  function setTriggerExpanded(row, expanded) {
    var trigger = row === "project"
      ? $("#project-chip")
      : document.querySelector('.status-line-btn[data-row="' + row + '"]');
    if (trigger) trigger.setAttribute("aria-expanded", String(!!expanded));
  }

  function openRowPicker(row) {
    closePicker(false);
    openPicker = row;
    var host = row === "project" ? $("#project-picker") : $("#row-picker");
    buildPicker(host, row);
    host.hidden = false;
    setTriggerExpanded(row, true);
    var fb = host.querySelector("button");
    if (fb) fb.focus();
  }
  function closePicker(refocus) {
    $("#row-picker").hidden = true;
    $("#project-picker").hidden = true;
    var prev = openPicker;
    openPicker = null;
    if (prev) setTriggerExpanded(prev, false);
    if (refocus !== false && prev) {
      var target = prev === "project"
        ? $("#project-chip")
        : document.querySelector('.status-line-btn[data-row="' + prev + '"]');
      if (target) target.focus();
    }
  }
  /* Picker-Inhalt nach Zustandsänderung neu zeichnen, Fokus möglichst halten */
  function refreshPicker(focusLabel) {
    if (!openPicker) return;
    var host = openPicker === "project" ? $("#project-picker") : $("#row-picker");
    buildPicker(host, openPicker);
    if (focusLabel) {
      var btns = Array.prototype.filter.call(host.querySelectorAll("button"), function (b) {
        return b.textContent.indexOf(focusLabel.replace("📥 ", "")) !== -1 ||
               b.textContent === focusLabel;
      });
      if (btns.length) btns[0].focus();
    }
  }

  /* §6.7: Der Dialog merkt sich die letzte Konfiguration über mehrere
     Öffnungen; nur der allererste Open (und „Mock zurücksetzen“) startet
     von den Defaults. */
  var dlgInitialized = false;

  function openStartDialog() {
    // Nur ein Meeting gleichzeitig (Plan §6.5: aktives Meeting steht immer oben)
    if (state.sessionActive) {
      announce("Es läuft bereits ein Meeting.");
      location.hash = "#/meeting-aktiv";
      return;
    }
    lastFocus = document.activeElement;
    if (!dlgInitialized) {
      resetDialogState();
      dlgInitialized = true;
    } else {
      closePicker(false);   // keinen veralteten Mini-Picker in den Dialog schleppen
    }
    startDialog.hidden = false;
    var ts = new Date();
    $("#meeting-title-input").value = ts.toLocaleDateString("de-DE") + " " +
      ts.toLocaleTimeString("de-DE", { hour: "2-digit", minute: "2-digit" }) + " Uhr";
    $("#meeting-title-input").focus();
  }
  window.closeStartDialog = function () {
    closePicker(false);
    startDialog.hidden = true;
    if (lastFocus) lastFocus.focus();
  };
  ["btn-neues-meeting", "btn-neues-meeting-empty"].forEach(function (id) {
    $("#" + id).addEventListener("click", openStartDialog);
  });

  /* Statuszeilen: komplett anklickbar → scoped Mini-Picker (Gate G1) */
  document.querySelectorAll(".status-line-btn").forEach(function (btn) {
    btn.addEventListener("click", function () {
      if (openPicker === this.dataset.row) closePicker(true);
      else openRowPicker(this.dataset.row);
    });
  });
  $("#project-chip").addEventListener("click", function () {
    if (openPicker === "project") closePicker(true);
    else openRowPicker("project");
  });
  $("#btn-weitere-einstellungen").addEventListener("click", function () {
    announce("Weitere Einstellungen – im Prototyp nicht implementiert.");
  });

  startDialog.addEventListener("click", function (e) {
    if (e.target !== startDialog) return;
    if (openPicker) { closePicker(true); return; }   // erster Klick schließt nur den Picker
    window.closeStartDialog();
  });
  /* Escape schließt zuerst den Mini-Picker, erst dann den Dialog (Zustand bleibt) */
  document.addEventListener("keydown", function (e) {
    if (e.key !== "Escape" || startDialog.hidden) return;
    if (openPicker) closePicker(true);
    else window.closeStartDialog();
  });

  /* Einfacher Fokus-Fall: Tab bleibt im offenen modalen Dialog (§6.14). */
  document.addEventListener("keydown", function (e) {
    if (e.key !== "Tab") return;
    var modal = !$("#stop-confirm").hidden ? $("#stop-confirm .modal")
      : (!$("#countdown-overlay").hidden ? $("#countdown-overlay .modal")
      : (!startDialog.hidden ? startDialog.querySelector(".modal") : null));
    if (!modal) return;
    var focusables = Array.prototype.filter.call(
      modal.querySelectorAll("button, input, select, a[href]"),
      function (el) { return el.offsetParent !== null; });
    if (!focusables.length) return;
    var first = focusables[0], last = focusables[focusables.length - 1];
    if (e.shiftKey && document.activeElement === first) { last.focus(); e.preventDefault(); }
    else if (!e.shiftKey && document.activeElement === last) { first.focus(); e.preventDefault(); }
  });

  $("#start-form").addEventListener("submit", function (e) {
    e.preventDefault();
    state.activeTitle = $("#meeting-title-input").value || "Meeting";
    window.closeStartDialog();
    beginCountdown();   // §6.9: Aufnahme beginnt erst nach 3-Sekunden-Countdown
  });

  /* ---------- 3-Sekunden-Countdown (§6.9) ----------
     Escape bricht nur den Countdown ab; vorher läuft keine Aufnahme
     und es erscheint nirgends ein roter Aufnahmezustand (Gate G3). */
  var countdownIv = null;

  function beginCountdown() {
    var ov = $("#countdown-overlay");
    setCountdownNumber(3);
    ov.hidden = false;
    announce("Meeting startet in 3 Sekunden. Escape bricht ab.");
    $("#btn-countdown-abbruch").focus();
    var n = 3;
    countdownIv = window.setInterval(function () {
      n -= 1;
      if (n <= 0) { finishCountdown(); return; }
      setCountdownNumber(n);
      announce(n + " …");
    }, 1000);
  }
  function setCountdownNumber(n) {
    $("#countdown-number").textContent = String(n);
  }
  function finishCountdown() {
    if (countdownIv) { window.clearInterval(countdownIv); countdownIv = null; }
    $("#countdown-overlay").hidden = true;
    startSession();
  }
  function cancelCountdown() {
    if (!countdownIv && $("#countdown-overlay").hidden) return;
    if (countdownIv) { window.clearInterval(countdownIv); countdownIv = null; }
    $("#countdown-overlay").hidden = true;
    // Zurück in den Startdialog; dessen Zustand bleibt erhalten (§6.9)
    startDialog.hidden = false;
    $("#meeting-title-input").focus();
    announce("Countdown abgebrochen. Es wurde nichts aufgenommen.");
  }
  $("#btn-countdown-abbruch").addEventListener("click", cancelCountdown);
  document.addEventListener("keydown", function (e) {
    if (e.key === "Escape" && !$("#countdown-overlay").hidden) cancelCountdown();
  });

  function startSession() {
    state.sessionActive = true;
    state.paused = false;
    state.seconds = 0;
    announce("Aufnahme läuft. " + state.activeTitle);
    startSegments();
    $("#btn-zum-text").hidden = true;
    renderRecordingStatus();
    location.hash = "#/meeting-aktiv";
  }

  /* ---------- Laufende Ansicht (§6.11) ---------- */
  var demoSegments = [
    [2, "Wir starten mit dem Tagesordnungspunkt Budget."],
    [4, "Ich schlage vor, das Marketingbudget um zehn Prozent zu erhöhen."],
    [6, "Einwand: Der Messestand braucht dieses Jahr mehr Mittel."],
    [8, "Entscheidung: Wir verschieben die Entscheidung um eine Woche."],
    [10, "Nächster Punkt: Personalplanung für das vierte Quartal."],
    [12, "Vorschlag: Zwei zusätzliche Werkstudenten für den Support."],
    [14, "Einwand: Das Budget dafür ist noch nicht bewilligt."],
    [16, "Wir prüfen die Zahlen bis Donnerstag und melden uns."],
    [18, "Abschließend: Das Protokoll geht heute Abend an alle."],
    [20, "Damit ist der Tagesordnungspunkt Budget abgeschlossen."]
  ];
  /* Demo-Zeitlupe: eine „Segmentsekunde“ entspricht 500 ms Echtzeit. Ein
     Intervall-Treiber statt fester Timeouts, damit pausierte Segmente beim
     Fortsetzen nachrücken, statt verworfen zu werden. */
  var SEG_MS = 500;
  var segDriver = null, segClock = 0, segFired = [];
  var nextFillerAt = Infinity, fillerCount = 0;
  var sessionCaptured = [];   // fertige Segmente der laufenden Session (§6.13)

  function startSegments() {
    stopSegments();
    // Neue Session beginnt wieder unten klebend – sonst würde eine
    // hochgescrollte Vorgängersession die neue stumm stehen lassen (§6.11)
    stickToBottom = true;
    $("#btn-zum-text").hidden = true;
    $("#live-segments").innerHTML = "";
    $("#listening-state").textContent = "Hört zu …";
    sessionCaptured = [];   // Transkript der Session für die Detailansicht (§6.13)
    segClock = 0;
    segFired = demoSegments.map(function () { return false; });
    nextFillerAt = Infinity;
    fillerCount = 0;
    segDriver = window.setInterval(function () {
      if (!state.sessionActive || state.paused) return;   // Pause stoppt die Demo-Zeit
      segClock += 100;
      var scriptedLeft = false;
      demoSegments.forEach(function (seg, i) {
        if (!segFired[i]) {
          if (segClock >= seg[0] * SEG_MS) {
            segFired[i] = true;
            appendSegment(seg, i);
          } else {
            scriptedLeft = true;
          }
        }
      });
      // Danach wächst das Transkript weiter wie in einem echten Meeting –
      // dadurch ist der Scroll-Lock jederzeit erreichbar (§6.11).
      if (!scriptedLeft && nextFillerAt === Infinity) nextFillerAt = segClock + 1000;
      if (segClock >= nextFillerAt && fillerCount < 40) {
        nextFillerAt += 1000;
        fillerCount += 1;
        appendSegment([Math.round(segClock / SEG_MS),
                       "Weiteres fertiges Segment aus dem Meetingverlauf (Mock) …"], -1);
      }
    }, 100);
  }
  function stopSegments() {
    if (segDriver) { window.clearInterval(segDriver); segDriver = null; }
  }
  function appendSegment(seg, i) {
    var li = document.createElement("li");
    li.innerHTML = "<span class='segment-time'>+" + seg[0] + "s</span>" + esc(seg[1]);
    $("#live-segments").appendChild(li);
    // Für die spätere Detailansicht mitschneiden (§6.13)
    sessionCaptured.push({ t: seg[0], text: seg[1] });
    // Nur am unteren Rand mitlaufen; sonst bleibt die Ansicht stehen (§6.11)
    if (stickToBottom) transcriptArea.scrollTop = transcriptArea.scrollHeight;
    $("#listening-state").textContent =
      i === demoSegments.length - 1 ? "Abschnitt wird transkribiert …" : "Hört zu …";
  }
  /* Timeline-Marker (Pause/Fortsetzen); Icon+Text, nie nur Farbe */
  function addTimelineMarker(text) {
    var li = document.createElement("li");
    li.className = "timeline-marker";
    li.innerHTML = "<span class='segment-time'>" + fmt(elapsedSec()) + "</span>" +
      "<span aria-hidden='true'>" + text.icon + "</span> " + esc(text.label);
    $("#live-segments").appendChild(li);
    if (stickToBottom) transcriptArea.scrollTop = transcriptArea.scrollHeight;
  }

  $("#btn-open-active").addEventListener("click", function () {
    location.hash = "#/meeting-aktiv";
  });

  /* Kein automatisches Springen, solange der Nutzer nach oben gescrollt hat;
     stattdessen erscheint der Button „Zum aktuellen Text“ (Plan §6.11). */
  var transcriptArea = $(".transcript-area");
  var stickToBottom = true;
  transcriptArea.addEventListener("scroll", function () {
    // „Near bottom“ darf nie wahr sein, wenn der Nutzer ganz oben steht –
    // auch nicht, wenn der Bereich bisher nur wenig überläuft. Deshalb ist
    // die Toleranz durch die maximale Scroll-Distanz begrenzt (§6.11).
    var maxScroll = transcriptArea.scrollHeight - transcriptArea.clientHeight;
    var distance = maxScroll - transcriptArea.scrollTop;
    var nearBottom = maxScroll === 0 || distance < Math.min(24, maxScroll);
    stickToBottom = nearBottom;
    $("#btn-zum-text").hidden = nearBottom || !state.sessionActive;
  });
  $("#btn-zum-text").addEventListener("click", function () {
    transcriptArea.scrollTop = transcriptArea.scrollHeight;
    this.hidden = true;
    announce("Zum aktuellen Text gesprungen.");
  });

  /* Zentrale Zustandsdarstellung: Text+Icon immer gemeinsam, Rot nur bei
     echter Aufnahme, Orange bei Pause (§6.8 / §6.11 – Gate G3) */
  function providerLabel() {
    return dlg.provider === "azure" ? "Azure Cloud"
      : dlg.provider === "custom" ? "Custom" : "Lokal";
  }
  function renderRecordingStatus() {
    var rec = state.sessionActive && !state.paused;
    var chipClass = "status-chip " + (rec ? "status-recording" : "status-paused");
    $("#live-status-chip").className = chipClass;
    $("#live-status-icon").textContent = rec ? "●" : "⏸";
    $("#live-status-text").textContent = rec ? "Aufnahme läuft" : "Pausiert";
    $("#live-provider").textContent = providerLabel();   // lesbar, nicht änderbar (§6.11)
    $("#am-status").className = chipClass;
    $("#am-status-icon").textContent = rec ? "●" : "⏸";
    $("#am-status-text").textContent = rec ? "Aufnahme läuft" : "Pausiert";
    document.querySelector(".pinned-label").textContent =
      rec ? "Aufnahme läuft" : "Pausiert";
    $("#pinned-recording").classList.toggle("is-paused", state.sessionActive && state.paused);
  }

  $("#btn-pause").addEventListener("click", function () {
    if (!state.sessionActive) return;
    state.paused = !state.paused;
    renderRecordingStatus();
    if (state.paused) {
      this.textContent = "▶ Fortsetzen";
      $("#listening-state").textContent = "Pausiert.";
      // Pegel sofort auf null – während der Pause wird kein Audio gespeichert
      $("#mic-level").value = 0;
      $("#sys-level").value = 0;
      // Plan „Pause“: sauberer Abschnittsende + Timeline-Marker
      addTimelineMarker({ icon: "⏸", label: "Pausiert – Aufnahme unterbrochen" });
      announce("Aufnahme pausiert.");
    } else {
      this.textContent = "Pause";
      $("#listening-state").textContent = "Hört zu …";
      addTimelineMarker({ icon: "▶", label: "Fortgesetzt" });
      announce("Aufnahme fortgesetzt. Geräte erneut geprüft.");
    }
  });

  /* §6.11: Minimieren/zum Tray; Aufnahmezustand bleibt im Pin sichtbar */
  $("#btn-tray").addEventListener("click", function () {
    announce("Fenster minimiert – die Aufnahme läuft im Tray weiter (Mock).");
    location.hash = "#/meetings";
  });

  /* ---------- Stop mit Pflicht-Bestätigung (§6.9, Gate G1) ----------
     Ein einzelner Druck auf „Meeting beenden“ beendet nie direkt; die
     Bestätigung verschwindet nach kurzer Zeit ohne Zustandsänderung. */
  var confirmTimer = null;

  function openStopConfirm() {
    if (!state.sessionActive || !$("#stop-confirm").hidden) return;
    $("#stop-confirm").hidden = false;
    $("#btn-stop-abbruch").focus();
    confirmTimer = window.setTimeout(function () {
      closeStopConfirm();
      announce("Bestätigung abgelaufen. Die Aufnahme läuft weiter.");
    }, 10000);
  }
  function closeStopConfirm() {
    $("#stop-confirm").hidden = true;
    if (confirmTimer) { window.clearTimeout(confirmTimer); confirmTimer = null; }
  }

  $("#btn-beenden").addEventListener("click", openStopConfirm);
  $("#btn-stop-abbruch").addEventListener("click", function () {
    closeStopConfirm();
    $("#btn-beenden").focus();
    announce("Aufnahme läuft weiter.");
  });
  $("#btn-stop-ok").addEventListener("click", function () {
    closeStopConfirm();
    var dur = fmt(elapsedSec());
    state.sessionActive = false;
    state.paused = false;
    state.seconds = 0;
    // Gesamtmenge der Abschnitte ist nach dem Speichern deterministisch
    // bekannt und treibt die Checklisten-Zählung (§6.12, Gate G1)
    var chunksTotal = Math.max(sessionCaptured.length * 2, 6);
    var newMeeting = {
      id: nextMeetingId(),
      title: state.activeTitle || "Meeting", date: "Gerade eben", duration: dur,
      provider: providerLabel(), status: "verarbeitung",
      project: dlg.project || null,   // §6.18: nur sichtbare Vorauswahl, kein Zwang
      summary: null,
      decisions: [], tasks: [], questions: [],
      segments: sessionCaptured.map(function (s) {
        var mm = Math.floor(s.t / 60), ss = String(s.t % 60).padStart(2, "0");
        return [mm + ":" + ss, s.text];
      }),
      audio: true,
      processing: { provider: providerLabel(), model: "Whisper Small",
                    chunksTotal: chunksTotal,
                    transcribed: Math.min(1, chunksTotal),
                    failedChunks: 0, retries: 0 }
    };
    sessionCaptured = [];
    state.meetings.unshift(newMeeting);
    state.lastProcessingId = newMeeting.id;
    stopSegments();
    // Checklisten-DOM synchron zurücksetzen, damit keine fertigen Zeilen oder
    // alten Vorschläge des vorherigen Meetings sichtbar bleiben, bis die neue
    // Checkliste startet (§6.12, Gates G1/G2)
    ["saved", "chunks", "transcribe", "summary"].forEach(function (s) {
      var li = document.querySelector('#proc-checklist li[data-step="' + s + '"]');
      if (li) li.className = "";
    });
    $("#ai-suggestion").hidden = true;
    announce("Aufnahme beendet. Audio sicher gespeichert.");
    location.hash = "#/verarbeitung";
  });
  document.addEventListener("keydown", function (e) {
    if (e.key === "Escape" && !$("#stop-confirm").hidden) {
      closeStopConfirm();
      $("#btn-beenden").focus();
      announce("Beenden abgebrochen. Die Aufnahme läuft weiter.");
      return;
    }
    // Escape schließt offene Karten- und Detailmenüs (§6.14: vollständige
    // Tastaturbedienung – ein mit der Tastatur geöffnetes Menü muss auch
    // wieder per Tastatur schließbar sein)
    if (e.key !== "Escape") return;
    var more = $("#detail-more-menu");
    if (document.querySelector(".menu:not(.menu-static)") || (more && more.open)) {
      closeMenus();
      announce("Menü geschlossen.");
    }
  });

  /* ---------- Verarbeitungs-Checkliste (§6.12) ----------
     Deterministische Stückzahlen aus den gespeicherten Meeting-Daten.
     Keine Prozentanzeige; solange die Gesamtmenge unbekannt ist (Phase
     „vorbereiten“), steht bewusst keine Zahl im Text (Gate G1). */
  var procTimers = [];

  function clearProcTimers() {
    procTimers.forEach(function (t) { window.clearTimeout(t); window.clearInterval(t); });
    procTimers = [];
  }
  function procLater(ms, fn) { procTimers.push(window.setTimeout(fn, ms)); }

  function checklistMeeting() {
    return meetingById(state.lastProcessingId) || state.meetings[0] || null;
  }

  function setStep(step, cls, text) {
    var li = document.querySelector('#proc-checklist li[data-step="' + step + '"]');
    if (!li) return;
    li.className = cls || "";
    li.textContent = text;
  }

  function runChecklist() {
    clearProcTimers();
    $("#btn-retry-failed").hidden = true;
    $("#btn-open-processing-meeting").hidden = true;
    $("#ai-suggestion").hidden = true;   // Vorschlag erst nach Abschluss (§6.18)
    var m = checklistMeeting();
    if (!m) return;
    m.status = "verarbeitung";
    $("#proc-meeting-label").textContent = "„" + m.title + "“ · " + m.duration;
    // Schritt 1 ist erledigt: Aufnahme liegt sicher vor
    setStep("saved", "done", "✓ Aufnahme sicher gespeichert");
    /* Phase Vorbereitung: Gesamtmenge noch unbekannt → bewusst OHNE Stückzahl
       und ohne Prozentangabe (Gate G1: keine erfundene Fortschrittszahl) */
    setStep("chunks", "", "● Audioabschnitte werden vorbereitet …");
    setStep("transcribe", "", "○ Abschnitte transkribieren …");
    setStep("summary", "", "○ Zusammenfassung wartet");

    var total = m.processing.chunksTotal;

    procLater(600, function () {
      // Jetzt ist die Gesamtmenge bekannt → deterministische Stückzahl
      m.processing.transcribed = 0;
      setStep("chunks", "done",
        "✓ " + total + " von " + total + " Audioabschnitten vorbereitet");

      // Transkription in vier deterministischen Etappen; optional scheitert
      // genau ein Abschnitt (Mock-Simulation, nicht zufällig)
      var simulateError = $("#sim-chunk-error").checked;
      var stages = [Math.round(total * 0.25), Math.round(total * 0.5),
                    Math.round(total * 0.75), total];
      // Offsets relativ zum Vorbereitungs-Abschluss; absolut also
      // 1100/1600/2100/2600 ms nach Checkliststart (deterministisch)
      var stageTimes = [500, 1000, 1500, 2000];
      stages.forEach(function (upto, idx) {
        procLater(stageTimes[idx], function () {
          if (simulateError && idx === stages.length - 1) {
            // Ein Abschnitt dauerhaft fehlgeschlagen → Wiederholung anbieten
            m.processing.transcribed = total - 1;
            m.processing.failedChunks = 1;
            setStep("transcribe", "error",
              "✗ " + (total - 1) + " von " + total + " Abschnitten transkribiert · 1 fehlerhaft");
            $("#btn-retry-failed").hidden = false;
            announce("Ein Abschnitt konnte nicht transkribiert werden.");
            return;
          }
          m.processing.transcribed = upto;
          setStep("transcribe", "",
            "● " + upto + " von " + total + " Abschnitten transkribiert");
          if (idx === stages.length - 1) finishChecklist(m);
        });
      });
    });
  }

  function finishChecklist(m) {
    m.processing.failedChunks = 0;
    m.processing.transcribed = m.processing.chunksTotal;
    setStep("transcribe", "done", "✓ " + m.processing.chunksTotal + " von " +
      m.processing.chunksTotal + " Abschnitten transkribiert");
    procLater(700, function () {
      m.status = "fertig";
      m.summary = "Automatische Kurzfassung: Besprechung abgeschlossen, " +
        "Ergebnisse wurden aus " + m.segments.length + " Transkriptsegmenten extrahiert (Mock).";
      setStep("summary", "done", "✓ Zusammenfassung erstellt");
      $("#btn-retry-failed").hidden = true;
      $("#btn-open-processing-meeting").hidden = false;   // direkt ins Detail (§6.13)
      // §6.18 (Gate G2): nach dem Stop darf ein Projektvorschlag erscheinen –
      // aber nie ohne explizite Wahl des Nutzers wird etwas verschoben
      if (!m.project) showProjectSuggestion(m);
      announce("Verarbeitung abgeschlossen. Systembenachrichtigung gesendet (Mock).");
    });
  }

  $("#btn-retry-failed").addEventListener("click", function () {
    var m = checklistMeeting();
    if (!m) return;
    this.hidden = true;
    m.processing.retries += 1;
    setStep("transcribe", "", "● Fehlerhafte Abschnitte werden erneut versucht …");
    announce("Fehlerhafte Abschnitte werden erneut versucht.");
    procLater(900, function () { finishChecklist(m); });
  });

  /* Im Hintergrund weiterarbeiten: zurück zur Liste; Verarbeitung läuft
     laut Mock weiter und das Meeting bleibt dort sichtbar (§6.12) */
  $("#btn-proc-bg").addEventListener("click", function () {
    announce("Verarbeitung läuft im Hintergrund weiter.");
    location.hash = "#/meetings";
  });

  /* Direkt aus der Checkliste ins Detail des verarbeiteten Meetings (§6.13) */
  $("#btn-open-processing-meeting").addEventListener("click", function () {
    var m = checklistMeeting();
    if (m) state.selectedId = m.id;
    location.hash = "#/meeting-detail";
  });

  window.addEventListener("hashchange", function () {
    if (currentRoute() === "verarbeitung") runChecklist();
  });

  /* ---------- Meeting-Detailansicht (§6.13) ----------
     Tabs Übersicht/Transkript/Audio/Verarbeitung; standardmäßig Übersicht;
     optionale Geteilte Ansicht nur auf breiten Bildschirmen (§6.13). */

  function statusChipHtml(m) {
    if (m.status === "fertig") return '<span class="status-chip status-ready">✓ Fertig</span>';
    if (m.status === "fehler") return '<span class="status-chip status-recording">✗ Fehler</span>';
    return '<span class="status-chip status-processing">⟳ In Verarbeitung</span>';
  }

  function renderDetail() {
    var m = meetingById(state.selectedId);
    if (!m) { location.hash = "#/meetings"; return; }
    $("#detail-title").value = m.title;
    $("#detail-date").textContent = m.date;
    $("#detail-duration").textContent = m.duration;
    $("#detail-provider-line").textContent = m.provider +
      (m.processing ? " · " + m.processing.model : "");
    $("#detail-status-chip").innerHTML = statusChipHtml(m);

    // Übersicht
    $("#detail-summary").textContent = m.summary !== null && m.summary !== undefined
      ? m.summary : "Zusammenfassung wird nach Abschluss der Verarbeitung erstellt …";
    fillSimpleList("#detail-decisions", m.decisions,
      "Noch keine Entscheidungen erkannt.");
    fillTaskList("#detail-tasks", m.tasks);
    fillSimpleList("#detail-questions", m.questions,
      "Keine offenen Fragen erkannt.");

    renderDetailTranscript(m);
    renderAudioPanel(m);
    renderProcessingPanel(m);

    // Standard-Tab ist Übersicht (§6.13); Split-Ansicht nicht als Default
    selectDetailTab("uebersicht");
    $("#split-toggle").checked = false;
    $("#view-meeting-detail").classList.remove("split-active");
    $("#detail-more-menu").open = false;
  }

  function fillSimpleList(sel, items, emptyText) {
    var ul = $(sel);
    ul.innerHTML = "";
    if (!items || !items.length) {
      var li0 = document.createElement("li");
      li0.className = "muted small";
      li0.textContent = emptyText;
      ul.appendChild(li0);
      return;
    }
    items.forEach(function (it) {
      var li = document.createElement("li");
      li.textContent = typeof it === "string" ? it : it.text;
      ul.appendChild(li);
    });
  }
  function fillTaskList(sel, tasks) {
    var ul = $(sel);
    ul.innerHTML = "";
    if (!tasks || !tasks.length) {
      var li0 = document.createElement("li");
      li0.className = "muted small";
      li0.textContent = "Noch keine Aufgaben erkannt.";
      ul.appendChild(li0);
      return;
    }
    tasks.forEach(function (t) {
      var li = document.createElement("li");
      li.textContent = (t.done ? "☑ " : "☐ ") + t.text;
      ul.appendChild(li);
    });
  }

  /* Transkript-Tab inklusive Suche mit Sprung zur Segmentposition (Gate G3) */
  function renderDetailTranscript(m) {
    var ul = $("#detail-segments");
    ul.innerHTML = "";
    $("#detail-transcript-search").value = "";
    $("#transcript-search-status").textContent = "";
    (m.segments || []).forEach(function (seg, i) {
      var li = document.createElement("li");
      li.dataset.index = String(i);
      li.dataset.pos = seg[0];
      li.tabIndex = -1;
      li.innerHTML = "<span class='segment-time'>" + esc(seg[0]) + "</span>" + esc(seg[1]);
      ul.appendChild(li);
    });
    if (!(m.segments || []).length) {
      var li0 = document.createElement("li");
      li0.className = "muted";
      li0.textContent = "Keine Transkriptsegmente vorhanden.";
      ul.appendChild(li0);
    }
  }

  function runTranscriptSearch() {
    var q = $("#detail-transcript-search").value.trim().toLowerCase();
    var lis = Array.prototype.slice.call(document.querySelectorAll("#detail-segments li[data-index]"));
    var hits = [];
    lis.forEach(function (li) {
      li.classList.remove("seg-hit", "seg-hit-first");
      if (q && li.textContent.toLowerCase().indexOf(q) !== -1) hits.push(li);
    });
    var status = $("#transcript-search-status");
    if (!q) { status.textContent = ""; return; }
    if (!hits.length) {
      status.textContent = "Kein Treffer für „" + q + "“.";
      return;
    }
    hits.forEach(function (li) { li.classList.add("seg-hit"); });
    var first = hits[0];
    first.classList.add("seg-hit-first");
    // Sprung zur gemockten Audioposition des ersten Treffers (§6.13/Gate G3)
    first.scrollIntoView({ block: "center" });
    first.focus();
    status.textContent = hits.length + " Treffer · erster Treffer bei Segment " +
      (Number(first.dataset.index) + 1) + " von " + lis.length +
      " · Audioposition " + first.dataset.pos;
    announce(status.textContent);
  }
  $("#detail-transcript-search").addEventListener("input", runTranscriptSearch);

  /* Audio-Tab: Wiedergabe und Aufbewahrung, nur solange Audio vorhanden (§6.13) */
  var audioPlaying = false;
  function renderAudioPanel(m) {
    audioPlaying = false;
    var host = $("#detail-audio-panel");
    if (!m.audio) {
      host.innerHTML =
        "<h2 class='small-heading'>Audio</h2>" +
        "<p class='muted'>Das Audio wurde entfernt. Transkript und Zusammenfassung bleiben verfügbar.</p>";
      return;
    }
    host.innerHTML =
      "<h2 class='small-heading'>Aufnahme</h2>" +
      "<div class='setting-row-inline'>" +
      "<button type='button' class='btn btn-secondary btn-small' id='btn-audio-play'>▶ Wiedergabe (Mock)</button>" +
      "<div class='audio-progress' role='progressbar' aria-label='Wiedergegebene Position' " +
      "aria-valuemin='0' aria-valuemax='100' aria-valuenow='0' id='audio-progress'></div>" +
      "<span class='mono muted' id='audio-pos'>0:00 / " + esc(m.duration) + "</span></div>" +
      "<p class='muted small'>Aufbewahrung: Das Audio bleibt lokal auf diesem Gerät gespeichert, " +
      "solange das Meeting existiert.</p>" +
      "<button type='button' class='btn btn-neutral btn-small' id='btn-audio-delete'>Audio löschen (Mock)</button>";
    $("#btn-audio-play").addEventListener("click", function () {
      audioPlaying = !audioPlaying;
      this.textContent = audioPlaying ? "⏸ Pause (Mock)" : "▶ Wiedergabe (Mock)";
      announce(audioPlaying ? "Wiedergabe gestartet (Mock)." : "Wiedergabe pausiert.");
    });
    $("#btn-audio-delete").addEventListener("click", function () {
      if (!window.confirm("Audio von „" + m.title + "“ löschen?\n\nTranskript und Zusammenfassung bleiben erhalten.")) return;
      m.audio = false;
      announce("Audio gelöscht (Mock).");
      renderAudioPanel(meetingById(state.selectedId));
    });
  }

  /* Verarbeitung-Tab: Provider, Modell, Jobs, Fehler und Wiederholungen (§6.13) */
  function renderProcessingPanel(m) {
    var p = m.processing || {};
    var jobs = "<li>" + (m.status === "fertig" ? "✓" : "●") +
      " Audio vorbereiten – " + (p.chunksTotal || 0) + " Abschnitte</li>" +
      "<li>" + (m.status === "fertig" ? "✓" : "●") + " Transkription – " +
      (p.transcribed || 0) + " von " + (p.chunksTotal || 0) + " Abschnitten" +
      (p.failedChunks ? " · " + p.failedChunks + " fehlerhaft" : "") + "</li>" +
      "<li>" + (m.status === "fertig" ? "✓" : "○") + " Zusammenfassung" +
      (m.status === "fertig" ? "" : " – wartet") + "</li>";
    $("#detail-processing-panel").innerHTML =
      "<h2 class='small-heading'>Verarbeitungsdetails</h2>" +
      "<dl class='status-lines'>" +
      "<div class='status-line'><dt>Provider</dt><dd>" + esc(p.provider || m.provider) + "</dd></div>" +
      "<div class='status-line'><dt>Modell</dt><dd>" + esc(p.model || "–") + "</dd></div>" +
      "</dl>" +
      "<h2 class='small-heading'>Jobs</h2><ul class='checklist' role='list'>" + jobs + "</ul>" +
      "<h2 class='small-heading'>Fehler &amp; Wiederholungen</h2>" +
      "<p>" + (p.failedChunks
        ? "1 Abschnitt fehlgeschlagen – in der Verarbeitung-Seite erneut versuchen."
        : "Keine offenen Fehler.") +
      " Wiederholungen bisher: " + (p.retries || 0) + ".</p>";
  }

  /* Detail-Tabs */
  function selectDetailTab(name) {
    document.querySelectorAll("#view-meeting-detail .tab").forEach(function (t) {
      t.setAttribute("aria-selected", String(t.dataset.dtab === name));
    });
    document.querySelectorAll("#view-meeting-detail .tabpanel").forEach(function (p) {
      p.hidden = p.dataset.dpanel !== name;
    });
  }
  document.querySelectorAll("#view-meeting-detail .tab").forEach(function (tab) {
    tab.addEventListener("click", function () { selectDetailTab(this.dataset.dtab); });
  });
  /* Pfeiltasten-Navigation im Tablist (§6.14: vollständige Tastaturbedienung) */
  document.querySelector("#view-meeting-detail .tabs").addEventListener("keydown",
    function (e) {
      if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
      var tabs = Array.prototype.slice.call(this.querySelectorAll(".tab"));
      var i = tabs.indexOf(document.activeElement);
      if (i === -1) return;
      var n = e.key === "ArrowRight"
        ? (i + 1) % tabs.length
        : (i - 1 + tabs.length) % tabs.length;
      tabs[n].focus();
      selectDetailTab(tabs[n].dataset.dtab);
      e.preventDefault();
    });

  /* Optionale Geteilte Ansicht (nur breite Screens bieten sie an, §6.13) */
  $("#split-toggle").addEventListener("click", function () {
    // Checkbox-Wechsel erst nach dem Click-Event auswerten
    window.setTimeout(function () {
      var on = $("#split-toggle").checked;
      $("#view-meeting-detail").classList.toggle("split-active", on);
      if (on) {
        // Geteilte Ansicht zeigt Übersicht + Transkript nebeneinander
        selectDetailTab("uebersicht");
        document.querySelector('[data-dpanel="transkript"]').hidden = false;
      } else {
        selectDetailTab("uebersicht");
      }
    }, 0);
  });

  /* Titel im Detail bearbeiten → Änderung erscheint auch in der Liste (G4) */
  $("#detail-title").addEventListener("change", function () {
    var m = meetingById(state.selectedId);
    if (!m) return;
    m.title = this.value.trim() || m.title;
    this.value = m.title;
    announce("Titel geändert: " + m.title);
  });

  $("#btn-detail-copy").addEventListener("click", function () {
    var m = meetingById(state.selectedId);
    announce("Zusammenfassung kopiert (Mock): „" + ((m && m.summary) || "–") + "“");
  });
  $("#btn-detail-export").addEventListener("click", function () {
    announce("Export geöffnet (Mock).");
  });
  $("#btn-detail-reprocess").addEventListener("click", function () {
    var m = meetingById(state.selectedId);
    if (!m) return;
    $("#detail-more-menu").open = false;
    state.lastProcessingId = m.id;
    location.hash = "#/verarbeitung";
  });
  $("#btn-detail-delete").addEventListener("click", function () {
    var m = meetingById(state.selectedId);
    if (!m) return;
    // Gleiche benannte Bestätigung wie in der Liste (Gate G2)
    if (confirmDeleteMeeting(m)) {
      state.meetings = state.meetings.filter(function (x) { return x !== m; });
      state.selectedId = null;
      announce("Meeting gelöscht.");
      location.hash = "#/meetings";
    } else {
      $("#detail-more-menu").open = false;
    }
  });

  /* ---------- Uhr / Tick ---------- */
  /* Nur das Intervall lässt die gemockte Zeit weiterlaufen; render() darf
     beim Anzeigen der Uhr keine Sekunden dazuerfinden. */
  function updateClockDisplays() {
    var t = fmt(elapsedSec());
    $("#pinned-time").textContent = t;
    $("#am-duration").textContent = t;
    $("#live-duration").textContent = t;
  }
  function advanceClock() {
    if (state.sessionActive && !state.paused) state.seconds += 1;
    updateClockDisplays();
  }
  window.setInterval(advanceClock, 1000);

  /* Audio-Wiedergabe-Fortschritt im Detail-Tab (§6.13) – ein globaler Ticker,
     damit renderAudioPanel keine Intervalle ansammelt */
  window.setInterval(function () {
    if (currentRoute() !== "meeting-detail" || !audioPlaying) return;
    var bar = document.getElementById("audio-progress");
    if (!bar) return;
    var now = (Number(bar.getAttribute("aria-valuenow") || 0) + 2) % 104;
    bar.setAttribute("aria-valuenow", String(Math.min(now, 100)));
    bar.style.width = Math.min(now, 100) + "%";
  }, 300);

  /* Pegel-Demo */
  window.setInterval(function () {
    if (state.sessionActive && !state.paused) {
      $("#mic-level").value = 25 + Math.round(Math.random() * 45);
      $("#sys-level").value = 45 + Math.round(Math.random() * 45);
    } else {
      $("#mic-level").value = 0; $("#sys-level").value = 0;
    }
  }, 700);

  /* ---------- Meeting-Ersteinrichtung (§6.6) ---------- */
  var setupTimers = [];

  function clearSetupTimers() {
    setupTimers.forEach(function (t) { window.clearTimeout(t); window.clearInterval(t); });
    setupTimers = [];
  }
  /* Livepegel-Mock: animiert ~2 s und ruft dann done() auf */
  function animateMeter(meter, done) {
    meter.value = 30;
    var iv = window.setInterval(function () {
      meter.value = 25 + Math.round(Math.random() * 55);
    }, 130);
    setupTimers.push(iv);
    setupTimers.push(window.setTimeout(function () {
      window.clearInterval(iv);
      meter.value = 0;
      done();
    }, 2000));
  }

  function showAudioHint(html) {
    var box = $("#setup-audio-hints");
    box.innerHTML = html;
    box.hidden = false;
    box.querySelectorAll("[data-fix]").forEach(function (b) {
      b.addEventListener("click", function () {
        var fix = b.dataset.fix;
        if (fix === "mic-denied") $("#sim-mic-denied").checked = false;
        if (fix === "no-mic") $("#sim-no-mic").checked = false;
        if (fix === "no-output") $("#sim-no-output").checked = false;
        runMicTest(); runSysTest();
      });
    });
  }
  function hideAudioHint() {
    $("#setup-audio-hints").hidden = true;
    $("#setup-audio-hints").innerHTML = "";
  }

  function runMicTest() {
    var res = $("#result-mic"), meter = $("#setup-mic-level");
    var denied = $("#sim-mic-denied").checked, missing = $("#sim-no-mic").checked;
    hideAudioHint();
    if (missing || denied) {
      meter.value = 0;
      res.textContent = denied
        ? "✗ Zugriff auf das Mikrofon wurde verweigert"
        : "✗ Kein Mikrofon erkannt";
      res.className = "test-result test-fail";
      showAudioHint(
        "<strong>" + (denied
          ? "Speak hat keinen Zugriff auf dein Mikrofon."
          : "Es ist kein Mikrofon angeschlossen.") + "</strong>" +
        "<button type='button' class='btn btn-secondary btn-small' data-fix='" +
          (denied ? "mic-denied" : "no-mic") + "'>" +
          (denied ? "Berechtigung in den Systemeinstellungen erteilen (Mock)"
                  : "Mikrofon anschließen (Mock)") + "</button>");
      return;
    }
    res.textContent = "Pegel wird gemessen …";
    res.className = "test-result";
    animateMeter(meter, function () {
      res.textContent = "✓ funktioniert";
      res.className = "test-result test-ok";
    });
  }

  function runSysTest() {
    var res = $("#result-sys"), meter = $("#setup-sys-level");
    hideAudioHint();
    if ($("#sim-no-output").checked) {
      meter.value = 0;
      res.textContent = "✗ Kein Ausgabegerät erkannt";
      res.className = "test-result test-fail";
      showAudioHint(
        "<strong>Es ist kein Ausgabegerät angeschlossen.</strong>" +
        "<button type='button' class='btn btn-secondary btn-small' data-fix='no-output'>Ausgabegerät anschließen (Mock)</button>");
      return;
    }
    res.textContent = "Testton wird abgespielt …";
    res.className = "test-result";
    animateMeter(meter, function () {
      // Kurzer Speak-Testton; Loopback empfängt ihn (Mock)
      res.textContent = "✓ Testton abgespielt · Loopback empfängt";
      res.className = "test-result test-ok";
    });
  }

  function checkShortcut() {
    var st = $("#shortcut-cand-status");
    st.textContent = "Wird geprüft …";   // Registrierung vor Anzeige prüfen (§6.6)
    st.className = "test-result";
    setupTimers.push(window.setTimeout(function () {
      if ($("#sim-shortcut-taken").checked) {
        st.textContent = "✗ nicht verfügbar – bereits belegt";
        st.className = "test-result test-fail";
        $("#shortcut-alternatives").hidden = false;
      } else {
        st.textContent = "✓ verfügbar";
        st.className = "test-result test-ok";
        $("#shortcut-alternatives").hidden = true;
      }
    }, 500));
  }

  function initSetupView() {
    clearSetupTimers();
    ["#result-mic", "#result-sys"].forEach(function (sel) {
      var el = $(sel); el.textContent = ""; el.className = "test-result";
    });
    $("#setup-mic-level").value = 0;
    $("#setup-sys-level").value = 0;
    hideAudioHint();
    checkShortcut();
  }

  $("#btn-test-mic").addEventListener("click", runMicTest);
  $("#btn-test-sys").addEventListener("click", runSysTest);
  $("#btn-shortcut-check").addEventListener("click", checkShortcut);
  document.querySelectorAll(".alt-pick").forEach(function (b) {
    b.addEventListener("click", function () {
      $("#shortcut-cand-key").textContent = b.textContent;
      $("#shortcut-cand-status").textContent = b.textContent + " gewählt ✓";
      $("#shortcut-cand-status").className = "test-result test-ok";
      $("#shortcut-alternatives").hidden = true;
      announce("Shortcut " + b.textContent + " gewählt.");
    });
  });

  $("#btn-setup-finish").addEventListener("click", function () {
    var chosen = document.querySelector('input[name="setup-provider"]:checked');
    dlg.provider = chosen ? chosen.value : "lokal";
    location.hash = "#/meetings";
    openStartDialog();   // §6.6: nach Abschluss direkt in den kompakten Startdialog
  });
  $("#btn-setup-later").addEventListener("click", function () {
    announce("Einrichtung übersprungen.");
    location.hash = "#/meetings";
  });

  /* ---------- Diktieren-Seite (§6.4) ---------- */
  $("#btn-dictat-testen").addEventListener("click", function () {
    var preview = $("#dict-preview");
    preview.hidden = false;
    $("#dict-preview-text").textContent = "Dies ist eine Testdiktat-Vorschau. Sie wird nur hier angezeigt und in keine andere App eingefügt.";
    announce("Testdiktat fertig. Ergebnis nur in der Vorschau.");
  });

  /* ---------- Einstellungen-Tabs ---------- */
  document.querySelectorAll(".tab").forEach(function (tab) {
    tab.addEventListener("click", function () {
      selectTab(tab.dataset.tab);
    });
  });
  function selectTab(name) {
    document.querySelectorAll(".tab").forEach(function (t) {
      t.setAttribute("aria-selected", String(t.dataset.tab === name));
    });
    document.querySelectorAll(".tabpanel").forEach(function (p) {
      p.hidden = p.dataset.panel !== name;
    });
  }
  // Deep-Link #/einstellungen/diktieren
  window.addEventListener("hashchange", function () {
    var parts = location.hash.replace(/^#\/?/, "").split("/");
    if (parts[0] === "einstellungen" && parts[1]) selectTab(parts[1]);
  });

  /* ---------- Kopierbuttons im Verlauf ---------- */
  document.querySelectorAll(".copy-btn").forEach(function (b) {
    b.addEventListener("click", function () {
      announce("In die Zwischenablage kopiert (Mock).");
    });
  });

  /* ================================================================
     Projekte, Eingang, Projekt-Detail, Import (Plan §6.16–§6.19)
     Alle Daten sind Mock-Daten; Belege springen in gemockte Quellen.
     ================================================================ */

  /* ---------- Gemockte Quellen + Belegregister (Gate G1) ---------- */
  var SOURCES = {
    "src-wa": { title: "WhatsApp-Export · BDNA Club", meta: "Importiert heute, 09:05 · Zeitraum 12.01.–12.08.2026 · nur lokal",
      lines: [
        ["12.03., 18:42", "Lena", "Können wir endlich die Einladungen für den Sommerball rausschicken?"],
        ["12.03., 18:47", "Tarek", "Ja – ich mache die Einladungen diese Woche fertig."],
        ["02.06., 09:15", "Lena", "Erinnerung: Die Einladungen sind immer noch nicht raus."],
        ["03.06., 11:02", "Tarek", "Sorry, diese Woche klappt es nicht, ich verschiebe auf nächste Woche."],
        ["10.07., 20:14", "Mira", "Sollen wir für den Sommerball jetzt 80 oder 120 Getränkekarten planen?"],
        ["10.07., 20:31", "Lena", "Nehmen wir 120, beim Sommerfest waren alle Getränke nach zwei Stunden weg."],
        ["28.07., 17:20", "Tarek", "Ich schicke heute die Zahlen für die Kasse an alle, bitte bis Donnerstag prüfen."]
      ] },
    "src-tr": { title: "Transkript · Wochenplanung", meta: "Meeting von gestern, 16:00 · lokal transkribiert",
      lines: [
        ["0:04", "", "Ich schlage vor, das Marketingbudget um zehn Prozent zu erhöhen."],
        ["0:08", "", "Entscheidung: Wir verschieben die Entscheidung um eine Woche."],
        ["0:12", "", "Vorschlag: Zwei zusätzliche Werkstudenten für den Support."],
        ["0:14", "", "Einwand: Das Budget dafür ist noch nicht bewilligt."],
        ["0:16", "", "Wir prüfen die Zahlen bis Donnerstag und melden uns."]
      ] },
    "src-berger": { title: "Transkript · Kundengespräch Berger", meta: "Meeting von Montag, 10:30 · lokal transkribiert",
      lines: [
        ["0:10", "", "Herr Berger bittet um ein Angebot für zwanzig Lizenzen."],
        ["0:22", "", "Berger meldet sich bis Donnerstag mit Rückmeldung zu den Zahlen."]
      ] }
  };
  var EVIDENCE = {
    "ev-invite":  { sourceId: "src-wa", line: 0, label: "WhatsApp · 12.03., 18:42" },
    "ev-chase":   { sourceId: "src-wa", line: 2, label: "WhatsApp · 02.06., 09:15" },
    "ev-drinks":  { sourceId: "src-wa", line: 5, label: "WhatsApp · 10.07., 20:31" },
    "ev-zahlen":  { sourceId: "src-wa", line: 6, label: "WhatsApp · 28.07., 17:20" },
    "ev-budget":  { sourceId: "src-tr", line: 1, label: "Transkript · 0:08" },
    "ev-budget-q":{ sourceId: "src-tr", line: 3, label: "Transkript · 0:14" },
    "ev-license": { sourceId: "src-berger", line: 0, label: "Transkript · 0:10" },
    "ev-berger-d": { sourceId: "src-berger", line: 1, label: "Transkript · 0:22" }
  };

  function evidenceLinkHtml(evId) {
    var ev = EVIDENCE[evId];
    if (!ev) return "";
    return '<li><a href="#" class="evidence-link" data-evidence="' + evId + '">' +
      '🔗 ' + esc(ev.label) + '</a></li>';
  }

  /* Inline-Variante ohne <li> für Fließtext (z. B. Detail-Kopfzeile) */
  function evidenceAnchorHtml(evId) {
    var ev = EVIDENCE[evId];
    if (!ev) return "";
    return '<a href="#" class="evidence-link" data-evidence="' + evId + '">🔗 ' +
      esc(ev.label) + '</a>';
  }

  /* Quellenansicht: Beleg-Klick springt zur gemockten Stelle und hebt sie
     hervor (Gate G1: jeder KI-Satz/Task hat ≥1 solchen Sprunglink) */
  function openSourceViewer(evId) {
    var ev = EVIDENCE[evId];
    if (!ev) return;
    var src = SOURCES[ev.sourceId];
    $("#sv-title").textContent = src.title;
    $("#sv-meta").textContent = src.meta;
    var ul = $("#sv-lines");
    ul.innerHTML = "";
    src.lines.forEach(function (ln, i) {
      var li = document.createElement("li");
      li.className = i === ev.line ? "src-hit" : "";
      li.setAttribute("data-src-line", String(i));
      li.innerHTML = "<span class='segment-time'>" + esc(ln[0]) + "</span>" +
        (ln[1] ? "<strong>" + esc(ln[1]) + ":</strong> " : "") + esc(ln[2]);
      ul.appendChild(li);
    });
    $("#source-viewer").hidden = false;
    announce("Quelle geöffnet: " + src.title);
    window.setTimeout(function () {
      var hit = ul.querySelector(".src-hit");
      if (hit) hit.scrollIntoView({ block: "center" });
    }, 0);
  }
  document.addEventListener("click", function (e) {
    var a = e.target.closest ? e.target.closest(".evidence-link") : null;
    if (!a) return;
    e.preventDefault();
    openSourceViewer(a.getAttribute("data-evidence"));
  });
  $("#btn-sv-close").addEventListener("click", function () {
    $("#source-viewer").hidden = true;
  });

  /* ---------- Projekt-Mockdaten (§6.16/§6.17) ---------- */
  var projectSeq = 0;
  function nextProjectId() { projectSeq += 1; return "p" + projectSeq; }

  function defaultProjects() {
    return [
      { id: nextProjectId(), name: "BDNA Club",
        statusText: "In Arbeit",
        aiLastRun: "Heute, 09:12",
        lastActiveRank: 5,
        unreviewed: 3,
        overview: {
          status: [
            { text: "Die Einladungen für den Sommerball sind überfällig.",
              evidence: ["ev-invite"] },
            { text: "Die Budgetentscheidung wurde um eine Woche verschoben.",
              evidence: ["ev-budget"] },
            { text: "Für den Sommerball werden 120 statt 80 Getränkekarten eingeplant.",
              evidence: ["ev-drinks"] }
          ],
          decisions: [
            { text: "Getränkekontingent auf 120 Karten erhöht.",
              evidence: ["ev-drinks"] }
          ],
          questions: [
            { text: "Ist das Budget für die Support-Werkstudenten bewilligt?",
              evidence: ["ev-budget-q"] }
          ],
          activity: ["Heute, 09:12 · KI-Analyse läuft (3 neue Vorschläge)",
                     "Gestern · Meeting „Wochenplanung“ verarbeitet"]
        },
        sources: [
          { name: "WhatsApp-Export · BDNA Club", kind: "WhatsApp-Export",
            status: "Importiert · indexiert", period: "12.01.–12.08.2026",
            privacy: "Nur lokal", importedOn: "Heute, 09:05" }
        ],
        tasks: [
          { id: "t-inv", title: "Einladungen versenden", status: "offen",
            dueLabel: "Fälligkeit: 12.08. (vor 3 Tagen)", overdueRank: 1,
            dueRank: 1, userPrio: null, aiScore: null, updatedRank: 4,
            evidence: ["ev-invite"],
            whyUrgent: { reason: "Überfällig seit drei Tagen; Lena hat am 2.6. nachgehakt.",
                         evidence: ["ev-invite", "ev-chase"] } },
          { id: "t-zahlen", title: "Zahlen bis Donnerstag prüfen", status: "vorgeschlagen",
            dueLabel: "KI-Vorschlag: bis Donnerstag", overdueRank: 0,
            dueRank: 2, userPrio: null, aiScore: 82, confidence: "87 %",
            updatedRank: 3,
            evidence: ["ev-zahlen"],
            whyUrgent: { reason: "Tarek hat die Prüfung für Donnerstag zugesagt; die Kassenentscheidung hängt daran.",
                         evidence: ["ev-zahlen"] } },
          { id: "t-protokoll", title: "Protokoll an das Team senden", status: "erledigt",
            dueLabel: "—", overdueRank: 0, dueRank: null, userPrio: null,
            aiScore: null, updatedRank: 2,
            evidence: ["ev-budget-q"], whyUrgent: null },
          { id: "t-flyer", title: "Alten Flyer-Entwurf drucken", status: "verworfen",
            dueLabel: "—", overdueRank: 0, dueRank: null, userPrio: null,
            aiScore: null, updatedRank: 1,
            evidence: ["ev-drinks"], whyUrgent: null }
        ]
      },
      { id: nextProjectId(), name: "Messestand 2026",
        statusText: "Ruhend",
        aiLastRun: "Gestern, 18:40",
        lastActiveRank: 3,
        unreviewed: 0,
        overview: {
          status: [
            { text: "Das Messestand-Budget bleibt unverändert.",
              evidence: ["ev-budget"] }
          ],
          decisions: [],
          questions: [],
          activity: ["Gestern, 18:40 · letzter KI-Lauf ohne neue Vorschläge"]
        },
        sources: [
          { name: "Notiz · Budgetgespräch", kind: "Notiz",
            status: "Indexiert", period: "—",
            privacy: "Nur lokal", importedOn: "Gestern, 18:39" }
        ],
        tasks: [
          { id: "t-messe", title: "Angebote für Standbau vergleichen", status: "offen",
            dueLabel: "Fälligkeit: Ende August", overdueRank: 0,
            dueRank: 3, userPrio: "Hoch", aiScore: null, updatedRank: 1,
            evidence: ["ev-budget"], whyUrgent: null }
        ]
      },
      { id: nextProjectId(), name: "Atlas GmbH",
        statusText: "In Arbeit",
        aiLastRun: "Heute, 08:30",
        lastActiveRank: 4,
        unreviewed: 1,
        overview: {
          status: [
            { text: "Für Atlas GmbH wird ein Angebot über zwanzig Lizenzen vorbereitet.",
              evidence: ["ev-license"] }
          ],
          decisions: [],
          questions: [
            { text: "Kommt die Rückmeldung zu den Zahlen bis Donnerstag?",
              evidence: ["ev-berger-d"] }
          ],
          activity: ["Heute, 08:30 · KI-Analyse: 1 neuer Vorschlag",
                     "Montag · Meeting „Kundengespräch Berger“ verarbeitet"]
        },
        sources: [
          { name: "Transkript · Kundengespräch Berger", kind: "Meeting-Transkript",
            status: "Indexiert", period: "—",
            privacy: "Nur lokal", importedOn: "Mo, 11:05" }
        ],
        tasks: [
          { id: "t-angebot", title: "Angebot über zwanzig Lizenzen erstellen", status: "vorgeschlagen",
            dueLabel: "KI-Vorschlag: bis Freitag", overdueRank: 0,
            dueRank: 4, userPrio: null, aiScore: 64, confidence: "81 %",
            updatedRank: 5,
            evidence: ["ev-license"],
            whyUrgent: { reason: "Berger hat das Angebot im Kundengespräch direkt erfragt; die Rückmeldung zu den Zahlen hängt daran.",
                         evidence: ["ev-license", "ev-berger-d"] } }
        ]
      }
    ];
  }

  state.projects = defaultProjects();
  state.selectedProjectId = null;
  state.projectSearch = "";
  state.projectSort = "aktiv";
  state.pendingSuggestionMeetingId = null;

  state.unassignedSources = [];   // Importe ohne Projekt (Eingang)

  function projectById(id) {
    return state.projects.filter(function (p) { return p.id === id; })[0] || null;
  }
  function activeTasks(p) {
    return p.tasks.filter(function (t) {
      return t.status === "offen" || t.status === "vorgeschlagen";
    });
  }
  /* Aufgabenreihenfolge gemäß §6.17: nachvollziehbar, nicht nur KI-Score:
     1. überfällig, 2. frühestes Fälligkeitsdatum, 3. Nutzerpriorität,
     4. KI-Dringlichkeit + Confidence, 5. zuletzt aktualisiert */
  function taskComparator(a, b) {
    if ((b.overdueRank || 0) !== (a.overdueRank || 0)) return (b.overdueRank || 0) - (a.overdueRank || 0);
    var da = a.dueRank == null ? Infinity : a.dueRank;
    var db = b.dueRank == null ? Infinity : b.dueRank;
    if (da !== db) return da - db;
    var pa = a.userPrio ? 1 : 0, pb = b.userPrio ? 1 : 0;
    if (pa !== pb) return pb - pa;
    if ((b.aiScore || 0) !== (a.aiScore || 0)) return (b.aiScore || 0) - (a.aiScore || 0);
    return (b.updatedRank || 0) - (a.updatedRank || 0);
  }

  function updateInboxBadge() {
    var n = state.meetings.filter(function (m) { return !m.project; }).length +
            state.unassignedSources.length;
    var badge = $("#eingang-badge");
    badge.textContent = String(n);
    badge.hidden = n === 0;
  }

  /* ---------- Projekte-Seite (§6.16) ---------- */
  function renderProjects() {
    updateInboxBadge();
    var emptySim = $("#sim-no-projects").checked;
    var projects = emptySim ? [] : state.projects;
    $("#projects-toolbar").hidden = emptySim && projects.length === 0;
    $("#projects-empty").hidden = !(emptySim || projects.length === 0);
    $("#projects-nosearch").hidden = true;

    // Suche über Namen, Status, Aufgaben und normalisierte Quellentexte
    var q = state.projectSearch.trim().toLowerCase();
    var shown = projects.filter(function (p) {
      if (!q) return true;
      var hay = [p.name, p.statusText]
        .concat(p.tasks.map(function (t) { return t.title; }))
        .concat(p.overview.status.map(function (s) { return s.text; }))
        .join(" ")
        .toLowerCase()
        .replace(/\s+/g, " ");   // normalisierte Quellentexte
      return hay.indexOf(q.replace(/\s+/g, " ")) !== -1;
    });
    if (state.projectSort === "name") {
      shown.sort(function (a, b) { return a.name.localeCompare(b.name); });
    } else if (state.projectSort === "dringend") {
      shown.sort(function (a, b) {
        var ta = activeTasks(a).sort(taskComparator)[0];
        var tb = activeTasks(b).sort(taskComparator)[0];
        return taskUrgencyRank(tb) - taskUrgencyRank(ta);
      });
    } else {
      shown.sort(function (a, b) { return b.lastActiveRank - a.lastActiveRank; });
    }

    var list = $("#project-list");
    list.innerHTML = "";

    // Eingang immer zuerst, nie wegsortierbar (§6.16)
    var unassignedCount = state.meetings.filter(function (m) { return !m.project; }).length +
      state.unassignedSources.length;
    var inbox = document.createElement("li");
    inbox.className = "project-card inbox-card";
    inbox.innerHTML =
      "<a href='#/eingang' class='project-link'>" +
      "<strong>Eingang – Nicht zugeordnet</strong>" +
      "<span class='badge badge-accent'>" + unassignedCount + "</span>" +
      "<span class='muted'>" + unassignedCount + (unassignedCount === 1 ? " Element" : " Elemente") +
      " ohne Projektzuordnung</span></a>";
    list.appendChild(inbox);

    shown.forEach(function (p) {
      var top = activeTasks(p).sort(taskComparator)[0];
      var topText = top
        ? "Nächste Aufgabe: " + esc(top.title) +
          (top.overdueRank ? " (überfällig)" :
           top.aiScore ? " (KI-Vorschlag)" : "") +
          " · " + esc(top.dueLabel)
        : "Keine offenen Aufgaben";
      var li = document.createElement("li");
      li.className = "project-card";
      li.innerHTML =
        "<a href='#/projekt-detail' class='project-link' data-project='" + p.id + "'>" +
        "<strong>" + esc(p.name) + "</strong> " +
        "<span class='status-chip " + (p.statusText === "In Arbeit" ? "status-processing" : "status-ready") + "'>" +
        esc(p.statusText) + "</span><br>" +
        "<span class='muted'>" + topText + "</span><br>" +
        "<span class='muted small'>Zuletzt aktiv: " +
        (p.lastActiveRank >= 5 ? "heute" : "vor " + (5 - p.lastActiveRank) + " Tagen") +
        " · " + p.unreviewed + " ungeprüfte Vorschläge</span></a>";
      li.querySelector(".project-link").addEventListener("click", function () {
        state.selectedProjectId = p.id;
      });
      list.appendChild(li);
    });

    if (!shown.length && !emptySim) $("#projects-nosearch").hidden = false;
  }
  function taskUrgencyRank(t) {
    if (!t) return 0;
    return (t.overdueRank || 0) * 100 + (t.aiScore || 0);
  }

  $("#project-search").addEventListener("input", function (e) {
    state.projectSearch = e.target.value; renderProjects();
  });
  $("#project-sort").addEventListener("change", function (e) {
    state.projectSort = e.target.value; renderProjects();
  });
  $("#sim-no-projects").addEventListener("change", renderProjects);

  /* Projekt erstellen: genau ein Pflichtfeld Name (§6.16) */
  function openProjectCreate() {
    $("#project-create-dialog").hidden = false;
    var inp = $("#project-name-input");
    inp.value = "";
    $("#btn-pc-create").disabled = true;
    $("#project-name-blocker").hidden = false;
    inp.focus();
  }
  $("#btn-projekt-erstellen").addEventListener("click", openProjectCreate);
  $("#btn-projekt-erstellen-empty").addEventListener("click", openProjectCreate);
  $("#project-name-input").addEventListener("input", function () {
    $("#btn-pc-create").disabled = this.value.trim().length === 0;
    $("#project-name-blocker").hidden = this.value.trim().length > 0;
  });
  $("#btn-pc-cancel").addEventListener("click", function () {
    $("#project-create-dialog").hidden = true;
  });
  $("#project-create-form").addEventListener("submit", function (e) {
    e.preventDefault();
    var name = $("#project-name-input").value.trim();
    if (!name) return;
    state.projects.push({
      id: nextProjectId(), name: name, statusText: "Neu",
      aiLastRun: "–", lastActiveRank: 9, unreviewed: 0,
      overview: { status: [], decisions: [], questions: [], activity: [] },
      sources: [], tasks: []
    });
    $("#project-create-dialog").hidden = true;
    $("#sim-no-projects").checked = false;
    renderProjects();
    announce("Projekt „" + name + "“ erstellt.");
  });
  $("#btn-import-start").addEventListener("click", function () {
    location.hash = "#/import";
  });
  $("#btn-import-empty").addEventListener("click", function () {
    location.hash = "#/import";
  });

  /* ---------- Projekt-Detailansicht (§6.17) ---------- */
  function selectProjectTab(name) {
    document.querySelectorAll("#view-projekt-detail .tab").forEach(function (t) {
      t.setAttribute("aria-selected", String(t.dataset.ptab === name));
    });
    document.querySelectorAll("#view-projekt-detail .tabpanel").forEach(function (p) {
      p.hidden = p.dataset.ppanel !== name;
    });
  }
  document.querySelectorAll("#view-projekt-detail .tab").forEach(function (tab) {
    tab.addEventListener("click", function () { selectProjectTab(this.dataset.ptab); });
  });

  function renderProjectDetail() {
    var p = projectById(state.selectedProjectId);
    if (!p) { location.hash = "#/projekte"; return; }
    $("#pd-name").textContent = p.name;
    $("#pd-more-menu").open = false;
    // Belegbarer Kurzstatus: jeder Satz mit ≥1 Beleglink (Gate G1)
    var firstStatus = p.overview.status[0];
    $("#pd-statusline").innerHTML =
      esc(p.statusText) +
      (firstStatus ? " · " + esc(firstStatus.text) + " " + evidenceAnchorHtml(firstStatus.evidence[0]) : "");
    $("#pd-ai-last-run").textContent = "Letzter KI-Lauf: " + p.aiLastRun;

    renderProjectOverview(p);
    renderProjectTasks(p);
    renderProjectMeetings(p);
    renderProjectSources(p);
    selectProjectTab("uebersicht");
  }

  function renderProjectOverview(p) {
    var el = $("#pd-overview");
    var html = "<h2 class='small-heading'>Status</h2>";
    if (!p.overview.status.length) html += "<p class='muted'>Noch keine Statusaussagen.</p>";
    p.overview.status.forEach(function (s) {
      html += "<p>" + esc(s.text) + "</p><ul class='evidence-list' role='list'>" +
        s.evidence.map(evidenceLinkHtml).join("") + "</ul>";
    });
    html += "<h2 class='small-heading'>Entscheidungen</h2>";
    if (!p.overview.decisions.length) html += "<p class='muted'>Keine Entscheidungen erkannt.</p>";
    p.overview.decisions.forEach(function (d) {
      html += "<p>" + esc(d.text) + "</p><ul class='evidence-list' role='list'>" +
        d.evidence.map(evidenceLinkHtml).join("") + "</ul>";
    });
    html += "<h2 class='small-heading'>Offene Fragen</h2>";
    if (!p.overview.questions.length) html += "<p class='muted'>Keine offenen Fragen erkannt.</p>";
    p.overview.questions.forEach(function (qq) {
      html += "<p>" + esc(qq.text) + "</p><ul class='evidence-list' role='list'>" +
        qq.evidence.map(evidenceLinkHtml).join("") + "</ul>";
    });
    var urgent = activeTasks(p).sort(taskComparator).slice(0, 2);
    html += "<h2 class='small-heading'>Dringende Aufgaben</h2>";
    html += urgent.length
      ? "<ul role='list'>" + urgent.map(function (t) {
          return "<li>" + esc(t.title) + (t.overdueRank ? " (überfällig)" : "") + "</li>";
        }).join("") + "</ul>"
      : "<p class='muted'>Keine dringenden Aufgaben.</p>";
    html += "<h2 class='small-heading'>Letzte Aktivität</h2><ul role='list'>" +
      p.overview.activity.map(function (a) { return "<li class='muted small'>" + esc(a) + "</li>"; }).join("") +
      "</ul>";
    el.innerHTML = html;
  }

  function taskCardHtml(p, t) {
    var chip = t.status === "offen"
      ? "<span class='status-chip status-processing'>Offen</span>"
      : t.status === "vorgeschlagen"
      ? "<span class='status-chip status-ready'>Vorgeschlagen (KI)</span>"
      : t.status === "erledigt"
      ? "<span class='status-chip status-done'>✓ Erledigt</span>"
      : "<span class='status-chip'>Verworfen</span>";
    var actions = "";
    if (t.status === "vorgeschlagen") {
      actions += "<button type='button' class='btn btn-primary btn-small' data-task-action='annehmen' data-task='" + t.id + "'>Annehmen</button>";
    }
    if (t.status === "offen" || t.status === "vorgeschlagen") {
      actions += " <button type='button' class='btn btn-secondary btn-small' data-task-action='erledigen' data-task='" + t.id + "'>Erledigen</button>" +
        " <button type='button' class='btn btn-ghost btn-small' data-task-action='bearbeiten' data-task='" + t.id + "'>Bearbeiten</button>" +
        " <button type='button' class='btn btn-ghost btn-small' data-task-action='verwerfen' data-task='" + t.id + "'>Verwerfen</button>";
    }
    var why = "";
    if (t.whyUrgent && t.whyUrgent.reason) {
      why = "<div class='why-box' data-why='" + t.id + "' hidden>" +
        "<strong>Warum dringend?</strong> " + esc(t.whyUrgent.reason) +
        "<ul class='evidence-list' role='list'>" +
        t.whyUrgent.evidence.map(evidenceLinkHtml).join("") + "</ul></div>" +
        "<button type='button' class='btn btn-ghost btn-small why-btn' data-why-toggle='" + t.id + "'" +
        " aria-expanded='false'>Warum dringend?</button>";
    }
    var meta = [];
    if (t.dueLabel !== "—") meta.push(esc(t.dueLabel));
    if (t.userPrio) meta.push("Priorität (von dir): " + esc(t.userPrio));
    if (t.aiScore != null) meta.push("KI-Dringlichkeit " + t.aiScore + "/100 · Confidence " + t.confidence);
    return "<div class='task-card' data-task-card='" + t.id + "'>" +
      "<div class='row-between'><strong>" + esc(t.title) + "</strong>" + chip + "</div>" +
      (meta.length ? "<p class='muted small'>" + meta.join(" · ") + "</p>" : "") +
      "<ul class='evidence-list' role='list'>" + t.evidence.map(evidenceLinkHtml).join("") + "</ul>" +
      why +
      "<div class='row-gap task-actions'>" + actions + "</div></div>";
  }

  function renderProjectTasks(p) {
    var host = $("#pd-tasks");
    var groups = [["vorgeschlagen", "Vorgeschlagen"],
                  ["offen", "Offen"],
                  ["erledigt", "Erledigt"],
                  ["verworfen", "Verworfen"]];
    var sorted = p.tasks.slice().sort(taskComparator);
    var html = "<p class='muted small'>Reihenfolge nachvollziehbar: überfällig → früheste Fälligkeit → deine Priorität → KI-Dringlichkeit → zuletzt aktualisiert. Erledigte oder bearbeitete Aufgaben werden bei einer Neuanalyse nie still zurückgesetzt.</p>";
    groups.forEach(function (g) {
      var items = sorted.filter(function (t) { return t.status === g[0]; });
      html += "<section class='task-group'><h2 class='small-heading'>" + g[1] +
        " <span class='badge'>" + items.length + "</span></h2>" +
        (items.length ? items.map(function (t) { return taskCardHtml(p, t); }).join("")
                      : "<p class='muted small'>Keine.</p>") + "</section>";
    });
    host.innerHTML = html;
  }

  /* Aufgaben-Aktionen direkt erreichbar (§6.17): Annehmen/Erledigen/
     Bearbeiten/Verwerfen; „Warum dringend?“ klappt Begründung + Belege auf */
  $("#pd-tasks").addEventListener("click", function (e) {
    var btn = e.target.closest("[data-task-action]");
    if (btn) {
      var p = projectById(state.selectedProjectId);
      var t = p.tasks.filter(function (x) { return x.id === btn.dataset.task; })[0];
      if (!t) return;
      if (btn.dataset.taskAction === "annehmen") {
        t.status = "offen"; t.updatedRank = 99;
        announce("Aufgabe angenommen: " + t.title);
      } else if (btn.dataset.taskAction === "erledigen") {
        t.status = "erledigt"; t.updatedRank = 99;
        announce("Aufgabe erledigt: " + t.title);
      } else if (btn.dataset.taskAction === "verwerfen") {
        t.status = "verworfen"; t.updatedRank = 99;
        announce("Aufgabe verworfen: " + t.title);
      } else if (btn.dataset.taskAction === "bearbeiten") {
        var neu = window.prompt("Aufgabe bearbeiten", t.title);
        if (neu && neu.trim()) { t.title = neu.trim(); t.updatedRank = 99; }
        announce("Aufgabe bearbeitet.");
      }
      renderProjectTasks(p); renderProjectOverview(p);
      return;
    }
    var whyBtn = e.target.closest("[data-why-toggle]");
    if (whyBtn) {
      var box = document.querySelector("[data-why='" + whyBtn.dataset.whyToggle + "']");
      if (box) {
        box.hidden = !box.hidden;
        whyBtn.setAttribute("aria-expanded", String(!box.hidden));
      }
    }
  });

  function renderProjectMeetings(p) {
    var el = $("#pd-meetings");
    var mine = state.meetings.filter(function (m) { return m.project === p.name; });
    // Dieselben Datensätze wie in der globalen Bibliothek – keine Kopien:
    // Änderungen im Detail wirken sofort auch hier.
    el.innerHTML = "<p class='muted small'>Dieselben Meeting-Datensätze wie in der globalen Bibliothek, nur gefiltert – keine duplizierten Kopien.</p>" +
      (mine.length
        ? "<ul role='list'>" + mine.map(function (m) {
            return "<li class='meeting-item'><strong>" + esc(m.title) + "</strong> · " +
              esc(m.date) + " · " + esc(m.duration) + " " + statusChipHtml(m) + "</li>";
          }).join("") + "</ul>"
        : "<p class='muted'>Noch keine Meetings diesem Projekt zugeordnet.</p>");
  }

  function renderProjectSources(p) {
    var el = $("#pd-sources");
    el.innerHTML = "<p class='muted small'>WhatsApp-Exporte, Dateien und Notizen mit Verarbeitungsstatus, Zeitraum, Datenschutzmodus und Importdatum.</p>" +
      (p.sources.length
        ? "<table class='sources-table'><thead><tr><th>Quelle</th><th>Status</th><th>Zeitraum</th><th>Datenschutz</th><th>Importiert</th></tr></thead><tbody>" +
          p.sources.map(function (s) {
            return "<tr><td>" + esc(s.name) + "<br><span class='muted small'>" + esc(s.kind) +
              "</span></td><td>" + esc(s.status) + "</td><td>" + esc(s.period) +
              "</td><td>" + esc(s.privacy) + "</td><td>" + esc(s.importedOn) + "</td></tr>";
          }).join("") + "</tbody></table>"
        : "<p class='muted'>Noch keine Quellen importiert.</p>");
  }

  $("#btn-pd-add-source").addEventListener("click", function () {
    location.hash = "#/import";
  });
  $("#btn-pd-rename").addEventListener("click", function () {
    var p = projectById(state.selectedProjectId);
    if (!p) return;
    var neu = window.prompt("Projekt umbenennen", p.name);
    if (neu && neu.trim()) {
      var alt = p.name;
      p.name = neu.trim();
      // Nur die Verknüpfung ändern – Meetings behalten Transkript/Audio/Belege
      state.meetings.forEach(function (m) { if (m.project === alt) m.project = p.name; });
      $("#pd-more-menu").open = false;
      renderProjectDetail(); renderMeetings();
      announce("Projekt umbenannt in „" + p.name + "“.");
    }
  });
  $("#btn-pd-delete").addEventListener("click", function () {
    var p = projectById(state.selectedProjectId);
    if (!p) return;
    if (window.confirm("Projekt „" + p.name + "“ löschen?\n\nMeetings, Transkripte und Audio bleiben erhalten und landen im Eingang „Nicht zugeordnet“. Nur Verknüpfungen, Aufgaben und Projektquellen werden entfernt.")) {
      state.meetings.forEach(function (m) { if (m.project === p.name) m.project = null; });
      state.projects = state.projects.filter(function (x) { return x !== p; });
      location.hash = "#/projekte";
      announce("Projekt gelöscht. Zugeordnete Meetings liegen im Eingang.");
    }
    $("#pd-more-menu").open = false;
  });

  /* ---------- Eingang – Nicht zugeordnet (§6.16/§6.18) ---------- */
  function renderEingang() {
    updateInboxBadge();
    var list = $("#eingang-list");
    list.innerHTML = "";
    var items = state.meetings.filter(function (m) { return !m.project; })
      .map(function (m) { return { kind: "Meeting", label: m.title + " (" + m.date + " · " + m.duration + ")", ref: m }; })
      .concat(state.unassignedSources.map(function (s) {
        return { kind: "Import", label: s.label, ref: s };
      }));
    if (!items.length) {
      list.innerHTML = "<li class='muted'>Der Eingang ist leer – alles ist zugeordnet.</li>";
    }
    items.forEach(function (it, idx) {
      var li = document.createElement("li");
      li.className = "inbox-item";
      li.innerHTML = "<label><input type='checkbox' data-inbox-idx='" + idx + "'> " +
        "<span class='badge'>" + esc(it.kind) + "</span> " + esc(it.label) + "</label>";
      li.querySelector("input")._ref = it.ref;
      li.querySelector("input")._kind = it.kind;
      list.appendChild(li);
    });
    // Zielprojekt-Auswahl frisch befüllen
    var sel = $("#eingang-target");
    sel.innerHTML = "";
    state.projects.forEach(function (p) {
      var o = document.createElement("option");
      o.value = p.id; o.textContent = p.name;
      sel.appendChild(o);
    });
  }

  $("#btn-eingang-assign").addEventListener("click", function () {
    var checked = [...document.querySelectorAll("#eingang-list input:checked")];
    if (!checked.length) { announce("Erst Elemente auswählen."); return; }
    var target = projectById($("#eingang-target").value);
    if (!target) { announce("Kein Zielprojekt vorhanden – erst ein Projekt erstellen."); return; }
    // Vorschau vor jeder Stapelzuordnung (§6.18)
    $("#assign-preview-text").textContent =
      checked.length + (checked.length === 1 ? " Element" : " Elemente") +
      " dem Projekt „" + target.name + "“ zuweisen?";
    $("#assign-preview-list").innerHTML = checked.map(function (inp) {
      return "<li>" + esc(inp._kind) + ": " + esc(inp._ref.title || inp._ref.label) + "</li>";
    }).join("");
    $("#assign-preview").hidden = false;
    $("#btn-assign-ok").onclick = function () {
      checked.forEach(function (inp) {
        if (inp._kind === "Meeting") inp._ref.project = target.name;
        else {
          state.unassignedSources = state.unassignedSources.filter(function (s) { return s !== inp._ref; });
          target.sources.push({ name: inp._ref.label, kind: inp._ref.kind,
            status: "Importiert · indexiert", period: inp._ref.period,
            privacy: "Nur lokal", importedOn: "Heute, 09:05" });
        }
      });
      $("#assign-preview").hidden = true;
      renderEingang();
      announce(checked.length + " Element(e) „" + target.name + "“ zugeordnet. Nur Verknüpfungen geändert.");
    };
  });
  $("#btn-assign-cancel").addEventListener("click", function () {
    $("#assign-preview").hidden = true;
    announce("Zuordnung abgebrochen – nichts wurde verschoben.");
  });

  /* ---------- KI-Projektvorschlag (§6.18, Gate G2) ---------- */
  function showProjectSuggestion(m) {
    state.pendingSuggestionMeetingId = m.id;
    var card = $("#ai-suggestion");
    $("#suggestion-headline").innerHTML =
      "Für <strong>„" + esc(m.title) + "“</strong> wird <strong>Projekt „BDNA Club“ vorgeschlagen</strong>" +
      " · Confidence 87 %";
    $("#suggestion-evidence").innerHTML =
      evidenceLinkHtml("ev-invite") + evidenceLinkHtml("ev-chase");
    $("#suggest-other-picker").hidden = true;
    $("#suggestion-note").textContent =
      "Ohne deine Wahl wird nichts verschoben.";
    card.hidden = false;
    announce("Projektvorschlag: BDNA Club. Bitte zuordnen oder ablehnen.");
  }

  function closeSuggestion(text) {
    $("#ai-suggestion").hidden = true;
    state.pendingSuggestionMeetingId = null;
    $("#suggestion-note").textContent = text || "";
    $("#suggestion-note").hidden = false;
  }

  $("#btn-suggest-zuordnen").addEventListener("click", function () {
    var m = meetingById(state.pendingSuggestionMeetingId);
    if (!m) return;
    m.project = "BDNA Club";
    closeSuggestion("„" + m.title + "“ wurde „BDNA Club“ zugeordnet. Nur die Verknüpfung wurde geändert.");
    announce("Meeting zugeordnet: BDNA Club.");
    updateInboxBadge();
  });
  $("#btn-suggest-other").addEventListener("click", function () {
    var host = $("#suggest-other-picker");
    host.innerHTML = "";
    var head = document.createElement("div");
    head.className = "picker-head";
    head.innerHTML = "<h3>Anderes Projekt wählen</h3>";
    host.appendChild(head);
    state.projects.forEach(function (p) {
      var b = document.createElement("button");
      b.type = "button";
      b.className = "picker-option";
      b.textContent = p.name;
      b.addEventListener("click", function () {
        var m = meetingById(state.pendingSuggestionMeetingId);
        if (!m) return;
        m.project = p.name;
        closeSuggestion("„" + m.title + "“ wurde „" + p.name + "“ zugeordnet.");
        announce("Meeting zugeordnet: " + p.name);
        updateInboxBadge();
      });
      host.appendChild(b);
    });
    host.hidden = false;
  });
  $("#btn-suggest-none").addEventListener("click", function () {
    var m = meetingById(state.pendingSuggestionMeetingId);
    closeSuggestion(m ? "„" + m.title + "“ bleibt ohne Projekt – es liegt im Eingang." : "");
    announce("Meeting bleibt ohne Projektzuordnung.");
  });

  /* ---------- WhatsApp-Importassistent (§6.19) ---------- */
  var IMPORT_FILES = {
    a: { name: "WhatsApp Export – BDNA Club.zip",
         chat: "BDNA Club", participants: 4, period: "12.01.–12.08.2026",
         messages: 48, media: 6, voice: 2, video: 1, unsupported: 1,
         warnings: ["1 Sprachnachricht ohne Transkript-Datei",
                    "1 nicht unterstützte Datei (sticker.webp)"],
         duplicates: [], newMessages: 48 },
    b: { name: "BDNA Club Chat (Teil 2).zip",
         chat: "BDNA Club", participants: 4, period: "01.06.–12.08.2026",
         messages: 26, media: 3, voice: 1, video: 0, unsupported: 0,
         warnings: ["Export überlappt mit bereits importiertem Archiv"],
         // Dubletten-Demo (Gate G4): 12 Nachrichten sind bereits vorhanden
         duplicates: ["Können wir endlich die Einladungen für den Sommerball rausschicken?",
                      "Ja – ich mache die Einladungen diese Woche fertig.",
                      "Erinnerung: Die Einladungen sind immer noch nicht raus.",
                      "Sorry, diese Woche klappt es nicht, ich verschiebe auf nächste Woche.",
                      "Sollen wir für den Sommerball jetzt 80 oder 120 Getränkekarten planen?",
                      "Nehmen wir 120, beim Sommerfest waren alle Getränke nach zwei Stunden weg.",
                      "Ich schicke heute die Zahlen für die Kasse an alle, bitte bis Donnerstag prüfen.",
                      "Wer holt eigentlich die Kisten vom Lager?",
                      "Ich kann Freitag gegen 16 Uhr.",
                      "Dann treffen wir uns um 15:30 am Lager.",
                      "Bringt bitte noch die Restposten-Coupons mit.",
                      "Und nicht vergessen: Kassenzettel für die Abrechnung aufheben."],
         newMessages: 14 }
  };
  var importState = null;   // aktuell gewählte Datei

  function setWizardStep(n) {
    document.querySelectorAll("#wizard-steps li").forEach(function (li) {
      var cur = Number(li.dataset.wstep) === n;
      li.classList.toggle("current", cur);
      if (cur) li.setAttribute("aria-current", "step");
      else li.removeAttribute("aria-current");
    });
    [1, 2, 3, 4].forEach(function (i) {
      $("#import-step-" + i).hidden = i !== n;
    });
  }
  function initImportWizard() {
    importState = null;
    $("#dedup-preview").hidden = true;
    $("#cloud-disclosure").hidden = true;
    $("#local-disclosure").hidden = false;
    $("#import-checklist").innerHTML = "";
    $("#import-result").hidden = true;
    $("#btn-import-run").hidden = false;
    $("#btn-import-finish").hidden = true;
    $("#btn-import-back3").hidden = true;
    var radio = document.querySelector('input[name="import-mode"][value="lokal"]');
    if (radio) radio.checked = true;
    fillImportProjectSelect();
    setWizardStep(1);
  }
  function fillImportProjectSelect(preselectId) {
    var sel = $("#import-project");
    sel.innerHTML = "";
    var none = document.createElement("option");
    none.value = ""; none.textContent = "Nicht zugeordnet (Eingang)";
    sel.appendChild(none);
    state.projects.forEach(function (p) {
      var o = document.createElement("option");
      o.value = p.id; o.textContent = p.name;
      sel.appendChild(o);
    });
    // Optionen tragen die Projekt-IDs – Vorauswahl daher per ID, nicht Name
    if (preselectId && projectById(preselectId)) sel.value = preselectId;
  }
  function projectByName(name) {
    return state.projects.filter(function (p) { return p.name === name; })[0] || null;
  }

  document.querySelectorAll(".file-pick").forEach(function (b) {
    b.addEventListener("click", function () {
      importState = IMPORT_FILES[b.id === "pick-file-a" ? "a" : "b"];
      renderDetected();
      setWizardStep(2);
      announce("Datei analysiert: " + importState.name);
    });
  });
  $("#btn-import-back1").addEventListener("click", function () { setWizardStep(1); });
  $("#btn-to-step3").addEventListener("click", function () {
    fillImportProjectSelect(state.selectedProjectId);
    setWizardStep(3);
  });
  $("#btn-import-back2").addEventListener("click", function () { setWizardStep(2); });

  function renderDetected() {
    var f = importState;
    $("#detected-grid").innerHTML =
      "<dt>Chatname</dt><dd>" + esc(f.chat) + "</dd>" +
      "<dt>Teilnehmer</dt><dd>" + f.participants + " (Lena, Tarek, Mira, Du)</dd>" +
      "<dt>Zeitraum</dt><dd>" + esc(f.period) + "</dd>" +
      "<dt>Nachrichten</dt><dd>" + f.messages + "</dd>" +
      "<dt>Medien</dt><dd>" + f.media + " Bilder · " + f.voice + " Sprachnachrichten · " +
      f.video + " Video</dd>" +
      "<dt>Warnungen</dt><dd>" + f.warnings.map(esc).join("<br>") + "</dd>" +
      "<dt>Mögliche Dubletten</dt><dd id='detected-dup-count'>" + f.duplicates.length + "</dd>";

    // Deduplizieren passiert lokal VOR jedem Import; die Vorschau zeigt
    // genau, welche Nachrichten übersprungen würden (Gate G4)
    var dp = $("#dedup-preview");
    if (f.duplicates.length) {
      dp.innerHTML = "<strong>Dubletten-Vorschau (lokal dedupliziert):</strong> " +
        f.duplicates.length + " Nachrichten existieren bereits im Projekt „BDNA Club“ " +
        "und werden beim Import übersprungen:" +
        "<ul class='dup-list' role='list'>" +
        f.duplicates.map(function (d) {
          return "<li class='dup-item'>⤵ schon vorhanden · „" + esc(d) + "“</li>";
        }).join("") + "</ul>" +
        "<p class='muted small'>Inhalts-Hashes machen erneute Importe idempotent – nichts wird doppelt gespeichert.</p>";
      dp.hidden = false;
    } else {
      dp.hidden = true;
    }
  }

  /* Schritt 3: exakte Offenlegung, was das Gerät verlässt (Gate G3) */
  function renderCloudDisclosure() {
    var cloud = document.querySelector('input[name="import-mode"]:checked').value === "cloud";
    $("#cloud-disclosure").hidden = !cloud;
    $("#local-disclosure").hidden = cloud;
    if (cloud) {
      var f = importState;
      $("#cloud-leaves-list").innerHTML =
        "<li>Nachrichtentexte: alle " + f.messages + " erkannten Nachrichten des Exports</li>" +
        "<li>Bilder: " + f.media + " Bilder als Base64-Dateninhalt – nur nach Bestätigung gesendet</li>" +
        "<li>Sprachnachrichten: " + f.voice + " Audiodatei(en) – nur wenn du unten explizit „Sprache transkribieren (Cloud)“ wählst, sonst nur lokales Asset</li>" +
        "<li>PDF/Dokumente: " + 0 + " – kein Dokument im Export</li>" +
        "<li><strong>Nicht</strong>: Video (" + f.video + ") und nicht unterstützte Dateien (" + f.unsupported + ") bleiben vollständig auf dem Gerät</li>";
      $("#cloud-cost").textContent =
        "Kostenschätzung: ca. 0,04 € (ca. 21.000 Text-Token · Mock-Betrag).";
    }
  }
  document.querySelectorAll('input[name="import-mode"]').forEach(function (r) {
    r.addEventListener("change", renderCloudDisclosure);
  });

  $("#btn-to-step4").addEventListener("click", function () {
    renderCloudDisclosure();
    setWizardStep(4);
    $("#import-checklist").innerHTML =
      "<li data-istep='parse' class='done'>✓ Lokal entpackt und geparst</li>" +
      "<li data-istep='dedup' class='done'>✓ Lokal dedupliziert (" + importState.duplicates.length + " Dubletten erkannt)</li>" +
      "<li data-istep='index' class='done'>✓ Lokal indexiert – Export ist durchsuchbar</li>" +
      "<li data-istep='cloud' hidden></li>";
    $("#import-result").hidden = true;
    $("#btn-import-run").hidden = false;
    $("#btn-import-finish").hidden = true;
    $("#btn-import-back3").hidden = false;
  });
  $("#btn-import-back3").addEventListener("click", function () { setWizardStep(3); });

  $("#btn-import-run").addEventListener("click", function () {
    var f = importState;
    var cloud = document.querySelector('input[name="import-mode"]:checked').value === "cloud";
    this.disabled = true;
    this.textContent = "Import läuft …";
    var steps = $("#import-checklist");
    var markDone = function (sel) {
      var li = steps.querySelector(sel);
      if (li) li.className = "done";
    };
    var cloudLi = steps.querySelector('[data-istep="cloud"]');
    if (cloud) {
      cloudLi.hidden = false;
      cloudLi.className = "";
      cloudLi.textContent = "● Cloud-Analyse der freigegebenen Inhalte …";
    }
    window.setTimeout(function () {
      if (cloud) markDone('[data-istep="cloud"]');
      // Ergebnis: importierte / übersprungene / fehlerhafte Elemente (§6.19)
      var skipped = f.duplicates.length;
      var failed = f.unsupported;
      var res = $("#import-result");
      res.innerHTML = "<strong>Import abgeschlossen.</strong>" +
        "<ul role='list'>" +
        "<li>Importiert: " + f.newMessages + " Nachrichten, " + (f.media - failed) + " Medien</li>" +
        "<li>Übersprungen (Dubletten): " + skipped + "</li>" +
        "<li>Fehlerhaft/nicht analysiert: " + failed + " (klar als „nicht analysiert“ markiert)</li>" +
        "</ul>" +
        "<p class='muted small'>Gesamt im Projekt: " + (48 + f.newMessages) +
        " Nachrichten – die " + skipped + " Dubletten wurden NICHT doppelt gespeichert.</p>";
      res.hidden = false;
      // Quelle dem gewählten Zielprojekt zuordnen (oder Eingang lassen)
      var projSel = $("#import-project").value;
      var target = projSel ? projectById(projSel) : null;
      var entry = { name: f.name, kind: "WhatsApp-Export",
        status: "Importiert · indexiert", period: f.period,
        privacy: cloud ? "Teils Cloud (bestätigte Inhalte)" : "Nur lokal",
        importedOn: "Gerade eben" };
      if (target) target.sources.push(entry);
      else state.unassignedSources.push({ label: f.name, kind: "WhatsApp-Export", period: f.period });
      updateInboxBadge();
      $("#btn-import-run").hidden = true;
      $("#btn-import-finish").hidden = false;
      $("#btn-import-run").disabled = false;
      $("#btn-import-run").textContent = "Import starten";
      announce("Import abgeschlossen. " + f.newMessages + " importiert, " + skipped + " übersprungen.");
    }, cloud ? 900 : 400);
  });
  $("#btn-import-finish").addEventListener("click", function () {
    location.hash = "#/projekte";
  });


  $("#mock-reset").addEventListener("click", function () {
    state.sessionActive = false; state.paused = false;
    state.seconds = 0;
    stopSegments();
    clearProcTimers();          // laufende Checklisten anhalten (§6.12)
    sessionCaptured = [];
    state.selectedId = null;
    $("#btn-retry-failed").hidden = true;
    $("#btn-open-processing-meeting").hidden = true;
    // offene Overlays (Countdown/Bestätigung) sauber schließen
    if (countdownIv) { window.clearInterval(countdownIv); countdownIv = null; }
    $("#countdown-overlay").hidden = true;
    closeStopConfirm();
    resetDialogState();
    dlgInitialized = true;   // Defaults sind gerade explizit hergestellt
    // Projekte/Eingang/Vorschlag/Import auf die Mock-Defaults zurücksetzen
    state.projects = defaultProjects();
    state.selectedProjectId = null;
    state.projectSearch = "";
    $("#project-search").value = "";
    state.projectSort = "aktiv";
    $("#project-sort").value = "aktiv";
    $("#sim-no-projects").checked = false;
    state.unassignedSources = [];
    state.pendingSuggestionMeetingId = null;
    $("#ai-suggestion").hidden = true;
    $("#source-viewer").hidden = true;
    $("#assign-preview").hidden = true;
    $("#project-create-dialog").hidden = true;
    importState = null;
    ["#sim-no-mic", "#sim-mic-denied", "#sim-no-output", "#sim-shortcut-taken",
     "#sim-chunk-error"]
      .forEach(function (sel) { $(sel).checked = false; });
    location.hash = "#/diktieren";
    render();
    announce("Prototyp-Zustand zurückgesetzt.");
  });

  /* ---------- Start ---------- */
  render();
})();
