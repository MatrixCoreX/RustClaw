import assert from "node:assert/strict";
import { mkdir } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { createServer } from "vite";

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || "playwright");
const root = fileURLToPath(new URL("../", import.meta.url));
const output = process.env.UI_BROWSER_ARTIFACTS || "/tmp/aipp-catalog-tests";
await mkdir(output, { recursive: true });
const server = await createServer({ root, server: { host: "127.0.0.1", port: 0, strictPort: true } });
let browser;
try {
  await server.listen();
  const base = `http://127.0.0.1:${server.httpServer.address().port}`;
  browser = await chromium.launch({ executablePath: process.env.BROWSER_EXECUTABLE || undefined, headless: true });
  const page = await browser.newPage();
  const errors = [], requests = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.route("https://fonts.googleapis.com/**", (route) => route.abort());
  await page.route("**/v1/**", (route) => { requests.push(route.request().method()); return route.fulfill({ status: 500 }); });
  for (const width of [320, 390, 768, 1440]) {
    await page.setViewportSize({ width, height: 900 });
    for (const theme of ["light", "dark"]) for (const lang of ["zh", "en"]) {
      await page.goto(`${base}/test/fixtures/aipp-catalog.html?lang=${lang}&theme=${theme}`);
      const grid = page.getByTestId("aipp-catalog-grid");
      await grid.waitFor();
      assert.equal(await grid.locator("button").count(), 7);
      const icons = await grid.getByTestId("aipp-launcher-icon").evaluateAll((elements) => elements.map((element) => {
        const box = element.getBoundingClientRect(), style = getComputedStyle(element);
        return { x: box.x, right: box.right, width: box.width, height: box.height, radius: style.borderTopLeftRadius };
      }));
      assert.equal(icons.length, 7);
      for (const icon of icons) {
        assert.ok(Math.abs(icon.width - icon.height) < 1, `Icon must be square: ${JSON.stringify(icon)}`);
        assert.ok(icon.width >= 64 && icon.width <= 97);
        assert.equal(icon.radius, "24%");
        assert.ok(icon.x >= 0 && icon.right <= width);
      }
      assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
      assert.equal(await grid.locator("button").evaluateAll((buttons) => buttons.every((button) => button.scrollWidth <= button.clientWidth + 1)), true);
      const open = grid.locator("button").first(), install = grid.locator("button").last();
      assert.match(await open.getAttribute("aria-label"), lang === "zh" ? /^打开/ : /^Open/);
      assert.match(await install.getAttribute("aria-label"), lang === "zh" ? /^安装 Ai APP/ : /^Install Ai APP/);
      assert.equal(await open.getAttribute("title"), lang === "zh" ? "查看已保存的内容。" : "View saved content.");
      await open.click();
      assert.equal(await page.evaluate(() => document.documentElement.dataset.openedApp), "example_app_0");
      await install.focus();
      await page.keyboard.press("Enter");
      assert.equal(await page.evaluate(() => document.documentElement.dataset.installedApp), "example_app_6");
      if (lang === "zh") await page.screenshot({ path: path.join(output, `catalog-${width}-${theme}.png`), fullPage: true });
      console.log(`PASS ${width}px ${theme} ${lang}: square icons, no overflow, open/install, keyboard, localized labels`);
    }
  }
  assert.deepEqual(errors, []);
  assert.deepEqual(requests, []);
} finally {
  await browser?.close();
  await server.close();
}
