import assert from "node:assert/strict";
import { mkdir } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { createServer } from "vite";

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || "playwright");
const root = fileURLToPath(new URL("../", import.meta.url));
const output = process.env.UI_BROWSER_ARTIFACTS || path.join(os.tmpdir(), "aipp-collection-tests");
await mkdir(output, { recursive: true });
const server = await createServer({ root, server: { host: "127.0.0.1", port: 0, strictPort: true } });
let browser;
try {
  await server.listen();
  browser = await chromium.launch({ executablePath: process.env.BROWSER_EXECUTABLE || undefined, headless: true });
  const page = await browser.newPage();
  const errors = [];
  page.on("pageerror", e => errors.push(e.message));
  const items = Array.from({ length: 6 }, (_, i) => ({
    schema_version: 1, global_sequence: i + 1, sequence: i + 1, post_sequence: null,
    image_sequence: null, kind: i % 2 ? "image" : "video", platform: ["douyin", "xiaohongshu", "kuaishou"][i % 3],
    source_mode: "home_feed", search_keyword: "", title: "A post with a long title / 一条比较长的采集内容标题",
    platform_text: "原有文案保持完整，长文本不会与相邻条目重叠。\nOriginal caption, with long unbroken words: " + "sample".repeat(24),
    source_url: "https://example.test/post", image_url: null, preview_available: true,
    discovered_at: "2026-09-09T00:00:00Z", published_at: i === 0 ? "2026-09-01" : null,
    publication_text: i === 1 ? "2 days ago" : null,
    engagement: { schema_version: 1, platform: ["douyin", "xiaohongshu", "kuaishou"][i % 3],
      captured_at: "2026-09-09T00:00:00Z", metrics: i < 3 ? {
        likes: { display: "1.2万", value: null }, comments: { display: "318", value: 318 },
        favorites: { display: "0", value: 0 }, shares: { display: "24", value: 24 },
      } : {} },
  }));
  await page.goto("about:blank");
  const preview = Buffer.from(await page.evaluate(() => {
    const canvas = document.createElement("canvas"); canvas.width = 160; canvas.height = 120;
    const ctx = canvas.getContext("2d"); ctx.fillStyle = "#216558"; ctx.fillRect(0, 0, 160, 120);
    ctx.fillStyle = "#dce8ef"; ctx.fillRect(20, 20, 120, 80);
    return canvas.toDataURL("image/png").split(",")[1];
  }), "base64");
  await page.route("**/v1/aipps**", async route => {
    const url = new URL(route.request().url());
    if (url.pathname.endsWith("/preview")) return route.fulfill({ contentType: "image/png", body: preview });
    return route.fulfill({ json: { ok: true, data: url.pathname === "/v1/aipps" ? { apps: [{
      skill_name: "example_collection", package_version: "1.0.0", renderer: "collection_feed_v1",
      data_contract: "media_collection_v1", icon: "gallery_vertical_end", default_locale: "en",
      titles: { en: "Collection", zh: "采集内容" }, descriptions: {}, installed: true,
      entrypoint: null, bridge_capabilities: [], task_channel_scope: null,
    }] } : { items, matching_total: 6, sort_order: "newest", next_cursor_sequence: null,
      next_before_sequence: null, active_run: null, platform_states: {}, updated_at: "2026-09-09T00:00:00Z" } } });
  });
  for (const width of [1440, 1280, 900, 390]) for (const theme of ["light", "dark"]) {
    const lang = theme === "light" ? "zh" : "en";
    await page.setViewportSize({ width, height: 1000 });
    await page.goto(`http://127.0.0.1:${server.httpServer.address().port}/test/fixtures/aipp-collection.html?lang=${lang}&theme=${theme}`);
    await page.locator("article").nth(5).waitFor();
    await page.locator("article img").first().waitFor();
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
    const boxes = await page.locator("article").evaluateAll(nodes => nodes.map(node => {
      const r = node.getBoundingClientRect(); return { x: r.x, y: r.y, width: r.width, height: r.height, overflow: node.scrollWidth > node.clientWidth };
    }));
    const columns = width >= 1280 ? 3 : width >= 768 ? 2 : 1;
    assert.equal(boxes.filter(b => Math.abs(b.y - boxes[0].y) < 1).length, columns);
    assert.ok(boxes.every(b => !b.overflow && b.x >= 0 && b.x + b.width <= width));
    assert.ok(boxes[columns].y >= boxes[0].y + boxes[0].height);
    const text = await page.locator("article").first().innerText();
    assert.ok(text.includes("2026-09-01"));
    assert.ok(text.includes(lang === "zh" ? "采集：" : "Collected:"));
    assert.ok((await page.locator("article").nth(1).innerText()).includes("2 days ago"));
    assert.ok((await page.locator("article").nth(2).innerText()).includes(lang === "zh" ? "未提供" : "Unavailable"));
    assert.equal(await page.locator('article').nth(3).locator('[aria-label="Engagement at capture"], [aria-label="采集时互动数据"]').count(), 0);
    await page.screenshot({ path: path.join(output, `${width}-${theme}.png`), fullPage: true });
    const first = page.locator("article").first();
    await first.getByRole("button", { name: lang === "zh" ? "展开全文" : "Show all", exact: true }).click();
    assert.ok((await first.boundingBox()).height > boxes[0].height);
    await first.getByRole("button", { name: lang === "zh" ? "收起" : "Collapse", exact: true }).click();
    assert.ok(Math.abs((await first.boundingBox()).height - boxes[0].height) < 1);
    console.log(`PASS ${width} ${theme}: ${columns} columns, dates, metric omission, no overflow`);
  }
  assert.deepEqual(errors, []);
  console.log(`PASS collection browser suite; screenshots: ${output}`);
} finally {
  await browser?.close();
  await server.close();
}
