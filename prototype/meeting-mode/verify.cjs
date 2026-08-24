// Verification harness for prototype/meeting-mode.
// Covers the task-0179 leaf gates (G1 stop confirmation, G2 scroll lock,
// G3 §6.8 colour semantics, G4 flows from start-dialog completion),
// regressions for task-0178 (§6.6 setup, §6.7 start dialog) and task-0180
// (§6.12 checklist, §6.5 list, §6.13 detail), plus the task-0181 leaf gates:
//   G1 – every AI status statement / task carries ≥1 evidence link that jumps
//        to a highlighted mocked source line (§6.16–§6.17)
//   G2 – AI project suggestion requires an explicit Zuordnen / Anderes
//        Projekt / Ohne Projekt choice, never auto-moves (§6.18)
//   G3 – import step 3 lists exactly which content would leave the device,
//        with cost estimate; local is the default (§6.19)
//   G4 – repeated-import demo shows a per-message dedup preview and imports
//        without duplicating messages (§6.19)
// Run from workspace root:  node prototype\meeting-mode\verify.cjs
// Uses the repo's Playwright package with the locally installed Chromium build.
"use strict";
const path = require("path");
const fs = require("fs");
const { chromium } = require(path.join(__dirname, "..", "..", "node_modules", "playwright"));

const exeRoot = path.join(process.env.LOCALAPPDATA, "ms-playwright");
// Discover whatever build is actually installed instead of hardcoding a
// version: concurrent `playwright install` runs prune old build dirs.
function findExe() {
  if (!fs.existsSync(exeRoot)) return null;
  const dirs = fs.readdirSync(exeRoot)
    .filter((d) => /^(chromium|chromium_headless_shell)-/.test(d))
    .sort()
    .reverse(); // newest build number first
  const inner = [
    ["chrome-headless-shell-win64", "chrome-headless-shell.exe"],
    ["chrome-win64", "chrome.exe"],
  ];
  for (const d of dirs) {
    for (const [sub, exe] of inner) {
      const p = path.join(exeRoot, d, sub, exe);
      if (fs.existsSync(p) && fs.statSync(p).isFile()) return p;
    }
  }
  return null;
}
const exe = findExe();
if (!exe) { console.error("No Chromium found under", exeRoot); process.exit(1); }
console.log("Using browser:", exe);

const url = "file:///" + path.join(__dirname, "index.html").replace(/\\/g, "/");

// Launch helper. Prefers the normal Playwright pipe transport; falls back to
// launching Chrome with a TCP debug port and attaching via CDP over WebSocket,
// which works in environments where named pipes are unavailable.
// VERIFY_CDP_URL=http://127.0.0.1:9333 attaches to an already-running Chrome
// started externally (e.g. by pwsh) and leaves it running for the caller.
async function launchBrowser() {
  if (process.env.VERIFY_CDP_URL) {
    const browser = await chromium.connectOverCDP(process.env.VERIFY_CDP_URL);
    return { browser, cleanup: async () => {} };
  }
  try {
    return { browser: await chromium.launch({ executablePath: exe }), cleanup: async () => {} };
  } catch (e) {
    if (!/EPERM/.test(String(e))) throw e;
    console.log("pipe transport blocked (EPERM); falling back to CDP over TCP");
  }
  const { spawn } = require("child_process");
  const os = require("os");
  const profile = fs.mkdtempSync(path.join(os.tmpdir(), "verify-profile-"));
  const port = 9223 + Math.floor(Math.random() * 500);
  const child = spawn(exe, [
    "--headless", "--no-sandbox", "--no-first-run", "--mute-audio",
    "--remote-debugging-port=" + port, "--user-data-dir=" + profile,
    "about:blank",
  ], { stdio: "ignore" });
  const deadline = Date.now() + 20000;
  while (Date.now() < deadline) {
    try {
      const res = await fetch("http://127.0.0.1:" + port + "/json/version");
      if (res.ok) {
        const browser = await chromium.connectOverCDP("http://127.0.0.1:" + port);
        return {
          browser,
          cleanup: async () => {
            try { child.kill(); } catch (_) {}
            fs.rmSync(profile, { recursive: true, force: true });
          },
        };
      }
    } catch (_) { /* not up yet */ }
    await new Promise((r) => setTimeout(r, 300));
  }
  try { child.kill(); } catch (_) {}
  fs.rmSync(profile, { recursive: true, force: true });
  throw new Error("Chromium debug endpoint did not come up on port " + port);
}

// Kept at module scope so the catch handler can dump progress on failure.
const results = [];
const jsErrors = [];
let page = null;

