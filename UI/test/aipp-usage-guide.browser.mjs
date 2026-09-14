import assert from "node:assert/strict";
import { mkdir } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { createServer } from "vite";
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || "playwright");
const root = fileURLToPath(new URL("../", import.meta.url));
const output = process.env.UI_BROWSER_ARTIFACTS || "/tmp/aipp-guide-tests";
await mkdir(output, { recursive: true });
const server = await createServer({ root, server: { host: "127.0.0.1", port: 0, strictPort: true } });
let browser;
try {
  await server.listen();
  const base = `http://127.0.0.1:${server.httpServer.address().port}`;
  browser = await chromium.launch({ executablePath: process.env.BROWSER_EXECUTABLE || undefined, headless: true });
  const page = await browser.newPage();
  const errors = [], requests = [];
  page.on("pageerror", error => errors.push(error.message));
  await page.route("**/v1/**", route => { requests.push(route.request().method()); return route.fulfill({ status: 500 }); });
  await page.addInitScript(() => { Object.defineProperty(navigator, "clipboard", { value: { writeText: async value => { window.copiedExample = value; } } }); });
  for (const viewport of [{ width: 1440, height: 900 }, { width: 390, height: 844 }, { width: 480, height: 320 }]) {
    await page.setViewportSize(viewport);
    for (const lang of ["zh", "en"]) for (const kind of ["collection", "activity"]) {
      const theme = lang === "zh" ? "light" : "dark";
      await page.goto(`${base}/test/fixtures/aipp-usage-guide.html?lang=${lang}&theme=${theme}&kind=${kind}`);
      const guide = page.getByTestId("aipp-usage-guide");
      await guide.waitFor();
      if (kind === "collection") {
        await guide.getByLabel(lang === "zh" ? "要采集的平台" : "Platform to collect from", { exact: true }).fill("example-platform");
        await guide.getByLabel(lang === "zh" ? "搜索关键词（可选）" : "Search keywords (optional)", { exact: true }).fill("finance 科技");
        await guide.getByRole("button", { name: lang === "zh" ? "复制：按关键词搜索后采集" : "Copy: Search by keyword and collect", exact: true }).click();
        assert.match(await page.evaluate(() => window.copiedExample), /example-platform/);
        assert.match(await page.evaluate(() => window.copiedExample), /finance 科技/);
      } else {
        await guide.getByLabel(lang === "zh" ? "要处理的媒体链接" : "Media URL to process", { exact: true }).fill("https://example.test/media");
        await guide.getByRole("button", { name: lang === "zh" ? "复制：提取文字" : "Copy: Extract text", exact: true }).click();
        assert.match(await page.evaluate(() => window.copiedExample), /https:\/\/example.test\/media/);
      }
      await guide.getByRole("button", { name: lang === "zh" ? "已复制" : "Copied", exact: true }).waitFor();
      await guide.getByRole("button", { name: lang === "zh" ? "已复制" : "Copied", exact: true }).waitFor({ state: "detached" });
      assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
      assert.equal(await guide.locator("input, button").evaluateAll(nodes => nodes.every(el => { const box = el.getBoundingClientRect(); return box.x >= 0 && box.right <= innerWidth + 1 && el.scrollWidth <= el.clientWidth + 1; })), true);
      await page.screenshot({ path: path.join(output, `${kind}-${viewport.width}-${lang}.png`), fullPage: true });
      await guide.getByRole("button", { name: lang === "zh" ? "打开 Agent" : "Open Agent", exact: true }).click();
      assert.equal(await page.evaluate(() => document.documentElement.dataset.agentOpened), "true");
      console.log(`PASS ${kind} ${viewport.width} ${lang}: localized instructions, parameterized examples, copy reset, navigation, no overflow`);
    }
  }
  assert.deepEqual(requests, []); assert.deepEqual(errors, []);
  console.log("PASS guide is read-only: no task or capability requests");
} finally { await browser?.close(); await server.close(); }
