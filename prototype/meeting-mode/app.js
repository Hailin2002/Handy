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
                "projekte", "diktatverlauf", "einstellungen", "hilfe"];

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

  function openStartDialog() {
    // Nur ein Meeting gleichzeitig (Plan §6.5: aktives Meeting steht immer oben)
    if (state.sessionActive) {
      announce("Es läuft bereits ein Meeting.");
      location.hash = "#/meeting-aktiv";
      return;
    }
    lastFocus = document.activeElement;
    startDialog.hidden = false;
    var ts = new Date();
    $("#meeting-title-input").value = ts.toLocaleDateString("de-DE") + " " +
      ts.toLocaleTimeString("de-DE", { hour: "2-digit", minute: "2-digit" }) + " Uhr";
    $("#meeting-title-input").focus();
  }
  window.closeStartDialog = function () {
    startDialog.hidden = true;
    if (lastFocus) lastFocus.focus();
  };
  ["btn-neues-meeting", "btn-neues-meeting-empty"].forEach(function (id) {
    $("#" + id).addEventListener("click", openStartDialog);
  });
  startDialog.addEventListener("click", function (e) {
    if (e.target === startDialog) window.closeStartDialog();
  });
  document.addEventListener("keydown", function (e) {
    if (e.key === "Escape" && !startDialog.hidden) window.closeStartDialog();
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

  $("#project-chip").addEventListener("click", function () {
    var next = this.classList.contains("chip-selected") ? "Kein Projekt" : "BDNA Club";
    this.textContent = next;
    this.classList.toggle("chip-selected", next !== "Kein Projekt");
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
    location.hash = "#/diktieren";
    render();
    announce("Prototyp-Zustand zurückgesetzt.");
  });

  /* ---------- Start ---------- */
  render();
})();
