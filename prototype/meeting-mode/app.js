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
  var state = {
    sessionActive: false,      // läuft gerade ein Meeting? (steuert Pin in der Navigation)
    paused: false,
    seconds: 0,                // gemockte Aufnahmedauer, pro Tick inkrementiert
    meetings: [
      { title: "Wochenplanung", date: "Gestern, 16:00", duration: "00:58:12",
        provider: "Lokal", status: "fertig",
        summary: "Team einigte sich auf Release-Termin und zwei Follow-up-Aufgaben." },
      { title: "Kundengespräch Berger", date: "Mo, 10:30", duration: "00:31:40",
        provider: "Lokal", status: "verarbeitung",
        summary: "Transkription läuft …" }
    ],
    filter: "alle",
    search: "",
    activeTitle: ""
  };

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
                "meeting-einrichtung"];

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
    tick();
  }

  window.addEventListener("hashchange", render);

  /* ---------- Meetings-Ansicht ---------- */
  function meetingMatches(m) {
    if (state.filter !== "alle" && m.status !== state.filter) return false;
    if (!state.search) return true;
    return (m.title + " " + m.summary).toLowerCase().indexOf(state.search.toLowerCase()) !== -1;
  }

  function renderMeetings() {
    var list = $("#meeting-list");
    list.innerHTML = "";
    var shown = state.meetings.filter(meetingMatches);
    $("#meetings-empty").hidden = state.sessionActive || state.meetings.length > 0;

    shown.forEach(function (m, i) {
      var li = document.createElement("li");
      li.className = "meeting-item";
      var chip = m.status === "fertig"
        ? '<span class="status-chip status-ready">✓ Fertig</span>'
        : '<span class="status-chip status-processing">⟳ In Verarbeitung</span>';
      li.innerHTML =
        "<div class='row-between'>" +
        "<div><strong>" + m.title + "</strong> · <span class='muted'>" + m.date +
        "</span> · " + m.duration + " · " + m.provider + "</div>" +
        "<div class='menu-row'><button type='button' class='btn btn-ghost btn-small menu-btn' " +
        "aria-haspopup='true' aria-expanded='false' aria-label='Aktionen für " + m.title + "'>⋯ Menü</button></div></div>" +
        chip + "<p class='muted small'>" + m.summary + "</p>";
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
      b.textContent = item[0];
      if (item[1]) b.className = item[1];
      if (item[0].indexOf("Löschen") === 0) {
        b.addEventListener("click", function () {
          // Irreversible Löschung nennt konkret, was entfernt wird (§6.14)
          if (window.confirm("„" + m.title + "“ endgültig löschen?\n\nEs werden Audio, Transkript und Zusammenfassung unwiderruflich entfernt.")) {
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
    document.querySelectorAll(".menu").forEach(function (el) { el.remove(); });
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
      : (!startDialog.hidden ? startDialog.querySelector(".modal") : null);
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
    state.sessionActive = true;
    state.paused = false;
    state.seconds = 0;
    window.closeStartDialog();
    announce("Aufnahme läuft. " + state.activeTitle);
    startSegments();
    $("#btn-zum-text").hidden = true;
    location.hash = "#/meeting-aktiv";
  });

  /* ---------- Laufende Ansicht (§6.11) ---------- */
  var demoSegments = [
    [2, "Wir starten mit dem Tagesordnungspunkt Budget."],
    [6, "Ich schlage vor, das Marketingbudget um zehn Prozent zu erhöhen."],
    [11, "Einwand: Der Messestand braucht dieses Jahr mehr Mittel."],
    [16, "Entscheidung: Wir verschieben die Entscheidung um eine Woche."]
  ];
  var segmentTimers = [];

  function startSegments() {
    stopSegments();
    var ul = $("#live-segments");
    ul.innerHTML = "";
    demoSegments.forEach(function (seg, i) {
      segmentTimers.push(window.setTimeout(function () {
        if (!state.sessionActive || state.paused) return;
        var li = document.createElement("li");
        li.innerHTML = "<span class='segment-time'>+" + seg[0] * 10 + "s</span>" + seg[1];
        ul.appendChild(li);
        // Nur am unteren Rand mitlaufen; sonst bleibt die Ansicht stehen (§6.11)
        if (typeof stickToBottom !== "undefined" && stickToBottom) {
          transcriptArea.scrollTop = transcriptArea.scrollHeight;
        }
        $("#listening-state").textContent =
          i === demoSegments.length - 1 ? "Abschnitt wird transkribiert …" : "Hört zu …";
      }, seg[0] * 1500)); // Demo-Zeitlupe
    });
  }
  function stopSegments() {
    segmentTimers.forEach(function (t) { window.clearTimeout(t); });
    segmentTimers = [];
  }

  $("#btn-open-active").addEventListener("click", function () {
    location.hash = "#/meeting-aktiv";
  });

  /* Kein automatisches Springen, solange der Nutzer nach oben gescrollt hat;
     stattdessen erscheint der Button „Zum aktuellen Text“ (Plan §6.11). */
  var transcriptArea = $(".transcript-area");
  var stickToBottom = true;
  transcriptArea.addEventListener("scroll", function () {
    var nearBottom = transcriptArea.scrollHeight - transcriptArea.clientHeight -
      transcriptArea.scrollTop < 24;
    stickToBottom = nearBottom;
    $("#btn-zum-text").hidden = nearBottom || !state.sessionActive;
  });
  $("#btn-zum-text").addEventListener("click", function () {
    transcriptArea.scrollTop = transcriptArea.scrollHeight;
    this.hidden = true;
    announce("Zum aktuellen Text gesprungen.");
  });

  $("#btn-pause").addEventListener("click", function () {
    state.paused = !state.paused;
    if (state.paused) {
      this.textContent = "Fortsetzen";
      $("#live-region").textContent = "Pausiert.";
      $("#listening-state").textContent = "Pausiert.";
    } else {
      this.textContent = "Pause";
      announce("Aufnahme fortgesetzt.");
    }
  });

  $("#btn-beenden").addEventListener("click", function () {
    $("#stop-confirm").hidden = false;
    $("#btn-stop-abbruch").focus();
  });
  $("#btn-stop-abbruch").addEventListener("click", function () {
    $("#stop-confirm").hidden = true;
    $("#btn-beenden").focus();
  });
  $("#btn-stop-ok").addEventListener("click", function () {
    $("#stop-confirm").hidden = true;
    var dur = fmt(elapsedSec());
    state.sessionActive = false;
    state.paused = false;
    state.seconds = 0;
    state.meetings.unshift({
      title: state.activeTitle, date: "Gerade eben", duration: dur,
      provider: "Lokal", status: "verarbeitung",
      summary: "Verarbeitung gestartet: Transkription läuft …"
    });
    stopSegments();
    announce("Aufnahme beendet. Audio sicher gespeichert.");
    location.hash = "#/verarbeitung";
  });
  document.addEventListener("keydown", function (e) {
    if (e.key === "Escape" && !$("#stop-confirm").hidden) {
      $("#stop-confirm").hidden = true;
      $("#btn-beenden").focus();
    }
  });

  /* ---------- Verarbeitungs-Checkliste (§6.12) ---------- */
  function runChecklist() {
    var steps = document.querySelectorAll("#proc-checklist li");
    steps.forEach(function (li) { li.className = ""; });
    // Schrittexte zurücksetzen, damit ein erneuter Lauf sauber startet
    steps[1].textContent = "○ Audioabschnitte vorbereiten …";
    steps[2].textContent = "○ Abschnitte transkribieren …";
    steps[3].textContent = "○ Zusammenfassung wartet";
    var seq = [
      [400, function () { steps[1].textContent = "● 3 von 3 Audioabschnitten vorbereitet"; steps[1].className = "done"; }],
      [1200, function () { steps[2].textContent = "● 2 von 3 Abschnitten transkribiert"; }],
      [2200, function () { steps[2].textContent = "✓ 3 von 3 Abschnitten transkribiert"; steps[2].className = "done"; }],
      [3200, function () {
        steps[3].textContent = "✓ Zusammenfassung erstellt";
        steps[3].className = "done";
        announce("Verarbeitung abgeschlossen.");
      }]
    ];
    seq.forEach(function (item) { window.setTimeout(item[1], item[0]); });
  }
  window.addEventListener("hashchange", function () {
    if (currentRoute() === "verarbeitung") runChecklist();
  });

  /* ---------- Uhr / Tick ---------- */
  function tick() {
    if (state.sessionActive && !state.paused) state.seconds += 1;
    var t = fmt(elapsedSec());
    $("#pinned-time").textContent = t;
    $("#am-duration").textContent = t;
    $("#live-duration").textContent = t;
  }
  window.setInterval(tick, 1000);

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
    resetDialogState();
    dlgInitialized = true;   // Defaults sind gerade explizit hergestellt
    ["#sim-no-mic", "#sim-mic-denied", "#sim-no-output", "#sim-shortcut-taken"]
      .forEach(function (sel) { $(sel).checked = false; });
    location.hash = "#/diktieren";
    render();
    announce("Prototyp-Zustand zurückgesetzt.");
  });

  /* ---------- Start ---------- */
  render();
})();
