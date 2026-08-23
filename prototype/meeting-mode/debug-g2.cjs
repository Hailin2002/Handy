// Debug: replicate the G2 suggestion flow with diagnostics.
"use strict";
const path = require("path");
const fs = require("fs");
const { chromium } = require(path.join(__dirname, "..", "..", "node_modules", "playwright"));

function findChromium() {
  const root = path.join(process.env.LOCALAPPDATA, "ms-playwright");
  const dir = fs.readdirSync(root).filter((d) => d.startsWith("chromium")).sort().pop();
  return path.join(root, dir, "chrome-headless-shell-win64", "chrome-headless-shell.exe");
}

(async () => {
  const browser = await chromium.launch({
    executablePath: findChromium(),
    headless: true,
    args: ["--no-sandbox"],
  });
  const page = await browser.newPage();
  const errors = [];
  page.on("pageerror", (e) => errors.push("pageerror: " + e.message));
  page.on("console", (m) => { if (m.type() === "error") errors.push("console: " + m.text()); });

  const url = "file:///" + path.join(__dirname, "index.html").replace(/\\/g, "/");
  await page.goto(url + "#/meetings");
  await page.waitForTimeout(150);

  await page.click("#btn-neues-meeting");
  await page.waitForTimeout(60);
  console.log("chip:", (await page.textContent("#project-chip")).trim());
  console.log("start disabled:", await page.evaluate(() => document.getElementById("btn-meeting-starten").disabled));
  console.log("blocker:", await page.evaluate(() => document.getElementById("start-blocker").textContent));
  await page.click("#btn-meeting-starten");
  await page.waitForTimeout(3300);
  console.log("hash after countdown:", await page.evaluate(() => location.hash));
  await page.click("#btn-beenden");
  await page.waitForTimeout(50);
  await page.click("#btn-stop-ok");
  // Poll the suggestion card every 250ms
  for (let i = 0; i < 24; i++) {
    await page.waitForTimeout(250);
    const st = await page.evaluate(() => ({
      t: Date.now(),
      summary: document.querySelector('#proc-checklist li[data-step="summary"]').className,
      suggHidden: document.getElementById("ai-suggestion").hidden,
      hl: document.getElementById("suggestion-headline").textContent.slice(0, 50),
      label: document.getElementById("proc-meeting-label").textContent,
      hash: location.hash,
    }));
    console.log(i, JSON.stringify(st));
    if (!st.suggHidden) break;
  }
  console.log("errors:", errors);
  await browser.close();
})().catch((e) => { console.error("DEBUG ERROR:", e); process.exit(1); });
