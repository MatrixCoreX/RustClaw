import assert from "node:assert/strict";
import { mkdir, readFile } from "node:fs/promises";
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
  let galleryMode = false;
  let filterMode = false;
  const itemRequests = [];
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
    const cursor = Number(url.searchParams.get("cursor_sequence")) || null;
    const platform = url.searchParams.get("platform");
    const filtered = filterMode ? items.filter(item => !platform || item.platform === platform)
      .sort((a, b) => url.searchParams.get("sort_order") === "oldest"
        ? a.global_sequence - b.global_sequence : b.global_sequence - a.global_sequence) : items;
    const start = cursor == null ? 0 : filtered.findIndex(item => item.global_sequence === cursor) + 1;
    const size = filterMode ? 20 : 2;
    const selected = galleryMode ? filtered.slice(start, start + size) : filtered;
    const nextCursor = galleryMode && start + size < filtered.length ? selected.at(-1).global_sequence : null;
    if (url.pathname.endsWith("/items")) itemRequests.push(url);
    return route.fulfill({ json: { ok: true, data: url.pathname === "/v1/aipps" ? { apps: [{
      skill_name: "example_collection", package_version: "1.0.0", renderer: "collection_feed_v1",
      data_contract: "media_collection_v1", icon: "gallery_vertical_end", default_locale: "en",
      titles: { en: "Collection", zh: "采集内容" }, descriptions: {}, installed: true,
      entrypoint: null, bridge_capabilities: [], task_channel_scope: null,
    }] } : { items: selected, matching_total: filtered.length, sort_order: url.searchParams.get("sort_order") || "newest", next_cursor_sequence: nextCursor,
      next_before_sequence: null, active_run: null, platform_states: filterMode ? { douyin: {}, xiaohongshu: {}, kuaishou: {} } : {}, updated_at: "2026-09-09T00:00:00Z" } } });
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
  galleryMode = true;
  const template = { ...items[1], platform: "example", title: "Gallery / 图集", platform_text: "One post, five retained images" };
  items.splice(0, items.length, ...Array.from({ length: 5 }, (_, i) => ({
    ...template, global_sequence: 10 - i, image_sequence: 5 - i, post_sequence: 10,
  })), { ...template, global_sequence: 5, kind: "video", post_sequence: null },
  { ...template, global_sequence: 4, post_sequence: null });
  for (const width of [1440, 390]) for (const theme of ["light", "dark"]) {
    const lang = theme === "light" ? "zh" : "en";
    await page.setViewportSize({ width, height: 900 });
    await page.goto(`http://127.0.0.1:${server.httpServer.address().port}/test/fixtures/aipp-collection.html?lang=${lang}&theme=${theme}`);
    const cards = page.locator("article");
    const card = cards.first();
    await card.getByText("1 / 5", { exact: true }).waitFor();
    assert.equal(await cards.count(), 3);
    assert.equal(await card.locator("h2").count(), 1);
    for (let i = 0; i < 2; i++) await card.getByTitle(lang === "zh" ? "下一张图片" : "Next image", { exact: true }).click();
    await card.getByText("3 / 5", { exact: true }).waitFor();
    await card.locator("img").waitFor();
    await card.getByRole("button", { name: lang === "zh" ? "放大图片" : "Enlarge image", exact: true }).click();
    const dialog = page.getByRole("dialog");
    await dialog.waitFor();
    const downloadEvent = page.waitForEvent("download");
    await dialog.locator("footer button").click();
    const download = await downloadEvent;
    assert.equal(download.suggestedFilename(), "media-000000000008.png");
    assert.deepEqual(await readFile(await download.path()), preview);
    await page.keyboard.press("Escape");
    await page.getByTitle(lang === "zh" ? "刷新内容" : "Refresh content", { exact: true }).click();
    await card.getByText("3 / 5", { exact: true }).waitFor();
    assert.equal(await cards.count(), 3);
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
    await page.screenshot({ path: path.join(output, `gallery-${width}-${theme}.png`), fullPage: true });
    assert.equal(await page.getByRole("button", { name: lang === "zh" ? "下一页" : "Next", exact: true }).isDisabled(), true);
    assert.equal(await page.getByText("#5", { exact: true }).count(), 1);
    assert.equal(await page.getByText("#4", { exact: true }).count(), 1);
    console.log(`PASS gallery ${width} ${theme}: complete pagination, one card, image switch, download, refresh`);
  }
  filterMode = true;
  items.splice(0, items.length,
    ...Array.from({ length: 40 }, (_, i) => ({ ...template, platform: "xiaohongshu", global_sequence: 100 - i, post_sequence: 50, image_sequence: 40 - i })),
    ...Array.from({ length: 30 }, (_, i) => ({ ...template, platform: ["kuaishou", "douyin", "xiaohongshu"][Math.floor(i / 10)],
      global_sequence: 60 - i, post_sequence: 49 - i, image_sequence: 1 })));
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    await page.goto(`http://127.0.0.1:${server.httpServer.address().port}/test/fixtures/aipp-collection.html?lang=zh&theme=light`);
    const waitCards = count => page.waitForFunction(expected => document.querySelectorAll("article").length === expected
      && document.querySelector('[aria-busy="false"]'), count);
    await waitCards(20);
    for (const name of ["xiaohongshu", "kuaishou", "douyin"]) assert.ok((await page.locator("article").allTextContents()).some(text => text.includes(name)));
    const select = page.getByRole("combobox", { name: "按平台筛选" });
    await select.selectOption("xiaohongshu");
    await waitCards(11);
    assert.ok((await page.locator("article").allTextContents()).every(text => text.includes("xiaohongshu")));
    itemRequests.length = 0;
    await select.selectOption("all");
    await waitCards(20);
    assert.ok(itemRequests.length >= 3);
    assert.ok(itemRequests.every(url => !url.searchParams.has("platform")));
    await page.getByRole("button", { name: "下一页", exact: true }).click();
    await waitCards(11);
    await page.getByText("第 2 页 · 11 篇", { exact: true }).waitFor();
    await select.selectOption("kuaishou");
    await waitCards(10);
    await page.getByText("第 1 页 · 10 篇", { exact: true }).waitFor();
    assert.ok((await page.locator("article").allTextContents()).every(text => text.includes("kuaishou")));
    await select.selectOption("all");
    await waitCards(20);
    await page.getByRole("combobox", { name: "按采集时间排序" }).selectOption("oldest");
    await page.getByText("#31", { exact: true }).waitFor();
    await page.getByRole("button", { name: "下一页", exact: true }).click();
    await waitCards(11);
    await page.getByText("1 / 40", { exact: true }).waitFor();
    assert.equal(await page.getByRole("button", { name: "下一页", exact: true }).isDisabled(), true);
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
    await page.screenshot({ path: path.join(output, `platform-pagination-${width}.png`), fullPage: true });
    console.log(`PASS all-platform ${width}: post-based pages, all three platforms, filter reset, both time orders`);
  }
  assert.deepEqual(errors, []);
  console.log(`PASS collection browser suite; screenshots: ${output}`);
} finally {
  await browser?.close();
  await server.close();
}
