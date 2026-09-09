import assert from "node:assert/strict";
import test from "node:test";
import { chromium } from "playwright";
import { browserCapability } from "../src/browser.mjs";
import { capturePublication, normalizePublication } from "../src/publication.mjs";
import { IMAGE_COLUMNS, VIDEO_COLUMNS, renderCsv } from "../src/csv.mjs";

test("publication timestamps preserve precision without borrowing collection time", () => {
  assert.equal(normalizePublication(1788739200), "2026-09-07T00:00:00.000Z");
  assert.equal(normalizePublication(1788739200000), "2026-09-07T00:00:00.000Z");
  assert.equal(normalizePublication("2026-09-07"), "2026-09-07");
  assert.equal(normalizePublication("2026-09-07T08:00:00+08:00"), "2026-09-07T00:00:00.000Z");
  for (const value of [null, "", "2天前", "昨天", "09-07", "2026-02-30", 0, "2030-01-01T00:00:00Z"]) {
    assert.equal(normalizePublication(value), null);
  }
});

test("both CSVs retain source date separately from discovery time", () => {
  for (const columns of [IMAGE_COLUMNS, VIDEO_COLUMNS]) {
    const csv = renderCsv(columns, [{ published_at: "2026-09-07", discovered_at: "2026-09-09T00:00:00Z", publication_source: "dom_attribute" }]);
    assert.ok(columns.includes("publication_text"));
    assert.match(csv, /2026-09-07/);
    assert.match(csv, /2026-09-09T00:00:00Z/);
    assert.match(csv, /dom_attribute/);
  }
});

test("publication extraction is scoped to the current post across all platforms", {
  skip: process.env.MEDIA_DISCOVERY_BROWSER_TEST !== "1",
}, async (t) => {
  const browser = await chromium.launch({ executablePath: (await browserCapability()).chromium_executable, headless: true });
  t.after(() => browser.close());
  const page = await browser.newPage();
  await page.setContent(`<article data-aweme-id="other"><time datetime="2020-01-01">wrong post</time></article>
    <article data-aweme-id="123"><time datetime="2026-09-07">date</time>
    <div data-comment-id="comment"><time datetime="2021-01-01">comment</time></div></article>`);
  assert.equal((await capturePublication(page, "douyin", "douyin:123")).published_at, "2026-09-07");
  await page.setContent('<article><div class="bottom-container"><span class="date">2天前</span></div></article>');
  const relative = await capturePublication(page, "xiaohongshu", "123");
  assert.equal(relative.published_at, null);
  assert.equal(relative.publication_text, "2天前");
  await page.setContent('<article><time style="display:none" datetime="2021-01-01"></time><p>2026-09-07 in caption</p></article>');
  assert.equal((await capturePublication(page, "kuaishou", "123")).published_at, null);
  assert.equal((await capturePublication(page, "kuaishou", "123")).publication_text, null);
  for (const [platform, key] of [["douyin", "aweme_id"], ["xiaohongshu", "noteId"], ["kuaishou", "photoId"]]) {
    await page.setContent(`<script type="application/json">${JSON.stringify({ posts: [
      { [key]: "wrong", createTime: 1700000000000 },
      { [key]: "123", createTime: 1788739200000 },
    ] })}</script>`);
    assert.equal((await capturePublication(page, platform, "123")).published_at, "2026-09-07T00:00:00.000Z");
    assert.equal((await capturePublication(page, platform, "missing")).published_at, null);
  }
  await page.setContent('<script type="application/ld+json">{"url":"https://www.douyin.com/video/123","datePublished":"2026-09-07","dateModified":"2026-09-08"}</script>');
  assert.equal((await capturePublication(page, "douyin", "123")).published_at, "2026-09-07");
  assert.equal((await capturePublication(page, "douyin", "12")).published_at, null);
});
