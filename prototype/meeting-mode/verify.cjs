// Verification harness for prototype/meeting-mode.
// Covers the task-0179 leaf gates (G1 stop confirmation, G2 scroll lock,
// G3 §6.8 colour semantics, G4 flows from start-dialog completion) plus
// regression checks for task-0178 (§6.6 setup, §6.7 start dialog).
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
let page = null;

(async () => {
  const { browser, cleanup } = await launchBrowser();
  page = await browser.newPage();
  const errors = [];
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
