import assert from "node:assert/strict";
import fs from "node:fs/promises";
import test from "node:test";

import { chromium } from "playwright";

import { renderedVideoCover } from "../src/browser.mjs";

const RUN_BROWSER_TEST = process.env.MEDIA_DISCOVERY_BROWSER_TEST === "1";

async function browserExecutable() {
  const candidates = process.env.MEDIA_DISCOVERY_CHROME_BIN?.trim()
    ? [process.env.MEDIA_DISCOVERY_CHROME_BIN.trim()]
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
  throw new Error("browser executable is required for the explicit cover integration test");
}

async function withPage(t, content) {
  const browser = await chromium.launch({ executablePath: await browserExecutable(), headless: true });
  t.after(() => browser.close());
  const page = await browser.newPage({ viewport: { width: 900, height: 700 } });
  await page.setContent(content);
  return page;
}

test("video cover selection prefers an unobscured rendered frame", {
  skip: !RUN_BROWSER_TEST,
}, async (t) => {
  const page = await withPage(t, '<main><video style="width:640px;height:360px;background:#222"></video></main>');
  const cover = await renderedVideoCover(page, "douyin");
  assert.equal(cover?.source, "rendered_video_frame");
  assert.equal(await cover?.locator.evaluate((node) => node.tagName), "VIDEO");
});

test("video cover selection refuses a media element hidden by an overlay", {
  skip: !RUN_BROWSER_TEST,
}, async (t) => {
  const page = await withPage(t, `
    <main style="position:relative;width:640px;height:360px">
      <video style="width:640px;height:360px;background:#222"></video>
      <section style="position:absolute;inset:0;z-index:2;background:white">overlay</section>
    </main>
  `);
  assert.equal(await renderedVideoCover(page.locator("main"), "douyin"), null);
});

test("platform poster controls provide the cover when no video frame is available", {
  skip: !RUN_BROWSER_TEST,
}, async (t) => {
  const page = await withPage(t, `
    <main data-e2e="video-cover" style="width:640px;height:360px">
      <img alt="" style="width:640px;height:360px" src="data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='640' height='360'%3E%3Crect width='640' height='360' fill='navy'/%3E%3C/svg%3E">
    </main>
  `);
  const cover = await renderedVideoCover(page, "douyin");
  assert.equal(cover?.source, "rendered_poster_image");
});

test("Kuaishou player controls do not mask the frame but unrelated overlays still do", {
  skip: !RUN_BROWSER_TEST,
}, async (t) => {
  const page = await withPage(t, `
    <main class="swiper-feed"><section class="swiper-slide-active">
      <div class="video-container" style="position:relative;width:640px;height:360px">
        <video class="kplayer-video" style="width:640px;height:360px;background:#222"></video>
        <div class="video-interact-panel" style="position:absolute;inset:0"><div class="mask" style="height:100%"></div></div>
        <div class="volume-control-wrapper" style="position:absolute;bottom:0;right:0;width:200px;height:90px">controls</div>
      </div>
    </section></main>
  `);
  assert.equal((await renderedVideoCover(page, "kuaishou"))?.source, "rendered_video_frame");
  await page.locator(".video-container").evaluate(node => {
    const overlay = document.createElement("div");
    overlay.style.cssText = "position:absolute;inset:0;background:white";
    node.append(overlay);
  });
  assert.equal(await renderedVideoCover(page, "kuaishou"), null);
  await page.locator(".video-container > div:last-child").evaluate(node => node.remove());
  await page.locator("body").evaluate(node => {
    const overlay = document.createElement("div");
    overlay.className = "video-interact-panel";
    overlay.style.cssText = "position:fixed;inset:0;background:white";
    node.append(overlay);
  });
  assert.equal(await renderedVideoCover(page, "kuaishou"), null);
});

test("Douyin discovery cards expose their rendered poster as the video cover", {
  skip: !RUN_BROWSER_TEST,
}, async (t) => {
  const page = await withPage(t, `
    <article data-aweme-id="123456789" style="position:relative;width:340px;height:280px">
      <img class="discover-video-card-img" alt="caption" style="width:340px;height:190px" src="data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='340' height='190'%3E%3Crect width='340' height='190' fill='navy'/%3E%3C/svg%3E">
      <span data-play-control style="position:absolute;inset:0 0 90px;z-index:2"></span>
    </article>
  `);
  const cover = await renderedVideoCover(page.locator("article"), "douyin");
  assert.equal(cover?.source, "rendered_poster_image");
});

test("Kuaishou feed cards expose their rendered poster as the video cover", {
  skip: !RUN_BROWSER_TEST,
}, async (t) => {
  const page = await withPage(t, `
    <article class="video-card" style="width:160px;height:280px">
      <div class="poster"><img class="poster-img" alt="" style="width:160px;height:240px" src="data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='160' height='240'%3E%3Crect width='160' height='240' fill='green'/%3E%3C/svg%3E"></div>
    </article>
  `);
  const cover = await renderedVideoCover(page.locator("article"), "kuaishou");
  assert.equal(cover?.source, "rendered_poster_image");
});

test("Xiaohongshu feed cards allow their structural play control over the poster", {
  skip: !RUN_BROWSER_TEST,
}, async (t) => {
  const page = await withPage(t, `
    <section class="note-item" data-note-id="abc123" style="position:relative;width:227px;height:375px">
      <a class="cover" style="display:block;width:227px;height:303px">
        <img alt="" style="width:227px;height:303px" src="data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='227' height='303'%3E%3Crect width='227' height='303' fill='red'/%3E%3C/svg%3E">
        <span class="play-icon" style="position:absolute;inset:0 0 72px;z-index:2"></span>
      </a>
    </section>
  `);
  const cover = await renderedVideoCover(page.locator("section"), "xiaohongshu");
  assert.equal(cover?.source, "rendered_poster_image");
});
