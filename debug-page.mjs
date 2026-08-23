import { chromium } from "@playwright/test";

const browser = await chromium.launch();
const page = await browser.newPage();
page.on("console", (msg) => console.log(`[console:${msg.type()}]`, msg.text()));
page.on("pageerror", (err) => console.log("[pageerror]", err.message));
page.on("requestfailed", (req) =>
  console.log("[requestfailed]", req.url(), req.failure()?.errorText),
);
page.on("response", (res) => {
  if (res.url().includes("mocks")) {
    console.log("[response]", res.status(), res.url());
  }
});
await page.addInitScript(() => {
  window.__SPEAK_E2E_MOCK_CONFIG__ = {};
});
let docRoutes = 0;
let injected = 0;
await page.route("**/*", async (route) => {
  if (route.request().resourceType() !== "document") {
    await route.continue();
    return;
  }
  docRoutes += 1;
  let response;
  try {
    response = await route.fetch();
  } catch (err) {
    console.log("[route] fetch failed:", String(err));
    await route.abort();
    return;
  }
  const html = await response.text();
  const marker = /<script type="module" src="\/src\/main\.tsx(\?[^"]*)?"?><\/script>/;
  console.log(
    "[route] doc",
    route.request().url(),
    "marker:",
    marker.test(html),
    "len:",
    html.length,
  );
  if (!marker.test(html)) {
    await route.fulfill({ response });
    return;
  }
  injected += 1;
  const body = html.replace(
    marker,
    '<script type="module" src="/src/mocks/install.ts"></script><script type="module" src="/src/main.tsx"></script>',
  );
  console.log(
    "[route] injected body contains tag:",
    body.includes("/src/mocks/install.ts"),
  );
  await route.fulfill({
    response,
    contentType: "text/html; charset=utf-8",
    body,
  });
});
await page.goto("http://localhost:1420/");
await page.waitForTimeout(3000);
console.log("docRoutes:", docRoutes, "injected:", injected);
const state = await page.evaluate(() => ({
  hasInternals: !!window.__TAURI_INTERNALS__,
  hasBridge: !!window.__SPEAK_E2E_MOCK_BRIDGE__,
}));
console.log("state:", JSON.stringify(state));
const html = await page.content();
const idx = html.indexOf("<body");
console.log("=== BODY SNIPPET ===");
console.log(html.slice(idx, idx + 1200));
await browser.close();
