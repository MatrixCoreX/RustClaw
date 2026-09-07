import assert from "node:assert/strict";
import fs from "node:fs/promises";
import test from "node:test";

import { chromium } from "playwright";

import { capturePlatformCaption } from "../src/browser.mjs";

const RUN_BROWSER_TEST = process.env.MEDIA_DISCOVERY_BROWSER_TEST === "1";

async function browserExecutable() {
  const configured = process.env.MEDIA_DISCOVERY_CHROME_BIN?.trim();
  const candidates = configured
    ? [configured]
    : process.platform === "darwin"
      ? [
          "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
          "/Applications/Chromium.app/Contents/MacOS/Chromium",
          "/opt/homebrew/bin/chromium",
          "/usr/local/bin/chromium",
        ]
      : [
          "/usr/bin/chromium",
          "/usr/bin/chromium-browser",
          "/usr/bin/google-chrome",
          "/usr/bin/google-chrome-stable",
          "/snap/bin/chromium",
        ];
  for (const candidate of candidates) {
    if (await fs.access(candidate).then(() => true).catch(() => false)) return candidate;
  }
  throw new Error("browser executable is required for the explicit caption integration test");
}

async function withPage(t, content) {
  const browser = await chromium.launch({ executablePath: await browserExecutable(), headless: true });
  t.after(() => browser.close());
  const page = await browser.newPage();
  await page.setContent(content);
  return page;
}

test("extracts a Douyin author caption without collecting adjacent controls", {
  skip: !RUN_BROWSER_TEST,
}, async (t) => {
  const page = await withPage(t, `
    <article data-e2e="detail-video-info">
      <h1>Una historia breve<br>#viaje</h1>
      <span data-e2e="video-player-digg">318</span>
      <time>2026-09-07</time>
    </article>
  `);

  const caption = await capturePlatformCaption(page, "douyin", "metadata fallback");
  assert.equal(caption, "Una historia breve\n#viaje");
});

test("extracts a Douyin feed caption from its structural marker", {
  skip: !RUN_BROWSER_TEST,
}, async (t) => {
  const page = await withPage(t, `
    <article data-aweme-id="123">
      <p data-e2e="feed-video-desc">مرحبا بالعالم</p>
      <button data-e2e="feed-comment-icon">86</button>
    </article>
  `);

  const caption = await capturePlatformCaption(page.locator("article"), "douyin");
  assert.equal(caption, "مرحبا بالعالم");
});

test("combines Xiaohongshu title and description as the author caption", {
  skip: !RUN_BROWSER_TEST,
}, async (t) => {
  const page = await withPage(t, `
    <article class="note-content">
      <h1 id="detail-title">週末散歩</h1>
      <p id="detail-desc">静かな午後。<br>#日記</p>
      <div class="like-wrapper"><span class="count">27</span></div>
    </article>
  `);

  const caption = await capturePlatformCaption(page, "xiaohongshu", "metadata fallback");
  assert.equal(caption, "週末散歩\n静かな午後。\n#日記");
});
