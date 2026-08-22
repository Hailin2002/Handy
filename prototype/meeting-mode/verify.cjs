// Verification harness for prototype/meeting-mode (Gates G1, G2).
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

(async () => {
  const { browser, cleanup } = await launchBrowser();
  const page = await browser.newPage();
  const errors = [];
  page.on("pageerror", (e) => errors.push("pageerror: " + e.message));
  page.on("console", (m) => { if (m.type() === "error") errors.push("console: " + m.text()); });

  const results = [];
  const check = (name, ok) => { results.push([ok ? "PASS" : "FAIL", name]); if (!ok) process.exitCode = 1; };

  await page.goto(url);
  await page.waitForTimeout(300);

  // G1: navigation between the five areas
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
    check(`G1 nav: ${route} -> ${viewId} visible`, visible);
  }
  // exactly one view visible at a time
  await page.goto(url + "#/projekte");
  await page.waitForTimeout(100);
  const visCount = await page.evaluate(() =>
    [...document.querySelectorAll(".view")].filter((v) => !v.hidden).length);
  check("G1 only one view visible at a time", visCount === 1);

  // aria-current follows route
  const cur = await page.getAttribute('.nav-item[data-route="projekte"]', "aria-current");
  check("aria-current set on active nav item", cur === "page");

  // G2: pinned entry hidden while no session
  check("G2 pinned entry hidden initially",
        await page.evaluate(() => document.getElementById("pinned-recording").hidden === true));

  // Start a mocked session via the start dialog
  await page.goto(url + "#/meetings");
  await page.waitForTimeout(100);
  await page.click("#btn-neues-meeting");
  check("start dialog opens", await page.evaluate(() => !document.getElementById("start-dialog").hidden));
  await page.click("#btn-meeting-starten");

  // G2: pinned entry visible with red dot and running timer while session active
  await page.waitForTimeout(1300);
  const pinned = await page.evaluate(() => {
    const pin = document.getElementById("pinned-recording");
    return {
      hidden: pin.hidden,
      label: pin.querySelector(".pinned-label").textContent,
      dotColor: getComputedStyle(pin.querySelector(".rec-dot")).backgroundColor,
      timer: pin.querySelector(".pinned-time").textContent,
      positionAfterMeetings:
        pin.previousElementSibling.querySelector(".nav-item").dataset.route === "meetings",
    };
  });
  check("G2 pinned entry visible during session", !pinned.hidden);
  check("G2 pinned label 'Aufnahme läuft'", pinned.label === "Aufnahme läuft");
  check("G2 red status dot (rgb(179,38,30))", pinned.dotColor === "rgb(179, 38, 30)");
  console.log("pinned:", JSON.stringify(pinned));
  check("G2 timer counts up (>00:00:00)", /^00:00:0[1-9]|00:00:[1-9]\d$/.test(pinned.timer));
  check("G2 pinned entry sits directly under Meetings", pinned.positionAfterMeetings);

  // Pinned entry navigates to running-meeting view
  await page.click(".nav-pinned");
  await page.waitForTimeout(100);
  check("pinned entry navigates to meeting-aktiv view",
        await page.evaluate(() => !document.getElementById("view-meeting-aktiv").hidden));

  // Stop flow: confirm dialog, then processing checklist, pin gone
  await page.click("#btn-beenden");
  check("stop confirmation shown", await page.evaluate(() => !document.getElementById("stop-confirm").hidden));
  await page.click("#btn-stop-ok");
  await page.waitForTimeout(200);
  check("pin hidden after stop",
        await page.evaluate(() => document.getElementById("pinned-recording").hidden === true));
  check("processing view reached",
        await page.evaluate(() => !document.getElementById("view-verarbeitung").hidden));
  await page.waitForTimeout(3600);
  check("checklist completes",
        await page.evaluate(() =>
          document.querySelector('#proc-checklist li[data-step="summary"]').className === "done"));
  await page.goto(url + "#/meetings");
  await page.waitForTimeout(200);
  check("finished meeting appears in list",
        await page.evaluate(() => document.querySelectorAll("#meeting-list .meeting-item").length >= 3));

  // Settings tabs work
  await page.goto(url + "#/einstellungen");
  await page.waitForTimeout(100);
  await page.click('.tab[data-tab="modelle"]');
  check("settings tab switch works",
        await page.evaluate(() => {
          const p = document.querySelector('.tabpanel[data-panel="modelle"]');
          return !p.hidden && document.querySelector('.tab[data-tab="modelle"]').getAttribute("aria-selected") === "true";
        }));

  // Escape closes start dialog without starting
  await page.goto(url + "#/meetings");
  await page.waitForTimeout(100);
  await page.click("#btn-neues-meeting");
  await page.keyboard.press("Escape");
  check("Escape closes start dialog",
        await page.evaluate(() => document.getElementById("start-dialog").hidden === true));

  /* ===== task-0178 Blatt: §6.6 Ersteinrichtung + §6.7 Startdialog ===== */

  // G4: setup screen navigable from prototype scaffold
  await page.goto(url + "#/meetings");
  await page.waitForTimeout(100);
  await page.click("#btn-open-setup");
  await page.waitForTimeout(100);
  check("G4 setup view reachable from Meetings scaffold",
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

  // §6.6 audio test with live-level mocks
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

  // §6.6 understandable correction action on missing device
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

  // §6.6 shortcut candidate availability
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

  // §6.6 finish lands directly in compact start dialog
  await page.click("#btn-setup-finish");
  await page.waitForTimeout(150);
  check("§6.6 finish opens compact start dialog",
        await page.evaluate(() => !document.getElementById("start-dialog").hidden));

  // G1: all four status rows clickable, scoped picker, dialog state preserved
  const titleBefore = await page.inputValue("#meeting-title-input");
  const rows = ["mic", "sys", "provider", "lang"];
  for (const row of rows) {
    await page.click('#row-' + row);
    await page.waitForTimeout(50);
    const open = await page.evaluate(() =>
      !document.getElementById("row-picker").hidden &&
      document.querySelectorAll("#row-picker .picker-option").length > 0);
    check(`G1 row '${row}' opens scoped picker`, open);
    await page.click("#row-picker .picker-back");
    await page.waitForTimeout(50);
    check(`G1 row '${row}' picker closes back to dialog`,
          await page.evaluate(() =>
            document.getElementById("row-picker").hidden === true &&
            !document.getElementById("start-dialog").hidden));
  }
  check("G1 dialog state preserved across pickers (title unchanged)",
        (await page.inputValue("#meeting-title-input")) === titleBefore);

  // Picker selection updates row without closing dialog state
  await page.click("#row-mic");
  await page.locator("#row-picker .picker-option", { hasText: "USB-Headset" }).click();
  await page.waitForTimeout(50);
  check("G1 picking a device updates its row live",
        /USB-Headset/.test(await page.textContent("#row-mic-value")));
  check("G1 dialog still open after pick",
        await page.evaluate(() => !document.getElementById("start-dialog").hidden));

  // G2: 'Kein Projekt' selectable and visibly distinct
  await page.click("#project-chip");
  await page.waitForTimeout(50);
  check("G2 project picker opens from chip",
        await page.evaluate(() => !document.getElementById("project-picker").hidden));
  const noneOpt = page.locator("#project-picker .picker-none-opt");
  check("G2 'Kein Projekt' option present and distinct (dashed)",
        (await noneOpt.count()) === 1 &&
        (await noneOpt.evaluate((el) => getComputedStyle(el).borderStyle)) === "dashed");
  await page.locator("#project-picker .picker-option", { hasText: "BDNA Club" }).click();
  check("G2 selecting a project reflects on chip",
        (await page.textContent("#project-chip")).trim() === "BDNA Club");
  // Picker bleibt nach einer Auswahl geöffnet (scoped Mini-Picker) – direkt
  // „Kein Projekt“ wählen, ohne den Chip zu klicken (der würde ihn schließen).
  await noneOpt.click();
  await page.waitForTimeout(50);
  check("G2 'Kein Projekt' selectable again, chip visibly unassigned",
        (await page.textContent("#project-chip")).trim() === "Kein Projekt" &&
        (await page.getAttribute("#project-chip", "class")).includes("chip-none"));

  // G3: start disabled for mocked missing device
  await page.click("#row-mic");
  await page.getByRole("button", { name: /Simulation: Gerät entfernen/ }).click();
  await page.waitForTimeout(50);
  check("G3 start disabled while mic missing",
        await page.evaluate(() => document.getElementById("btn-meeting-starten").disabled));
  check("G3 blocker names the fix location",
        await page.evaluate(() => {
          const b = document.getElementById("start-blocker");
          return !b.hidden && /Mikrofon/.test(b.textContent);
        }));
  check("G3 row shows attention state",
        /Aufmerksamkeit/.test(await page.textContent("#row-mic-value")));
  // fix inside the same picker without losing dialog
  await page.getByRole("button", { name: /Simulation zurücksetzen|Standardmikrofon/ }).first().click();
  await page.waitForTimeout(50);
  check("G3 fixing device re-enables start, dialog intact",
        await page.evaluate(() =>
          !document.getElementById("btn-meeting-starten").disabled &&
          !document.getElementById("start-dialog").hidden));

  // G3: start disabled for cloud provider without key
  await page.click("#row-provider");
  await page.locator("#row-picker .picker-option", { hasText: "Azure Cloud" }).click();
  await page.waitForTimeout(50);
  check("G3 azure without key disables start",
        await page.evaluate(() => document.getElementById("btn-meeting-starten").disabled));
  await page.getByRole("button", { name: /Azure-Schlüssel hinzufügen/ }).click();
  await page.waitForTimeout(50);
  check("G3 adding key re-enables start",
        await page.evaluate(() => !document.getElementById("btn-meeting-starten").disabled));
  await page.getByRole("button", { name: /^Lokal auf diesem Gerät/ }).click();

  // Escape first closes only the picker; second press closes the dialog
  await page.click("#row-lang");
  await page.keyboard.press("Escape");
  await page.waitForTimeout(50);
  check("Escape closes picker but keeps dialog open",
        await page.evaluate(() =>
          document.getElementById("row-picker").hidden === true &&
          !document.getElementById("start-dialog").hidden));
  await page.keyboard.press("Escape");
  await page.waitForTimeout(50);
  check("second Escape closes dialog",
        await page.evaluate(() => document.getElementById("start-dialog").hidden === true));

  console.log(results.map(([s, n]) => `${s}  ${n}`).join("\n"));
  console.log(errors.length ? "\nJS ERRORS:\n" + errors.join("\n") : "\nNo JS errors.");
  if (errors.length) process.exitCode = 1;
  await browser.close();
  await cleanup();
})().catch((e) => { console.error("HARNESS ERROR:", e); process.exit(1); });