(async () => {
  const { browser, cleanup } = await launchBrowser();
  page = await browser.newPage();
  const errors = jsErrors;
  page.on("pageerror", (e) => errors.push("pageerror: " + e.message));
  page.on("console", (m) => { if (m.type() === "error") errors.push("console: " + m.text()); });

  const check = (name, ok) => { results.push([ok ? "PASS" : "FAIL", name]); if (!ok) process.exitCode = 1; };

  await page.goto(url);
  await page.waitForTimeout(300);

  // Bestand: navigation between the five areas
  const areas = [
    ["diktieren", "view-diktieren"],
    ["meetings", "view-meetings"],
    ["projekte", "view-projekte"],
    ["diktatverlauf", "view-diktatverlauf"],
    ["einstellungen", "view-einstellungen"],
  ];
  for (const [route, viewId] of areas) {
    await page.goto(url + "#/" + route);
    await page.waitForTimeout(100);
    const visible = await page.evaluate((id) => {
      const el = document.getElementById(id);
      return el && !el.hidden && !!el.offsetParent;
    }, viewId);
    check(`nav: ${route} -> ${viewId} visible`, visible);
  }
  await page.goto(url + "#/projekte");
  await page.waitForTimeout(100);
  const visCount = await page.evaluate(() =>
    [...document.querySelectorAll(".view")].filter((v) => !v.hidden).length);
  check("only one view visible at a time", visCount === 1);
  check("pinned entry hidden initially",
        await page.evaluate(() => document.getElementById("pinned-recording").hidden === true));

  /* ===== G4: Startdialog-Abschluss → Countdown → Laufendansicht ===== */
  await page.goto(url + "#/meetings");
  await page.waitForTimeout(100);
  await page.click("#btn-neues-meeting");
  check("start dialog opens", await page.evaluate(() => !document.getElementById("start-dialog").hidden));
  await page.click("#btn-meeting-starten");
  await page.waitForTimeout(80);

  // Countdown statt Sofort-Start; noch kein Aufnahmezustand irgendwo (G3)
  check("G4 countdown overlay shown after submit",
        await page.evaluate(() => !document.getElementById("countdown-overlay").hidden));
  check("§6.9 countdown starts at 3",
        await page.evaluate(() => document.getElementById("countdown-number").textContent === "3"));
  check("G3 no recording state during countdown",
        await page.evaluate(() =>
          document.getElementById("pinned-recording").hidden &&
          document.getElementById("view-meeting-aktiv").hidden));
  // Regression: das Overlay liegt außerhalb der (versteckten) Ansicht und ist
  // während des Countdowns tatsächlich am Bildschirm sichtbar
  check("§6.9 countdown overlay actually rendered on screen",
        await page.evaluate(() => {
          const ov = document.getElementById("countdown-overlay");
          return !ov.hidden && !!ov.querySelector(".modal").offsetParent;
        }));

  // Escape bricht nur den Countdown ab und landet im intakten Dialog
  await page.keyboard.press("Escape");
  await page.waitForTimeout(60);
  check("§6.9 Escape cancels countdown",
        await page.evaluate(() => document.getElementById("countdown-overlay").hidden === true));
  check("§6.9 cancel returns to intact start dialog",
        await page.evaluate(() => !document.getElementById("start-dialog").hidden));
  check("G3 nothing recorded after cancel",
        await page.evaluate(() =>
          document.getElementById("pinned-recording").hidden &&
          location.hash.indexOf("meeting-aktiv") === -1));

  // Erneut starten; Countdown läuft ab
  await page.click("#btn-meeting-starten");
  await page.waitForTimeout(3300);
  check("G4 countdown finishes into running view",
        await page.evaluate(() =>
          document.getElementById("countdown-overlay").hidden &&
          !document.getElementById("view-meeting-aktiv").hidden));
  // Die Uhr läuft im Sekundentakt; auf den ersten Tick warten statt zur
  // rennempfindlichen Stelle direkt zu sampeln.
  let timerCounts = true;
  try {
    await page.waitForFunction(
      () => document.querySelector(".pinned-time").textContent !== "00:00:00",
      null, { timeout: 4000 });
  } catch (_) { timerCounts = false; }
  const pinned0 = await page.evaluate(() => ({
    hidden: document.getElementById("pinned-recording").hidden,
    label: document.querySelector(".pinned-label").textContent,
    dotColor: getComputedStyle(document.querySelector(".rec-dot")).backgroundColor,
    timer: document.querySelector(".pinned-time").textContent,
  }));
  check("G4 pinned entry visible with running session", !pinned0.hidden);
  check("G4 timer counts (" + pinned0.timer + ")", timerCounts && /^00:00:0[1-9]/.test(pinned0.timer));

  /* ===== G3: Farbsemantik während echter Aufnahme (§6.8) ===== */
  await page.goto(url + "#/meeting-aktiv");
  await page.waitForTimeout(100);
  const recState = await page.evaluate(() => {
    const chip = document.getElementById("live-status-chip");
    return {
      cls: chip.className,
      icon: document.getElementById("live-status-icon").textContent,
      text: document.getElementById("live-status-text").textContent,
      color: getComputedStyle(chip).color,
      dotColor: getComputedStyle(document.querySelector(".rec-dot")).backgroundColor,
    };
  });
  check("G3 status bar chip red class while recording", recState.cls.includes("status-recording"));
  check("G3 chip has icon + text", recState.icon === "●" && recState.text === "Aufnahme läuft");
  check("G3 chip colour is danger red", recState.color === "rgb(179, 38, 30)");
  check("G3 pinned dot red while recording", recState.dotColor === "rgb(179, 38, 30)");

  // Regression: Navigation darf der laufenden Aufnahme keine Sekunden schenken
  // (alter Bug: render() -> tick() zählte pro Hashwechsel +1 hoch). Gegen die
  // Wanduhr gemessen: die angezeigte Differenz darf die real verstrichene Zeit
  // nicht überschreiten. Ein Intervall-Tick kann legitim in das Messfenster
  // fallen, dessen volle Sekunde teilweise vor dem Fenster lag – daher ist
  // gegen ceil(Wanduhr) + 1 begrenzt; ein echter Phantom-Bug (+N pro
  // Navigation) würde diese Grenze bei mehreren Navigationen sprengen.
  const navTimer = {
    before: await page.evaluate(() => document.querySelector(".pinned-time").textContent),
    t0: Date.now(),
  };
  for (const r of ["meetings", "projekte", "meeting-aktiv"]) {
    await page.goto(url + "#/" + r);
    await page.waitForTimeout(120);
  }
  await page.waitForTimeout(1400); // Messfenster groß genug für Tick-Phasen-Sicherheit
  navTimer.after = await page.evaluate(() => document.querySelector(".pinned-time").textContent);
  const wallSec = (Date.now() - navTimer.t0) / 1000;
  const countedSec = Number(navTimer.after.slice(-2)) - Number(navTimer.before.slice(-2));
  // Die reinen Wartezeiten garantieren wallSec >= 1,76 s, die Prüfung unten
  // ist damit unabhängig von der Maschinengeschwindigkeit erfüllbar.
  check(`navigation adds no phantom seconds to the timer (counted ${countedSec}s, wall ${wallSec.toFixed(2)}s)`,
        countedSec >= 0 && countedSec <= Math.ceil(wallSec) + 1 && wallSec >= 1.7);

  /* ===== G2: Scroll-Lock + „Zum aktuellen Text“ (§6.11) ===== */
  await page.waitForFunction(() => {
    const a = document.querySelector(".transcript-area");
    return a.scrollHeight > a.clientHeight;
  }, null, { timeout: 20000 });
  await page.evaluate(() => {
    const a = document.querySelector(".transcript-area");
    a.scrollTop = 0;
    a.dispatchEvent(new Event("scroll"));
  });
  await page.waitForTimeout(50);
  check("G2 jump-to-current button revealed on scroll-up",
        await page.evaluate(() => document.getElementById("btn-zum-text").hidden === false));
  await page.waitForTimeout(1600); // neue Segmente kommen, während der Nutzer oben ist
  check("G2 auto-scroll halted after user scrolled up",
        await page.evaluate(() => {
          const a = document.querySelector(".transcript-area");
          return a.scrollTop <= 2 && a.scrollHeight > a.clientHeight;
        }));
  await page.click("#btn-zum-text");
  await page.waitForTimeout(80);
  check("G2 jump button scrolls to bottom and hides itself",
        await page.evaluate(() => {
          const a = document.querySelector(".transcript-area");
          return a.scrollTop + a.clientHeight >= a.scrollHeight - 24 &&
                 document.getElementById("btn-zum-text").hidden === true;
        }));

  /* ===== Pause mit Timeline-Marker (§6.11 / „Pause“) ===== */
  await page.click("#btn-pause");
  await page.waitForTimeout(80);
  const pausedState = await page.evaluate(() => ({
    icon: document.getElementById("live-status-icon").textContent,
    text: document.getElementById("live-status-text").textContent,
    chipCls: document.getElementById("live-status-chip").className,
    chipColor: getComputedStyle(document.getElementById("live-status-chip")).color,
    pinLabel: document.querySelector(".pinned-label").textContent,
    pinCls: document.getElementById("pinned-recording").className,
    dotColor: getComputedStyle(document.querySelector(".rec-dot")).backgroundColor,
    markers: [...document.querySelectorAll("#live-segments .timeline-marker")]
      .map((li) => li.textContent).join(" | "),
    micLevel: document.getElementById("mic-level").value,
    sysLevel: document.getElementById("sys-level").value,
    pauseBtn: document.getElementById("btn-pause").textContent,
  }));
  check("pause switches chip to paused state (icon+text)",
        pausedState.icon === "⏸" && pausedState.text === "Pausiert" &&
        pausedState.chipCls.includes("status-paused"));
  check("G3 paused chip is orange, not red", pausedState.chipColor === "rgb(154, 91, 0)");
  check("G3 pinned entry shows orange paused state",
        pausedState.pinLabel === "Pausiert" &&
        pausedState.pinCls.includes("is-paused") &&
        pausedState.dotColor === "rgb(154, 91, 0)");
  check("pause writes timeline marker", /Pausiert/.test(pausedState.markers));
  check("levels drop to zero while paused",
        pausedState.micLevel === 0 && pausedState.sysLevel === 0);
  check("pause button labelled 'Fortsetzen'", /Fortsetzen/.test(pausedState.pauseBtn));
  const frozenTimer = await page.evaluate(() => document.querySelector(".pinned-time").textContent);
  await page.waitForTimeout(1400);
  check("duration freezes while paused",
        (await page.evaluate(() => document.querySelector(".pinned-time").textContent)) === frozenTimer);

  await page.click("#btn-pause"); // Fortsetzen
  await page.waitForTimeout(80);
  const resumedState = await page.evaluate(() => ({
    text: document.getElementById("live-status-text").textContent,
    chipCls: document.getElementById("live-status-chip").className,
    chipColor: getComputedStyle(document.getElementById("live-status-chip")).color,
    markers: [...document.querySelectorAll("#live-segments .timeline-marker")]
      .map((li) => li.textContent).join(" | "),
  }));
  check("resume restores recording state (red + icon + text)",
        resumedState.text === "Aufnahme läuft" &&
        resumedState.chipCls.includes("status-recording") &&
        resumedState.chipColor === "rgb(179, 38, 30)");
  check("resume writes timeline marker", /Fortgesetzt/.test(resumedState.markers));

  /* ===== G1: Ein einzelner Druck auf „Meeting beenden“ beendet nie direkt ===== */
  await page.click("#btn-beenden");
  await page.waitForTimeout(50);
  check("G1 stop press only opens confirmation",
        await page.evaluate(() =>
          !document.getElementById("stop-confirm").hidden &&
          !document.getElementById("pinned-recording").hidden));
  await page.keyboard.press("Escape");
  await page.waitForTimeout(50);
  check("G1 Escape cancels stop, recording continues",
        await page.evaluate(() =>
          document.getElementById("stop-confirm").hidden &&
          !document.getElementById("pinned-recording").hidden));
  await page.click("#btn-beenden");
  await page.waitForTimeout(50);
  await page.click("#btn-stop-abbruch");
  await page.waitForTimeout(50);
  check("G1 explicit 'Weiter aufnehmen' keeps recording",
        await page.evaluate(() =>
          document.getElementById("stop-confirm").hidden &&
          !document.getElementById("pinned-recording").hidden));

  /* ===== §6.9: Bestätigung verschwindet nach kurzer Zeit ohne Zustandsänderung ===== */
  await page.click("#btn-beenden");
  await page.waitForTimeout(50);
  check("confirmation visible before auto-dismiss",
        await page.evaluate(() => !document.getElementById("stop-confirm").hidden));
  await page.waitForTimeout(10400);
  check("§6.9 confirmation auto-dismisses without stopping",
        await page.evaluate(() =>
          document.getElementById("stop-confirm").hidden &&
          !document.getElementById("pinned-recording").hidden));

  /* ===== G4: bestätigtes Beenden → Verarbeitung ===== */
  await page.click("#btn-beenden");
  await page.waitForTimeout(50);
  await page.click("#btn-stop-ok");
  await page.waitForTimeout(200);
  check("G4 confirmed stop reaches processing view",
        await page.evaluate(() =>
          !document.getElementById("view-verarbeitung").hidden &&
          document.getElementById("pinned-recording").hidden));
  await page.waitForTimeout(3700);
  check("checklist completes",
        await page.evaluate(() =>
          document.querySelector('#proc-checklist li[data-step="summary"]').className === "done"));
  await page.goto(url + "#/meetings");
  await page.waitForTimeout(200);
  check("finished meeting appears in list",
        await page.evaluate(() =>
          document.querySelectorAll("#meeting-list .meeting-item").length >= 3));

  /* ===== Regression: Session-Neustart stellt die Auto-Nachführung wieder her =====
     Der Nutzer ist in G2 nach oben gescrollt (stickToBottom = false geblieben);
     eine frische Session muss trotzdem wieder am aktuellen Text kleben,
     ohne dass der Sprung-Button dafür nötig wäre (§6.11). */
  await page.click("#btn-neues-meeting");
  await page.waitForTimeout(80);
  await page.click("#btn-meeting-starten");
  await page.waitForTimeout(3300);
  check("reg-G2 fresh session reaches running view",
        await page.evaluate(() => !document.getElementById("view-meeting-aktiv").hidden));
  await page.waitForFunction(() => {
    const a = document.querySelector(".transcript-area");
    return a.scrollHeight > a.clientHeight;
  }, null, { timeout: 25000 });
  await page.waitForTimeout(1600); // neue Segmente kommen – ganz ohne Nutzer-Scroll
  check("reg-G2 auto-follow restored in fresh session despite earlier scroll-up",
        await page.evaluate(() => {
          const a = document.querySelector(".transcript-area");
          return a.scrollTop + a.clientHeight >= a.scrollHeight - 24 &&
                 document.getElementById("btn-zum-text").hidden === true;
        }));
  // Zweite Sitzung ordentlich beenden, damit die Folgeabschnitte sauber starten
  await page.click("#btn-beenden");
  await page.waitForTimeout(50);
  await page.click("#btn-stop-ok");
  await page.waitForTimeout(200);

  /* ===== Regression: task-0178 Blatt (§6.6 Ersteinrichtung + §6.7 Startdialog) ===== */

  await page.goto(url + "#/meetings");
  await page.waitForTimeout(100);
  await page.click("#btn-open-setup");
  await page.waitForTimeout(100);
  check("reg-0178 setup view reachable",
        await page.evaluate(() => {
          const v = document.getElementById("view-meeting-einrichtung");
          return v && !v.hidden && !!v.offsetParent;
        }));
  check("§6.6 provider default 'Lokal' preselected",
        await page.evaluate(() =>
          document.querySelector('input[name="setup-provider"][value="lokal"]').checked));
  check("§6.6 Azure explains upload",
        await page.evaluate(() =>
          /hochgeladen/.test(document.querySelector("#view-meeting-einrichtung").textContent)));
  check("§6.6 Custom sits under 'Weitere Optionen'",
        await page.evaluate(() => {
          const d = document.querySelector("#view-meeting-einrichtung details.more-options");
          return !!d && !d.open &&
            !!d.querySelector('input[name="setup-provider"][value="custom"]');
        }));

  await page.click("#btn-test-mic");
  const micRunning = await page.evaluate(() =>
    Number(document.getElementById("setup-mic-level").value) > 0);
  check("§6.6 mic test shows live level", micRunning);
  await page.waitForTimeout(2400);
  check("§6.6 mic test passes",
        /funktioniert/.test(await page.textContent("#result-mic")));
  await page.click("#btn-test-sys");
  await page.waitForTimeout(2400);
  check("§6.6 system test confirms loopback",
        /Loopback empfängt/.test(await page.textContent("#result-sys")));

  await page.check("#sim-no-mic");
  await page.click("#btn-test-mic");
  check("§6.6 missing device yields fail status",
        /Kein Mikrofon erkannt/.test(await page.textContent("#result-mic")));
  check("§6.6 correction action offered",
        await page.evaluate(() => {
          const h = document.getElementById("setup-audio-hints");
          return !h.hidden && !!h.querySelector("button[data-fix]");
        }));
  await page.uncheck("#sim-no-mic");

  await page.waitForTimeout(700);
  check("§6.6 shortcut candidate Ctrl+Alt+Space checked and available",
        /verfügbar/.test(await page.textContent("#shortcut-cand-status")));
  await page.check("#sim-shortcut-taken");
  await page.click("#btn-shortcut-check");
  await page.waitForTimeout(700);
  check("§6.6 taken candidate offers conflict-free alternatives",
        await page.evaluate(() =>
          !document.getElementById("shortcut-alternatives").hidden &&
          document.querySelectorAll(".alt-pick").length >= 2));
  await page.uncheck("#sim-shortcut-taken");

  await page.click("#btn-setup-finish");
  await page.waitForTimeout(150);
  check("§6.6 finish opens compact start dialog",
        await page.evaluate(() => !document.getElementById("start-dialog").hidden));

  // All four status rows clickable, scoped picker, dialog state preserved
  const titleBefore = await page.inputValue("#meeting-title-input");
  for (const row of ["mic", "sys", "provider", "lang"]) {
    await page.click("#row-" + row);
    await page.waitForTimeout(50);
    const open = await page.evaluate(() =>
      !document.getElementById("row-picker").hidden &&
      document.querySelectorAll("#row-picker .picker-option").length > 0);
    check(`reg-0178 row '${row}' opens scoped picker`, open);
    await page.click("#row-picker .picker-back");
    await page.waitForTimeout(50);
    check(`reg-0178 row '${row}' picker closes back to dialog`,
          await page.evaluate(() =>
            document.getElementById("row-picker").hidden === true &&
            !document.getElementById("start-dialog").hidden));
  }
  check("reg-0178 dialog state preserved across pickers (title unchanged)",
        (await page.inputValue("#meeting-title-input")) === titleBefore);

  await page.click("#row-mic");
  await page.locator("#row-picker .picker-option", { hasText: "USB-Headset" }).click();
  await page.waitForTimeout(50);
  check("reg-0178 picking a device updates its row live",
        /USB-Headset/.test(await page.textContent("#row-mic-value")));

  await page.click("#project-chip");
  await page.waitForTimeout(50);
  const noneOpt = page.locator("#project-picker .picker-none-opt");
  check("reg-0178 'Kein Projekt' option present and distinct (dashed)",
        (await noneOpt.count()) === 1 &&
        (await noneOpt.evaluate((el) => getComputedStyle(el).borderStyle)) === "dashed");
  await page.locator("#project-picker .picker-option", { hasText: "BDNA Club" }).click();
  check("reg-0178 selecting a project reflects on chip",
        (await page.textContent("#project-chip")).trim() === "BDNA Club");
  await noneOpt.click();
  await page.waitForTimeout(50);
  check("reg-0178 'Kein Projekt' selectable again, chip visibly unassigned",
        (await page.textContent("#project-chip")).trim() === "Kein Projekt" &&
        (await page.getAttribute("#project-chip", "class")).includes("chip-none"));

  // Start disabled for mocked missing device, fixable in-place
  await page.click("#row-mic");
  await page.getByRole("button", { name: /Simulation: Gerät entfernen/ }).click();
  await page.waitForTimeout(50);
  check("reg-G3 start disabled while mic missing",
        await page.evaluate(() => document.getElementById("btn-meeting-starten").disabled));
  check("reg-G3 blocker names the fix location",
        await page.evaluate(() => {
          const b = document.getElementById("start-blocker");
          return !b.hidden && /Mikrofon/.test(b.textContent);
        }));
  await page.getByRole("button", { name: /Simulation zurücksetzen|Standardmikrofon/ }).first().click();
  await page.waitForTimeout(50);
  check("reg-G3 fixing device re-enables start, dialog intact",
        await page.evaluate(() =>
          !document.getElementById("btn-meeting-starten").disabled &&
          !document.getElementById("start-dialog").hidden));

  // Cloud provider without key
  await page.click("#row-provider");
  await page.locator("#row-picker .picker-option", { hasText: "Azure Cloud" }).click();
  await page.waitForTimeout(50);
  check("reg-G3 azure without key disables start",
        await page.evaluate(() => document.getElementById("btn-meeting-starten").disabled));
  await page.getByRole("button", { name: /Azure-Schlüssel hinzufügen/ }).click();
  await page.waitForTimeout(50);
  check("reg-G3 adding key re-enables start",
        await page.evaluate(() => !document.getElementById("btn-meeting-starten").disabled));
  await page.getByRole("button", { name: /^Lokal auf diesem Gerät/ }).click();

  // Escape first closes only the picker; second press closes the dialog
  await page.click("#row-lang");
  await page.keyboard.press("Escape");
  await page.waitForTimeout(50);
  check("reg-0178 Escape closes picker but keeps dialog open",
        await page.evaluate(() =>
          document.getElementById("row-picker").hidden === true &&
          !document.getElementById("start-dialog").hidden));
  await page.keyboard.press("Escape");
  await page.waitForTimeout(50);
  check("second Escape closes dialog",
        await page.evaluate(() => document.getElementById("start-dialog").hidden === true));

  /* ===== task-0180 Blatt: §6.12 Checkliste, §6.5 Liste, §6.13 Detail ===== */

  // Dialog-Handler: window.confirm-Nachrichten mitprotokollieren und
  // standardmäßig annehmen (für die Lösch-Bestätigungen).
  const confirmMessages = [];
  let confirmAnswer = true;
  page.on("dialog", async (d) => {
    if (d.type() === "confirm") {
      confirmMessages.push(d.message());
      if (confirmAnswer) await d.accept(); else await d.dismiss();
    } else await d.accept();
  });

  await page.goto(url + "#/meetings");
  await page.waitForTimeout(150);

  /* --- G4: Liste -> Detail -> zurück --- */
  const cardCount = await page.evaluate(() =>
    document.querySelectorAll("#meeting-list .card-open").length);
  check("G4(list) populated list renders cards with open targets", cardCount >= 3);
  await page.locator("#meeting-list .card-open").first().click();
  await page.waitForTimeout(100);
  check("G4 list card opens detail view",
        await page.evaluate(() =>
          !document.getElementById("view-meeting-detail").hidden &&
          document.getElementById("view-meetings").hidden));
  check("§6.13 detail defaults to Übersicht tab",
        await page.evaluate(() =>
          !document.querySelector('[data-dpanel="uebersicht"]').hidden &&
          document.querySelector('[data-dpanel="transkript"]').hidden &&
          document.querySelector('[data-dtab="uebersicht"]').getAttribute("aria-selected") === "true"));
  check("§6.13 overview shows summary/decisions/tasks/questions",
        await page.evaluate(() => {
          const v = document.getElementById("view-meeting-detail");
          return !!v.querySelector("#detail-summary").textContent &&
            !!v.querySelector("#detail-decisions") &&
            !!v.querySelector("#detail-tasks") &&
            !!v.querySelector("#detail-questions");
        }));
  check("§6.13 labeled actions Kopieren/Exportieren/Weitere Aktionen present",
        await page.evaluate(() =>
          document.getElementById("btn-detail-copy").textContent === "Kopieren" &&
          document.getElementById("btn-detail-export").textContent === "Exportieren" &&
          /Weitere Aktionen/.test(document.querySelector("#detail-more-menu summary").textContent)));
  await page.click("#btn-detail-back");
  await page.waitForTimeout(100);
  check("G4 detail back returns to list",
        await page.evaluate(() =>
          !document.getElementById("view-meetings").hidden &&
          document.getElementById("view-meeting-detail").hidden));

  /* --- Detail-Tabs inkl. Transkript-Suche (Gate G3) --- */
  await page.locator("#meeting-list .card-open", { hasText: "Wochenplanung" }).click();
  await page.waitForTimeout(100);
  for (const tabName of ["transkript", "audio", "verarbeitung"]) {
    await page.click(`[data-dtab="${tabName}"]`);
    await page.waitForTimeout(50);
    const vis = await page.evaluate((n) =>
      !document.querySelector(`[data-dpanel="${n}"]`).hidden, tabName);
    check(`§6.13 tab '${tabName}' opens its panel`, vis);
  }
  await page.click('[data-dtab="transkript"]');
  await page.waitForTimeout(50);
  // Container künstlich schmal machen, damit der Sprung real scrollen muss
  await page.evaluate(() => {
    document.getElementById("detail-transcript-area").style.maxHeight = "120px";
  });
  await page.fill("#detail-transcript-search", "Messestand");
  await page.waitForTimeout(80);
  const g3 = await page.evaluate(() => {
    const hits = [...document.querySelectorAll("#detail-segments .seg-hit")];
    const first = document.querySelector("#detail-segments .seg-hit-first");
    const area = document.getElementById("detail-transcript-area");
    return {
      hitCount: hits.length,
      focused: document.activeElement === first,
      scrolled: area.scrollTop > 2,
      pos: first ? first.dataset.pos : null,
      status: document.getElementById("transcript-search-status").textContent,
    };
  });
  check("G3 search highlights matching segments", g3.hitCount === 1);
  check("G3 search jumps to mocked segment position",
        g3.focused && g3.scrolled && g3.pos === "0:06");
  check("G3 search status names segment index and audio position",
        /Segment 3 von 9/.test(g3.status) && /0:06/.test(g3.status));
  await page.evaluate(() => {
    const el = document.getElementById("detail-transcript-area");
    if (el) { el.style.maxHeight = ""; }
  });

  /* --- Geteilte Ansicht (§6.13): optional, nie Default --- */
  const splitOffered = await page.evaluate(() => {
    const t = document.querySelector(".split-toggle");
    return t && getComputedStyle(t).display !== "none";   // Viewport 1280 breit
  });
  check("§6.13 split view offered on wide screens", splitOffered);
  check("§6.13 split view NOT active by default",
        await page.evaluate(() =>
          !document.getElementById("view-meeting-detail").classList.contains("split-active")));
  await page.check("#split-toggle");
  await page.waitForTimeout(80);
  const splitOn = await page.evaluate(() =>
    document.getElementById("view-meeting-detail").classList.contains("split-active") &&
    !document.querySelector('[data-dpanel="transkript"]').hidden &&
    !document.querySelector('[data-dpanel="uebersicht"]').hidden);
  check("§6.13 split view shows Übersicht+Transkript side by side", splitOn);
  await page.uncheck("#split-toggle");
  await page.waitForTimeout(80);
  check("§6.13 unchecking split view restores single tab",
        await page.evaluate(() => {
          const v = document.getElementById("view-meeting-detail");
          return !v.classList.contains("split-active") &&
            document.querySelector('[data-dpanel="transkript"]').hidden;
        }));

  // Titel im Detail bearbeiten → erscheint in der Liste (G4-Verzahnung)
  await page.fill("#detail-title", "Wochenplanung (umbenannt)");
  await page.dispatchEvent("#detail-title", "change");
  await page.click("#btn-detail-back");
  await page.waitForTimeout(100);
  check("G4 edited detail title appears in list",
        await page.evaluate(() =>
          [...document.querySelectorAll("#meeting-list .card-open")]
            .some((b) => /Wochenplanung \(umbenannt\)/.test(b.textContent))));
  await page.locator("#meeting-list .card-open", { hasText: "Wochenplanung (umbenannt)" }).click();
  await page.waitForTimeout(80);
  await page.fill("#detail-title", "Wochenplanung");
  await page.dispatchEvent("#detail-title", "change");

  /* --- G2: Delete ohne Hover, benannte Artefakte (Liste + Detail) --- */
  confirmMessages.length = 0;
  await page.goto(url + "#/meetings");
  await page.waitForTimeout(150);
  const before = await page.evaluate(() =>
    document.querySelectorAll("#meeting-list .meeting-item").length);
  // Drei-Punkte-Menü per Klick – kein Hover beteiligt
  await page.locator("#meeting-list .menu-btn").nth(1).click();
  await page.waitForTimeout(60);
  check("G2 three-dot menu opens by click (no hover needed)",
        await page.evaluate(() => !!document.querySelector("#meeting-list .menu")));
  check("G2 delete item always visible in menu (not hover-gated)",
        await page.evaluate(() => {
          const b = [...document.querySelectorAll("#meeting-list .menu .menu-danger")]
            .find((x) => /Löschen/.test(x.textContent));
          if (!b) return false;
          const cs = getComputedStyle(b);
          return cs.display !== "none" && cs.visibility !== "hidden" && cs.opacity !== "0";
        }));
  const targetTitle = await page.evaluate(() =>
    document.querySelectorAll("#meeting-list .card-open")[1].textContent.split("·")[0].trim());
  await page.locator("#meeting-list .menu .menu-danger").click();
  await page.waitForTimeout(100);
  check("G2 delete confirm names the artifacts (Audio/Transkript/Zusammenfassung)",
        confirmMessages.length === 1 &&
        /Audio,\s*Transkript und Zusammenfassung/.test(confirmMessages[0]) &&
        confirmMessages[0].indexOf(targetTitle) !== -1);
  check("G2 confirmed delete removes the meeting",
        (await page.evaluate(() =>
          document.querySelectorAll("#meeting-list .meeting-item").length)) === before - 1);

  // Gleiche Bestätigung aus dem Detail-Menü; Abbruch entfernt nichts
  await page.locator("#meeting-list .card-open").first().click();
  await page.waitForTimeout(80);
  await page.click("#detail-more-menu summary");
  await page.waitForTimeout(60);
  confirmMessages.length = 0;
  confirmAnswer = false;
  await page.click("#btn-detail-delete");
  await page.waitForTimeout(80);
  check("G2 detail menu delete asks with named artifacts too",
        confirmMessages.length === 1 &&
        /Audio,\s*Transkript und Zusammenfassung/.test(confirmMessages[0]));
  check("G2 declining delete keeps the meeting and stays in detail",
        await page.evaluate(() =>
          !document.getElementById("view-meeting-detail").hidden));
  confirmAnswer = true;

  /* --- G1: Checkliste mit deterministischen Stückzahlen, nie Prozent --- */
  await page.goto(url + "#/meetings");
  await page.waitForTimeout(100);
  await page.click("#btn-neues-meeting");
  await page.waitForTimeout(80);
  await page.click("#btn-meeting-starten");
  await page.waitForTimeout(3400);
  await page.click("#btn-beenden");
  await page.waitForTimeout(50);
  await page.click("#btn-stop-ok");
  await page.waitForTimeout(250);
  const early = await page.evaluate(() => ({
    chunks: document.querySelector('#proc-checklist li[data-step="chunks"]').textContent,
    html: document.getElementById("proc-checklist").innerHTML,
  }));
  check("G1 while total unknown: preparation shows NO count and NO percentage",
        early.chunks.indexOf("%") === -1 && !/\d/.test(early.chunks));
  check("G1 checklist never uses a fake percentage",
        early.html.indexOf("%") === -1);
  await page.waitForTimeout(3550);
  const lateCk = await page.evaluate(() => {
    const get = (s) => document.querySelector(`#proc-checklist li[data-step="${s}"]`);
    return {
      chunks: get("chunks").textContent,
      transcribeCls: get("transcribe").className,
      transcribe: get("transcribe").textContent,
      summaryDone: get("summary").className === "done",
      html: document.getElementById("proc-checklist").innerHTML,
    };
  });
  {
    const mm = lateCk.transcribe.match(/^✓ (\d+) von (\d+) Abschnitten transkribiert$/);
    check("G1 final transcription count is deterministic n von n", !!mm && mm[1] === mm[2]);
    const cm = lateCk.chunks.match(/^✓ (\d+) von (\d+)/);
    check("G1 preparation count consistent with transcription count",
          !!cm && !!mm && cm[1] === cm[2] && cm[1] === mm[1]);
  }
  check("G1 checklist completes into summary without any percentage",
        lateCk.summaryDone && lateCk.transcribeCls === "done" &&
        lateCk.html.indexOf("%") === -1);

  /* --- §6.12: Fehlerhafte Abschnitte erneut versuchen (deterministisch) --- */
  await page.check("#sim-chunk-error");
  await page.goto(url + "#/meetings");
  await page.waitForTimeout(120);
  await page.goto(url + "#/verarbeitung");     // erneuter Checklistendurchlauf
  await page.waitForTimeout(300);
  // Im Hintergrund weiterarbeiten – Liste zeigt das Meeting als in Verarbeitung
  await page.click("#btn-proc-bg");
  await page.waitForTimeout(120);
  check("§6.12 background-continue returns to the list",
        await page.evaluate(() => !document.getElementById("view-meetings").hidden));
  check("§6.12 processing meeting stays visible in the list",
        await page.evaluate(() =>
          [...document.querySelectorAll("#meeting-list .status-chip")]
            .some((c) => /In Verarbeitung/.test(c.textContent))));
  await page.goto(url + "#/verarbeitung");
  await page.waitForTimeout(2900);
  check("§6.12 failed chunk surfaces error state + retry action",
        await page.evaluate(() => {
          const tr = document.querySelector('#proc-checklist li[data-step="transcribe"]');
          return tr.className === "error" && /fehlerhaft/.test(tr.textContent) &&
            !document.getElementById("btn-retry-failed").hidden;
        }));
  await page.click("#btn-retry-failed");
  await page.waitForTimeout(2000);
  check("§6.12 retry completes transcription and summary",
        await page.evaluate(() =>
          document.querySelector('#proc-checklist li[data-step="summary"]').className === "done" &&
          document.getElementById("btn-retry-failed").hidden));
  check("§6.12 completion offers direct entry to the detail view",
        await page.evaluate(() => !document.getElementById("btn-open-processing-meeting").hidden));
  await page.click("#btn-open-processing-meeting");
  await page.waitForTimeout(120);
  check("§6.12 'Meeting öffnen' reaches the finished meeting's detail",
        await page.evaluate(() =>
          !document.getElementById("view-meeting-detail").hidden &&
          /Fertig/.test(document.getElementById("detail-status-chip").textContent)));
  // Verarbeitungstab zeigt Provider/Modell/Jobs/Wiederholungen (§6.13)
  await page.click('[data-dtab="verarbeitung"]');
  await page.waitForTimeout(50);
  check("§6.13 processing tab shows provider/model/jobs/retries",
        await page.evaluate(() => {
          const p = document.getElementById("detail-processing-panel").textContent;
          return /Whisper Small/.test(p) && /Jobs/.test(p) &&
            /Wiederholungen/.test(p) && /Keine offenen Fehler/.test(p);
        }));
  // Audio-Tab: Wiedergabe-Mock + Aufbewahrungshinweis
  await page.click('[data-dtab="audio"]');
  await page.waitForTimeout(50);
  check("§6.13 audio tab offers playback mock and retention note",
        await page.evaluate(() => {
          const p = document.getElementById("detail-audio-panel");
          return !!p.querySelector("#btn-audio-play") &&
            /lokal auf diesem Gerät gespeichert/.test(p.textContent);
        }));
  // Simulation wieder ausschalten (wir sind gerade in der Detailansicht,
  // daher direkt per DOM statt per sichtbarem Klick)
  await page.evaluate(() => {
    const el = document.getElementById("sim-chunk-error");
    if (el) el.checked = false;
  });

  /* ===== task-0181 Blatt: §6.16 Projekte, §6.17 Detail, §6.18 Zuordnung,
          §6.19 WhatsApp-Import ===== */

  /* --- §6.16: Leerer und gefüllter Zustand, Sortierung, Eingang --- */
  await page.goto(url + "#/projekte");
  await page.waitForTimeout(120);
  check("§6.16 populated home shows inbox entry 'Nicht zugeordnet' first",
        await page.evaluate(() => {
          const first = document.querySelector("#project-list .project-card");
          return !!first && /Nicht zugeordnet/.test(first.textContent) &&
            !!first.querySelector(".badge");
        }));
  await page.check("#sim-no-projects");
  await page.waitForTimeout(80);
  check("§6.16 empty state: primary create + secondary import, toolbar hidden",
        await page.evaluate(() =>
          !document.getElementById("projects-empty").hidden &&
          document.getElementById("projects-toolbar").hidden &&
          document.getElementById("btn-projekt-erstellen-empty") &&
          /WhatsApp-Export importieren/.test(
            document.getElementById("btn-import-empty").textContent)));
  await page.uncheck("#sim-no-projects");
  await page.waitForTimeout(60);

  // Projekt erstellen: genau ein Pflichtfeld Name
  const projCountBefore = await page.evaluate(
    () => document.querySelectorAll("#project-list .project-card").length);
  await page.click("#btn-projekt-erstellen");
  await page.waitForTimeout(60);
  check("§6.16 create dialog blocks empty name",
        await page.evaluate(() =>
          document.getElementById("btn-pc-create").disabled));
  await page.fill("#project-name-input", "A-Testprojekt");
  await page.waitForTimeout(40);
  check("§6.16 create enables submit once name entered",
        await page.evaluate(() => !document.getElementById("btn-pc-create").disabled));
  await page.click("#btn-pc-create");
  await page.waitForTimeout(80);
  check("§6.16 created project appears in list",
        (await page.evaluate(
          () => document.querySelectorAll("#project-list .project-card").length)) ===
        projCountBefore + 1);

  // Sortierung 'Name': alphabetisch vor 'Zuletzt aktiv'
  await page.selectOption("#project-sort", "name");
  await page.waitForTimeout(80);
  check("§6.16 sort by name puts created project first",
        await page.evaluate(() => {
          const cards = [...document.querySelectorAll("#project-list .project-card")]
            .filter((c) => !c.classList.contains("inbox-card"));
          return cards.length >= 2 && /^A-Testprojekt/.test(cards[0].textContent.trim());
        }));
  await page.selectOption("#project-sort", "aktiv");
  await page.waitForTimeout(60);

  // Suche trifft auch normalisierte Quellentexte/Status/Aufgaben
  await page.fill("#project-search", "getränkekarten");
  await page.waitForTimeout(80);
  check("§6.16 search matches normalized source texts",
        await page.evaluate(() => {
          const cards = [...document.querySelectorAll("#project-list .project-card")]
            .filter((c) => !c.classList.contains("inbox-card"));
          return cards.length === 1 && /BDNA Club/.test(cards[0].textContent);
        }));
  await page.fill("#project-search", "");
  await page.waitForTimeout(60);

  /* --- §6.17 + Gate G1: jede KI-Aussage/Task mit Beleglink → Quellstelle --- */
  await page.locator("#project-list .project-link[data-project]",
                     { hasText: "BDNA Club" }).click();
  await page.waitForTimeout(120);
  check("§6.17 detail header shows status line with evidence link",
        await page.evaluate(() => {
          const sl = document.getElementById("pd-statusline");
          return !!sl.querySelector("a.evidence-link") &&
            /Letzter KI-Lauf/.test(document.getElementById("pd-ai-last-run").textContent);
        }));
  const g1Overview = await page.evaluate(() => {
    const ov = document.getElementById("pd-overview");
    const lists = [...ov.querySelectorAll(".evidence-list")];
    return {
      statements: ov.querySelectorAll(".small-heading").length,
      allHaveLinks: lists.length > 0 && lists.every((l) => l.querySelectorAll("a.evidence-link").length >= 1),
      urgentShown: /Dringende Aufgaben/.test(ov.textContent),
      activityShown: /Letzte Aktivität/.test(ov.textContent),
    };
  });
  check("G1 every overview statement has ≥1 evidence link",
        g1Overview.allHaveLinks && g1Overview.statements >= 4 &&
        g1Overview.urgentShown && g1Overview.activityShown);

  await page.click('[data-ptab="aufgaben"]');
  await page.waitForTimeout(80);
  const g1Tasks = await page.evaluate(() => ({
    groups: [...document.querySelectorAll("#pd-tasks .task-group .small-heading")]
      .map((h) => h.textContent).join("|"),
    cardCount: document.querySelectorAll("#pd-tasks .task-card").length,
    allCardsHaveLinks:
      [...document.querySelectorAll("#pd-tasks .task-card")].length > 0 &&
      [...document.querySelectorAll("#pd-tasks .task-card")].every(
        (c) => c.querySelectorAll("a.evidence-link").length >= 1),
    whyBtns: document.querySelectorAll("#pd-tasks .why-btn").length,
  }));
  check("§6.17 task board has Vorgeschlagen/Offen/Erledigt/Verworfen groups",
        ["Vorgeschlagen", "Offen", "Erledigt", "Verworfen"].every((g) =>
          g1Tasks.groups.indexOf(g) !== -1));
  check("G1 every task shows ≥1 evidence link",
        g1Tasks.allCardsHaveLinks);
  // „Warum dringend?“ klappt Begründung + Belege auf (§6.17)
  await page.click("#pd-tasks .why-btn");
  await page.waitForTimeout(60);
  check("§6.17 'Warum dringend?' reveals reason with evidence links",
        await page.evaluate(() => {
          const box = document.querySelector("#pd-tasks .why-box:not([hidden])");
          return !!box && /Warum dringend\?/.test(box.textContent) &&
            box.querySelectorAll("a.evidence-link").length >= 1 &&
            document.querySelector("#pd-tasks .why-btn").getAttribute("aria-expanded") === "true";
        }));

  // Belegklick springt zur gemockten Quellenstelle (Gate G1)
  await page.click("#pd-tasks .task-card a.evidence-link");
  await page.waitForTimeout(120);
  check("G1 evidence link opens mocked source with highlighted line",
        await page.evaluate(() => {
          const sv = document.getElementById("source-viewer");
          return !sv.hidden && !!sv.querySelector(".src-hit") &&
            sv.querySelectorAll("#sv-lines li").length > 1;
        }));
  await page.click("#btn-sv-close");
  await page.waitForTimeout(50);

  // Aufgaben-Aktionen direkt erreichbar; Vorschlag annehmen (§6.17)
  const sugBefore = await page.evaluate(() => {
    const group = [...document.querySelectorAll("#pd-tasks .task-group")]
      .find((g) => /Vorgeschlagen/.test(g.querySelector(".small-heading").textContent));
    return group ? group.textContent : "";
  });
  check("§6.17 AI task starts as Vorgeschlagen with direct actions",
        /Annehmen/.test(sugBefore) && /Erledigen/.test(sugBefore) &&
        /Verwerfen/.test(sugBefore));
  await page.locator('#pd-tasks [data-task-action="annehmen"]').first().click();
  await page.waitForTimeout(80);
  check("§6.17 accepting moves task to Offen",
        await page.evaluate(() => {
          const offen = [...document.querySelectorAll("#pd-tasks .task-group")]
            .find((g) => /Offen/.test(g.querySelector(".small-heading").textContent));
          return !!offen && /Zahlen bis Donnerstag prüfen/.test(offen.textContent);
        }));

  /* --- §6.19 + Gate G3: Schritt 3 listet exakt, was das Gerät verlässt --- */
  await page.goto(url + "#/import");
  await page.waitForTimeout(100);
  check("§6.19 wizard starts at step 1 with two mock files",
        await page.evaluate(() =>
          !document.getElementById("import-step-1").hidden &&
          document.querySelectorAll(".file-pick").length === 2));
  await page.click("#pick-file-a");
  await page.waitForTimeout(80);
  check("§6.19 step 2 shows detected metadata",
        await page.evaluate(() =>
          !document.getElementById("import-step-2").hidden &&
          /BDNA Club/.test(document.getElementById("detected-grid").textContent) &&
          /Dubletten/.test(document.getElementById("detected-grid").textContent)));
  check("§6.19 no dedup preview for fresh archive",
        await page.evaluate(() => document.getElementById("dedup-preview").hidden));
  await page.click("#btn-to-step3");
  await page.waitForTimeout(80);
  check("G3 default is local-only: nothing leaves the device",
        await page.evaluate(() =>
          !document.getElementById("local-disclosure").hidden &&
          document.getElementById("cloud-disclosure").hidden &&
          document.querySelector('input[name="import-mode"][value="lokal"]').checked));
  check("§6.19 local import keeps everything on the device",
        await page.evaluate(() =>
          /Nichts verlässt das Gerät/.test(document.getElementById("local-disclosure").textContent) &&
          /lokal gespeichert/.test(document.getElementById("local-disclosure").textContent)));
  await page.check('input[name="import-mode"][value="cloud"]');
  await page.dispatchEvent('input[name="import-mode"][value="cloud"]', "change");
  await page.waitForTimeout(80);
  const disclosure = await page.evaluate(() => ({
    visible: !document.getElementById("cloud-disclosure").hidden,
    localHidden: document.getElementById("local-disclosure").hidden,
    text: document.getElementById("cloud-leaves-list").textContent +
      " " + document.getElementById("cloud-cost").textContent,
  }));
  check("G3 cloud choice discloses exactly which content leaves the device",
        disclosure.visible && disclosure.localHidden &&
        /Nachrichtentexte/.test(disclosure.text) &&
        /Base64/.test(disclosure.text) &&
        /Sprachnachrichten/.test(disclosure.text) &&
        /Kostenschätzung/.test(disclosure.text));
  check("G3 disclosure names what explicitly stays on the device",
        /Video/.test(disclosure.text) && /Gerät/.test(disclosure.text));

  /* --- Gate G4: wiederholter Import zeigt Dedup-Vorschau, keine Dubletten --- */
  await page.click("#btn-import-back2");   // Schritt 3 → 2
  await page.waitForTimeout(60);
  await page.click("#btn-import-back1");   // Schritt 2 → 1 (Dateiauswahl)
  await page.waitForTimeout(60);
  await page.click("#pick-file-b");   // überlappt mit bereits importiertem Archiv
  await page.waitForTimeout(80);
  check("G4 repeated import shows dedup preview listing each duplicate",
        await page.evaluate(() => {
          const dp = document.getElementById("dedup-preview");
          return !dp.hidden &&
            document.getElementById("detected-dup-count").textContent === "12" &&
            dp.querySelectorAll(".dup-item").length === 12 &&
            /schon vorhanden/.test(dp.textContent) &&
            /idempotent/.test(dp.textContent);
        }));
  await page.click("#btn-to-step3");
  await page.waitForTimeout(60);
  // Lokal lassen (Empfehlung) und Import durchführen
  const totalLine = await (async () => {
    await page.click("#btn-to-step4");
    await page.waitForTimeout(80);
    check("§6.19 step 4 checklist runs local parse/dedup/index first",
          await page.evaluate(() =>
            !document.querySelector('#import-checklist li[data-istep="parse"]').hidden &&
            /dedupliziert/.test(
              document.querySelector('#import-checklist li[data-istep="dedup"]').textContent) &&
            document.getElementById("btn-import-run").disabled === false));
    await page.click("#btn-import-run");
    await page.waitForFunction(() => !document.getElementById("import-result").hidden,
                               null, { timeout: 5000 });
    return page.evaluate(() => document.getElementById("import-result").textContent);
  })();
  check("G4 result imports only new messages, skips duplicates without doubling",
        /Importiert: 14 Nachrichten/.test(totalLine) &&
        /Übersprungen \(Dubletten\): 12/.test(totalLine) &&
        /62/.test(totalLine) && /NICHT doppelt gespeichert/.test(totalLine));
  await page.click("#btn-import-finish");
  await page.waitForTimeout(100);

  /* --- §6.16/§6.18: Eingang mit Stapelzuordnung + Vorschau --- */
  await page.goto(url + "#/eingang");
  await page.waitForTimeout(100);
  const inboxBefore = await page.evaluate(
    () => document.querySelectorAll("#eingang-list .inbox-item input:not(:checked)").length);
  await page.locator("#eingang-list input[data-inbox-idx]").first().check();
  const targetOptions = await page.evaluate(() =>
    [...document.querySelectorAll("#eingang-target option")].map((o) => o.textContent));
  check(`§6.18 target select lists existing projects (options=${JSON.stringify(targetOptions)})`,
        targetOptions.some((t) => t.trim() === "Atlas GmbH"));
  if (targetOptions.some((t) => t.trim() === "Atlas GmbH")) {
    await page.selectOption("#eingang-target", { label: "Atlas GmbH" });
    await page.click("#btn-eingang-assign");
  await page.waitForTimeout(80);
  check("§6.18 batch assignment always shows preview before moving",
        await page.evaluate(() =>
          !document.getElementById("assign-preview").hidden &&
          /Atlas GmbH/.test(document.getElementById("assign-preview-text").textContent) &&
          document.querySelectorAll("#assign-preview-list li").length >= 1));
  await page.click("#btn-assign-cancel");
  await page.waitForTimeout(60);
  check("§6.18 cancelling preview moves nothing",
        (await page.evaluate(
          () => document.querySelectorAll("#eingang-list .inbox-item").length)) === inboxBefore);
  await page.locator("#eingang-list input[data-inbox-idx]").first().check();
  await page.click("#btn-eingang-assign");
  await page.waitForTimeout(60);
  await page.click("#btn-assign-ok");
  await page.waitForTimeout(80);
  check("§6.18 confirmed batch assignment removes items from inbox",
        (await page.evaluate(
          () => document.querySelectorAll("#eingang-list .inbox-item").length)) ===
        inboxBefore - 1);
  }

  /* --- §6.18 + Gate G2: KI-Projektvorschlag erfordert explizite Wahl --- */
  await page.goto(url + "#/meetings");
  await page.waitForTimeout(100);
  await page.click("#btn-neues-meeting");
  await page.waitForTimeout(60);
  // Sicherstellen, dass das Meeting ohne Projekt startet
  let chipWasReset = false;
  if ((await page.textContent("#project-chip")).trim() !== "Kein Projekt") {
    chipWasReset = true;
    await page.click("#project-chip");
    await page.waitForTimeout(50);
    await page.click("#project-picker .picker-none-opt");
    await page.waitForTimeout(50);
  }
  await page.click("#btn-meeting-starten");
  await page.waitForTimeout(3300);
  await page.click("#btn-beenden");
  await page.waitForTimeout(50);
  await page.click("#btn-stop-ok");
  await page.waitForFunction(
    () => document.querySelector('#proc-checklist li[data-step="summary"]').className === "done",
    null, { timeout: 8000 });
  await page.waitForTimeout(900);   // finishChecklist-Nachlauf mit dem Vorschlag
  const suggestion = await page.evaluate(() => ({
    visible: !document.getElementById("ai-suggestion").hidden,
    headline: document.getElementById("suggestion-headline").textContent,
    evidence: document.querySelectorAll("#suggestion-evidence a.evidence-link").length,
    buttons: [...document.querySelectorAll("#suggestion-actions button")]
      .map((b) => b.textContent.trim()),
    note: document.getElementById("suggestion-note").textContent,
  }));
  check(`G2 suggestion appears after processing with confidence + 2 evidence links (visible=${suggestion.visible} ev=${suggestion.evidence} chipReset=${chipWasReset} hl="${suggestion.headline.slice(0, 70)}")`,
        suggestion.visible && /Confidence/.test(suggestion.headline) &&
        suggestion.evidence >= 2);
  check("G2 exactly three equal choices: Zuordnen / Anderes Projekt / Ohne Projekt",
        suggestion.buttons.length === 3 &&
        suggestion.buttons[0] === "Zuordnen" &&
        /Anderes Projekt/.test(suggestion.buttons[1]) &&
        /Ohne Projekt/.test(suggestion.buttons[2]));
  check("G2 nothing is moved without an explicit user choice",
        /Ohne deine Wahl wird nichts verschoben/.test(suggestion.note));
  const suggestedTitle =
    (suggestion.headline.match(/„(.+?)“/) || [])[1] || "";
  // Ohne Projekt lassen: Meeting bleibt im Eingang
  await page.click("#btn-suggest-none");
  await page.waitForTimeout(60);
  await page.goto(url + "#/eingang");
  await page.waitForTimeout(100);
  check("G2 declining keeps the meeting unassigned in the inbox",
        suggestedTitle !== "" &&
        (await page.evaluate((t) =>
          [...document.querySelectorAll("#eingang-list .inbox-item")]
            .some((li) => li.textContent.indexOf(t) !== -1), suggestedTitle)));

  // Zweiter Lauf: „Anderes Projekt“ wählt ein anderes Ziel gleichwertig aus
  await page.goto(url + "#/meetings");
  await page.waitForTimeout(100);
  await page.click("#btn-neues-meeting");
  await page.waitForTimeout(60);
  if ((await page.textContent("#project-chip")).trim() !== "Kein Projekt") {
    await page.click("#project-chip");
    await page.waitForTimeout(50);
    await page.click("#project-picker .picker-none-opt");
    await page.waitForTimeout(50);
  }
  await page.click("#btn-meeting-starten");
  await page.waitForTimeout(3300);
  await page.click("#btn-beenden");
  await page.waitForTimeout(50);
  await page.click("#btn-stop-ok");
  await page.waitForFunction(
    () => !document.getElementById("ai-suggestion").hidden ||
          document.querySelector('#proc-checklist li[data-step="summary"]').className === "done",
    null, { timeout: 8000 });
  await page.waitForSelector("#ai-suggestion:not([hidden])", { timeout: 4000 });
  await page.click("#btn-suggest-other");
  await page.waitForTimeout(60);
  check("G2 'Anderes Projekt' picker lists existing projects equally reachable",
        await page.evaluate(() =>
          !document.getElementById("suggest-other-picker").hidden &&
          document.querySelectorAll("#suggest-other-picker .picker-option").length >= 2));
  await page.locator("#suggest-other-picker .picker-option",
                     { hasText: "Atlas GmbH" }).click();
  await page.waitForTimeout(80);
  check("G2 choosing another project closes suggestion and assigns it",
        await page.evaluate(() => document.getElementById("ai-suggestion").hidden));

  /* =========================================================================
     Blatt „Klickbarer Meeting-Mode-UX-Prototyp" – dessen eigene Gates G1–G4
     ========================================================================= */

  // Sichtbare Plan-Annotation (Leaf-G4): eine sichtbare .plan-tag-Zeile im
  // genannten Screen, die den Plan-Abschnitt nennt.
  const hasVisiblePlanTag = (sel, rx) => page.evaluate(([s, src]) => {
    const root = document.querySelector(s);
    if (!root) return false;
    const re = new RegExp(src);
    return [...root.querySelectorAll(".plan-tag")]
      .some((t) => !!t.offsetParent && re.test(t.textContent));
  }, [sel, rx.source]);

  /* ===== Leaf-G1: Sieben Stationen klickbar End-to-End (deutsch + englisch)
     ===== leaf-G4: Annotation je Station wird mitgeprüft ===== */
  await page.goto(url + "#/meetings");
  await page.waitForTimeout(100);

  // (1) Leerer Zustand per Simulation klickbar erreichbar
  await page.check("#sim-no-meetings");
  await page.waitForTimeout(60);
  check("leaf-G1 stage 'empty' visible with heading + primary action",
        await page.evaluate(() => {
          const e = document.getElementById("meetings-empty");
          return !e.hidden && !!e.offsetParent &&
            /Noch keine Meetings/.test(e.textContent) &&
            !!e.querySelector("#btn-neues-meeting-empty");
        }));
  check("leaf-G4 empty state carries visible Plan §6.5 annotation",
        await hasVisiblePlanTag("#view-meetings", /§6\.5/));
  await page.click("#btn-neues-meeting-empty");
  await page.waitForTimeout(80);
  check("leaf-G1 empty state's primary action opens the start dialog",
        await page.evaluate(() => !document.getElementById("start-dialog").hidden));
  await page.keyboard.press("Escape");
  await page.waitForTimeout(60);
  await page.uncheck("#sim-no-meetings");
  await page.waitForTimeout(60);
  check("leaf-G1 leaving the simulation restores the populated list",
        await page.evaluate(() =>
          document.getElementById("meetings-empty").hidden &&
          document.querySelectorAll("#meeting-list li").length >= 2));
  check("leaf-G1 English sample meeting exists alongside German ones",
        await page.evaluate(() => {
          const items = [...document.querySelectorAll("#meeting-list li")];
          return items.some((li) => li.textContent.indexOf("Weekly Sync (English)") !== -1) &&
                 items.some((li) => /[äöüß]/i.test(li.textContent));
        }));

  // (2) Ersteinrichtung
  await page.click("#btn-open-setup");
  await page.waitForTimeout(100);
  check("leaf-G1 stage 'setup' clickable from the Meetings page",
        await page.evaluate(() => !document.getElementById("view-meeting-einrichtung").hidden));
  check("leaf-G4 setup carries visible Plan §6.6 annotation",
        await hasVisiblePlanTag("#view-meeting-einrichtung", /§6\.6/));

  // (3) Kompakter Startdialog mit vier Statuszeilen
  await page.click("#btn-setup-finish");
  await page.waitForTimeout(120);
  check("leaf-G1 stage 'start dialog' shows all four status rows",
        await page.evaluate(() =>
          ["row-mic", "row-sys", "row-provider", "row-lang"].every((id) => {
            const b = document.getElementById(id);
            return b && !!b.offsetParent;
          })));
  check("leaf-G4 start dialog carries visible Plan §6.7 annotation",
        await hasVisiblePlanTag("#start-dialog", /§6\.7/));

  // Englisch wählen: Statuszeile Sprache → Mini-Picker → Option „English“
  await page.click("#row-lang");
  await page.waitForTimeout(60);
  await page.locator("#row-picker .picker-option", { hasText: "English" }).click();
  await page.waitForTimeout(60);
  check("leaf-G1 language row switches to English",
        /English/.test(await page.textContent("#row-lang-value")));

  // (4)+(5) Aufnahme und Pause mit englischem Live-Transkript
  await page.click("#btn-meeting-starten");
  await page.waitForTimeout(3300);
  check("leaf-G1 stage 'recording' reached after countdown",
        await page.evaluate(() =>
          document.getElementById("countdown-overlay").hidden &&
          !document.getElementById("view-meeting-aktiv").hidden));
  check("leaf-G4 running view carries visible Plan §6.11 annotation",
        await hasVisiblePlanTag("#view-meeting-aktiv", /§6\.11/));
  let enSegment = true;
  try {
    await page.waitForFunction(() =>
      [...document.querySelectorAll("#live-segments li")]
        .some((li) => /budget/i.test(li.textContent)), null, { timeout: 8000 });
  } catch (_) { enSegment = false; }
  check("leaf-G1 live transcript renders realistic English segments", enSegment);
  await page.click("#btn-pause");
  await page.waitForTimeout(150);
  check("leaf-G1 stage 'paused' shows icon + text, orange not red",
        await page.evaluate(() => {
          const chip = document.getElementById("live-status-chip");
          return chip.className.indexOf("status-paused") !== -1 &&
            document.getElementById("live-status-text").textContent.trim().length > 3 &&
            getComputedStyle(chip).color !== "rgb(179, 38, 30)";
        }));
  await page.click("#btn-pause");   // Fortsetzen
  await page.waitForTimeout(120);

  // (6) Verarbeitung nach Bestätigung
  await page.click("#btn-beenden");
  await page.waitForTimeout(50);
  check("leaf-G4 stop confirmation is annotated with Plan §6.9",
        await hasVisiblePlanTag("#stop-confirm", /§6\.9/));
  await page.click("#btn-stop-ok");
  await page.waitForTimeout(120);
  check("leaf-G1 stage 'processing' shows the four-step checklist",
        await page.evaluate(() =>
          !document.getElementById("view-verarbeitung").hidden &&
          document.querySelectorAll("#proc-checklist li").length === 4));
  check("leaf-G4 processing view carries visible Plan §6.12 annotation",
        await hasVisiblePlanTag("#view-verarbeitung", /§6\.12/));
  await page.waitForFunction(
    () => document.querySelector('#proc-checklist li[data-step="summary"]').className === "done",
    null, { timeout: 9000 });

  // (7) Detailansicht mit Übersicht/Transkript/Audio/Verarbeitung
  await page.click("#btn-open-processing-meeting");
  await page.waitForTimeout(150);
  check("leaf-G1 stage 'detail' opens defaulting to Overview",
        await page.evaluate(() =>
          !document.getElementById("view-meeting-detail").hidden &&
          document.querySelector('[role="tab"][data-dtab="uebersicht"]')
            .getAttribute("aria-selected") === "true"));
  for (const t of ["transkript", "audio", "verarbeitung"]) {
    await page.click(`[role="tab"][data-dtab="${t}"]`);
    await page.waitForTimeout(60);
    check(`leaf-G1 detail tab '${t}' clickable end-to-end`,
          await page.evaluate((tab) => {
            const tabEl = document.querySelector(`[role="tab"][data-dtab="${tab}"]`);
            const panel = document.querySelector(`[data-dpanel="${tab}"]`);
            return tabEl.getAttribute("aria-selected") === "true" && !panel.hidden;
          }, t));
  }
  check("leaf-G4 detail view carries visible Plan §6.13 annotation",
        await hasVisiblePlanTag("#view-meeting-detail", /§6\.13/));

  /* ===== Leaf-G2: 44×44-Ziele, Namen, sichtbarer Tastaturfokus ===== */
  // Systematischer Audit über alle Journey-Screens: jedes gerenderte,
  // interaktive Element braucht ≥44px Breite UND Höhe sowie einen
  // zugänglichen Namen (Text, aria-label/-labelledby, Label oder title).
  async function auditInteractive(rootSel) {
    return page.evaluate((sel) => {
      const root = sel === ":doc" ? document : document.querySelector(sel);
      if (!root || root.closest("[hidden]")) return null;
      const els = [...root.querySelectorAll(
        "button, a[href], input:not([type='hidden']), select, textarea, summary, [tabindex]:not([tabindex='-1'])")];
      const problems = [];
      for (const el of els) {
        if (el.closest("[hidden]")) continue;
        const r = el.getBoundingClientRect();
        if (!r.width && !r.height) continue;   // nicht gerendert (z. B. Menü zu)
        const desc = "<" + el.tagName.toLowerCase() +
          (el.id ? "#" + el.id : "") +
          (el.className && typeof el.className === "string" ? "." + el.className.split(" ").join(".") : "") + ">";
        let name = el.getAttribute("aria-label") || "";
        const lb = el.getAttribute("aria-labelledby");
        if (!name && lb) {
          name = lb.split(/\s+/).map((id) => document.getElementById(id))
            .filter(Boolean).map((n) => n.textContent).join(" ");
        }
        if (!name && /^(input|select|textarea)$/i.test(el.tagName)) {
          name = el.labels && el.labels[0] ? el.labels[0].textContent : "";
        }
        if (!name) name = el.textContent.replace(/\s+/g, " ").trim() || el.title || "";
        if (r.width < 43.5 || r.height < 43.5)
          problems.push("size " + Math.round(r.width) + "x" + Math.round(r.height) + ": " + desc);
        if (!name.trim()) problems.push("no accessible name: " + desc);
      }
      return problems;
    }, rootSel);
  }

  // Alle Ansichten nacheinander sichtbar machen und auditieren; Menüs in der
  // Detailansicht werden geöffnet, damit ihre Punkte mitgeprüft werden.
  const screensToAudit = [
    ["#/diktieren", "#view-diktieren"],
    ["#/meetings", "#view-meetings"],
    ["#/meeting-einrichtung", "#view-meeting-einrichtung"],
    ["#/projekte", "#view-projekte"],
    ["#/eingang", "#view-eingang"],
    ["#/import", "#view-import"],
    ["#/diktatverlauf", "#view-diktatverlauf"],
    ["#/einstellungen", "#view-einstellungen"],
    ["#/hilfe", "#view-hilfe"],
  ];
  for (const [route, sel] of screensToAudit) {
    await page.goto(url + route);
    await page.waitForTimeout(120);
    const problems = await auditInteractive(sel);
    check(`leaf-G2 ${route} interactive elements ≥44px + named` +
          (problems.length ? ` (${problems.slice(0, 3).join("; ")})` : ""),
          problems !== null && problems.length === 0);
  }
  check("leaf-G4 diktieren context page keeps the no-meeting-start rule",
        await page.evaluate(() =>
          !/Meeting starten/.test(document.querySelector("#view-diktieren").textContent)));

  // Meeting-Detail inkl. geöffnetem Aktionsmenü
  await page.goto(url + "#/meetings");
  await page.waitForTimeout(100);
  await page.click("#meeting-list li .card-open");
  await page.waitForTimeout(120);
  await page.click("#detail-more-menu summary");
  await page.waitForTimeout(60);
  const detailProblems = await auditInteractive("#view-meeting-detail");
  check("leaf-G2 #/meeting-detail (menu open) elements ≥44px + named" +
        (detailProblems.length ? ` (${detailProblems.slice(0, 3).join("; ")})` : ""),
        detailProblems.length === 0);

  // Laufendansicht inkl. Stop-Bestätigung und Countdown-Overlay
  await page.goto(url + "#/meetings");
  await page.waitForTimeout(100);
  await page.click("#btn-neues-meeting");
  await page.waitForTimeout(60);
  const dlgProblems = await auditInteractive("#start-dialog");
  check("leaf-G2 start dialog elements ≥44px + named" +
        (dlgProblems.length ? ` (${dlgProblems.slice(0, 3).join("; ")})` : ""),
        dlgProblems.length === 0);
  await page.click("#btn-meeting-starten");
  await page.waitForTimeout(200);
  const cdProblems = await auditInteractive("#countdown-overlay");
  check("leaf-G2 countdown overlay elements ≥44px + named", cdProblems.length === 0);
  await page.waitForTimeout(3200);
  const runProblems = await auditInteractive("#view-meeting-aktiv");
  check("leaf-G2 running view elements ≥44px + named" +
        (runProblems.length ? ` (${runProblems.slice(0, 3).join("; ")})` : ""),
        runProblems.length === 0);
  await page.click("#btn-beenden");
  await page.waitForTimeout(60);
  const scProblems = await auditInteractive("#stop-confirm");
  check("leaf-G2 stop confirmation elements ≥44px + named", scProblems.length === 0);
  await page.keyboard.press("Escape");

  // Sichtbarer Tastaturfokus: 12 Tab-Stops auf der Meetings-Seite müssen alle
  // eine erkennbare Outline zeigen (:focus-visible, Plan §6.14).
  await page.goto(url + "#/meetings");
  await page.waitForTimeout(120);
  let focusOk = true;
  const focusFails = [];
  for (let i = 0; i < 12; i++) {
    await page.keyboard.press("Tab");
    await page.waitForTimeout(20);
    const st = await page.evaluate(() => {
      const el = document.activeElement;
      if (!el || el === document.body) return null;
      const cs = getComputedStyle(el);
      return { tag: el.tagName.toLowerCase(), id: el.id,
               style: cs.outlineStyle, w: parseFloat(cs.outlineWidth),
               color: cs.outlineColor };
    });
    if (!st) continue;
    if (st.style === "none" || !(st.w > 0) || st.color === "rgba(0, 0, 0, 0)") {
      focusOk = false;
      focusFails.push(JSON.stringify(st));
    }
  }
  check("leaf-G2 keyboard focus visibly outlined at every Tab stop" +
        (focusFails.length ? ` (fails: ${focusFails.slice(0, 3).join(", ")})` : ""),
        focusOk);

  console.log(results.map(([s, n]) => `${s}  ${n}`).join("\n"));
  console.log(errors.length ? "\nJS ERRORS:\n" + errors.join("\n") : "\nNo JS errors.");
  if (errors.length) process.exitCode = 1;
  try { await browser.close(); } catch (_) {}
  await cleanup();
  // Über CDP angehängte Browser halten Node-Handles offen; hart beenden,
  // damit der Lauf definitiv endet (Exit-Code bleibt erhalten).
  process.exit(process.exitCode || 0);
})().catch(async (e) => {
  console.error("HARNESS ERROR:", e);
  if (jsErrors.length) {
    console.error("\nJS ERRORS seen before the failure:\n" + jsErrors.join("\n"));
  }
  if (results.length) {
    console.error("\nResults before the error:");
    console.error(results.map(([s, n]) => `${s}  ${n}`).join("\n"));
  }
  if (page) {
    try {
      const st = await page.evaluate(() => ({
        hash: location.hash,
        runningView: !document.getElementById("view-meeting-aktiv").hidden,
        startDialog: !document.getElementById("start-dialog").hidden,
        countdown: !document.getElementById("countdown-overlay").hidden,
        stopConfirm: !document.getElementById("stop-confirm").hidden,
        pinned: !document.getElementById("pinned-recording").hidden,
      }));
      console.error("\nPage state at failure: " + JSON.stringify(st));
    } catch (_) { /* page may be gone */ }
  }
  process.exit(1);
});
