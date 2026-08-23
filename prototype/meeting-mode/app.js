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
                "projekte", "diktatverlauf", "einstellungen", "hilfe",
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

  /* ---------- Mock zurücksetzen ---------- */
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
