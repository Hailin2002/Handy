import { chromium } from "@playwright/test";
const browser = await chromium.launch();
const page = await browser.newPage();
await page.route("**/*", async (route) => {
  if (route.request().resourceType() !== "document") { await route.continue(); return; }
  const response = await route.fetch();
  const html = await response.text();
  console.log("=== FETCHED DOC ===");
  console.log(JSON.stringify(html));
  await route.fulfill({ response });
});
await page.goto("http://localhost:1420/");
await browser.close();
