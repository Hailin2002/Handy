import { chromium } from "@playwright/test";

const browser = await chromium.launch();
const page = await browser.newPage();
page.on("console", (msg) => console.log(`[console:${msg.type()}]`, msg.text()));
page.on("pageerror", (err) => console.log("[pageerror]", err.message));
page.on("requestfailed", (req) =>
  console.log("[requestfailed]", req.url(), req.failure()?.errorText),
);
await page.addInitScript(() => {
  window.__SPEAK_E2E_MOCK_CONFIG__ = {};
});
let injected = 0;
const marker = /<script type="module" src="\/src\/main\.tsx(\?[^"]*)?"?><\/script>/;
await page.route("**/*", async (route) => {
  if (route.request().resourceType() !== "document") {
    await route.continue();
    return;
  }
  const response = await route.fetch();
  const html = await response.text();
  if (!marker.test(html)) {
    await route.fulfill({ response });
    return;
  }
  injected += 1;
  await route.fulfill({
    response,
    contentType: "text/html; charset=utf-8",
    body: html.replace(marker, (entry) => `<script type="module" src="/src/mocks/install.ts"></script>${entry}`),
  });
});
try {
  await page.goto("http://localhost:1420/", { timeout: 20000 });
  console.log("goto OK");
} catch (e) {
  console.log("goto FAILED:", String(e).split("\n")[0]);
}
console.log("injected:", injected);
await page.waitForTimeout(1500);
const state = await page.evaluate(() => ({
  hasInternals: !!window.__TAURI_INTERNALS__,
  hasBridge: !!window.__SPEAK_E2E_MOCK_BRIDGE__,
}));
console.log("state:", JSON.stringify(state));
const body = await page.evaluate(() => document.body.innerHTML.slice(0, 600));
console.log("=== BODY ===");
console.log(body);
await browser.close();
