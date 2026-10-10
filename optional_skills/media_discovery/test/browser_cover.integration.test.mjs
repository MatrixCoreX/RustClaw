import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import { chromium } from "playwright";

import {
  captureToutiaoPreview,
  renderedVideoCover,
  scopeHasVideo,
  screenshotLooksBlank,
} from "../src/browser.mjs";

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

test("Douyin transparent click catchers and side like buttons do not mask the frame", {
  skip: !RUN_BROWSER_TEST,
}, async (t) => {
  const page = await withPage(t, `
    <main style="position:relative;width:640px;height:360px">
      <video style="width:640px;height:360px;background:#222"></video>
      <div style="position:absolute;inset:0;background:transparent;z-index:2"></div>
      <button style="position:absolute;right:8px;top:40%;width:48px;height:80px;z-index:3;background:#111">like</button>
    </main>
  `);
  assert.equal((await renderedVideoCover(page, "douyin"))?.source, "rendered_video_frame");
});

test("Douyin player like and caption chrome do not mask the rendered frame", {
  skip: !RUN_BROWSER_TEST,
}, async (t) => {
  const page = await withPage(t, `
    <main data-e2e="video-player" style="position:relative;width:640px;height:360px">
      <video style="width:640px;height:360px;background:#222"></video>
      <button data-e2e="video-like-icon" style="position:absolute;right:8px;top:40%;width:48px;height:80px;z-index:3"></button>
      <p data-e2e="video-desc" style="position:absolute;left:0;right:80px;bottom:0;height:72px;z-index:3">caption</p>
    </main>
  `);
  assert.equal((await renderedVideoCover(page, "douyin"))?.source, "rendered_video_frame");
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

test("Weibo recognizes a loading video post and captures its scoped poster under player chrome", {
  skip: !RUN_BROWSER_TEST,
}, async (t) => {
  const page = await withPage(t, `
    <article style="position:relative;width:640px;height:420px">
      <a href="https://video.weibo.com/show?fid=1034:5351513040814119">微博视频</a>
      <div class="video-js" style="position:relative;width:640px;height:360px">
        <picture class="vjs-poster"><img alt="" style="width:640px;height:360px"
          src="data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='640' height='360'%3E%3Crect width='640' height='360' fill='navy'/%3E%3C/svg%3E"></picture>
        <div class="vjs-control-bar" style="position:absolute;inset:0;z-index:2;background:transparent"></div>
      </div>
    </article>
  `);
  const article = page.locator("article");
  assert.equal(await scopeHasVideo(article, "weibo"), true);
  const cover = await renderedVideoCover(article, "weibo");
  assert.equal(cover?.source, "rendered_poster_image");
});

test("TikTok and YouTube player chrome still allows a scoped video cover", {
  skip: !RUN_BROWSER_TEST,
}, async (t) => {
  const page = await withPage(t, `
    <main>
      <section data-e2e="browse-video" style="position:relative;width:300px;height:500px">
        <video style="width:300px;height:500px;background:#16324f"></video>
        <div style="position:absolute;inset:0;z-index:2;background:transparent"></div>
      </section>
      <div id="movie_player" style="position:relative;width:640px;height:360px">
        <video class="html5-main-video" style="width:640px;height:360px;background:#31572c"></video>
        <div class="ytp-chrome-bottom" style="position:absolute;left:0;right:0;bottom:0;height:50px;z-index:2"></div>
      </div>
    </main>
  `);
  assert.equal((await renderedVideoCover(page.locator('[data-e2e="browse-video"]'), "tiktok"))?.source,
    "rendered_video_frame");
  assert.equal((await renderedVideoCover(page.locator("#movie_player"), "youtube"))?.source,
    "rendered_video_frame");
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

test("a uniform black player screenshot is rejected as a blank cover", {
  skip: !RUN_BROWSER_TEST,
}, async (t) => {
  const page = await withPage(t, `
    <main style="width:800px;height:600px;background:#111"></main>
  `);
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "discovery-blank-cover-"));
  t.after(() => fs.rm(root, { recursive: true, force: true }));
  const target = path.join(root, "cover.png");
  await page.locator("main").screenshot({ path: target, type: "png" });
  assert.equal(await screenshotLooksBlank(target), true);
});

test("Toutiao text-only articles use the complete article surface as their preview", {
  skip: !RUN_BROWSER_TEST,
}, async (t) => {
  const page = await withPage(t, `
    <main>
      <section class="article-content" style="width:640px;min-height:420px;padding:24px;background:white;color:#222">
        <h1 style="height:48px">Article title</h1>
        <p>First paragraph of the exact article body.</p>
        <p>Second paragraph retained in the same preview.</p>
      </section>
    </main>
  `);
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "discovery-toutiao-preview-"));
  t.after(() => fs.rm(root, { recursive: true, force: true }));

  const preview = await captureToutiaoPreview(
    page,
    root,
    "run-article-preview",
    "toutiao:123456",
    "",
    1,
  );

  assert.match(preview.relativePath || "", /^images\//u);
  const output = path.join(root, "exports", ...preview.relativePath.split("/"));
  assert.equal(await screenshotLooksBlank(output), false);
});
