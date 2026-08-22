// Verification harness for prototype/meeting-mode (Gates G1, G2).
// Run from workspace root:  node prototype\meeting-mode\verify.cjs
// Uses the repo's Playwright package with the locally installed Chromium build.
"use strict";
const path = require("path");
const fs = require("fs");
const { chromium } = require(path.join(__dirname, "..", "..", "node_modules", "playwright"));

const exeRoot = path.join(process.env.LOCALAPPDATA, "ms-playwright");
const candidates = [
  ["chromium_headless_shell-1234", "chrome-headless-shell-win64", "chrome-headless-shell.exe"],
  ["chromium-1234", "chrome-win64", "chrome.exe"],
].map((parts) => path.join(exeRoot, ...parts));
const exe = candidates.find((p) => fs.existsSync(p) && fs.statSync(p).isFile());
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

  console.log(results.map(([s, n]) => `${s}  ${n}`).join("\n"));
  console.log(errors.length ? "\nJS ERRORS:\n" + errors.join("\n") : "\nNo JS errors.");
  if (errors.length) process.exitCode = 1;
  await browser.close();
  await cleanup();
})().catch((e) => { console.error("HARNESS ERROR:", e); process.exit(1); });
