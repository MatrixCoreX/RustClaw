import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import { chromium } from "playwright";

import { collectRenderedImages, discoverCandidates } from "../src/browser.mjs";

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
  throw new Error("browser executable is required for the explicit carousel integration test");
}

function fixtureImage(label, background, accent) {
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="640" height="480">
    <rect width="640" height="480" fill="${background}"/>
    <circle cx="150" cy="180" r="90" fill="${accent}"/>
    <rect x="280" y="90" width="250" height="210" rx="18" fill="${accent}" opacity="0.75"/>
    <text x="60" y="410" font-family="sans-serif" font-size="64" fill="white">${label}</text>
  </svg>`;
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`;
}

test("browser collector follows the rendered carousel and captures every image in order", {
  skip: !RUN_BROWSER_TEST,
}, async (t) => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "media-discovery-carousel-"));
  t.after(() => fs.rm(root, { recursive: true, force: true }));
  const browser = await chromium.launch({
    executablePath: await browserExecutable(),
    headless: true,
  });
  t.after(() => browser.close());
  const page = await browser.newPage({ viewport: { width: 900, height: 700 } });
  const images = [
    fixtureImage("IMAGE 1", "#b42318", "#f79009"),
    fixtureImage("IMAGE 2", "#175cd3", "#12b76a"),
    fixtureImage("IMAGE 3", "#7a5af8", "#ee46bc"),
  ];
  await page.setContent(`
    <main style="width:640px;margin:20px auto">
      <img id="slide" width="640" height="480" alt="fixture carousel image">
      <button class="swiper-button-next" aria-label="next">next</button>
    </main>
    <script>
      const sources = ${JSON.stringify(images)};
      const image = document.querySelector("#slide");
      const next = document.querySelector(".swiper-button-next");
      let index = 0;
      image.src = sources[index];
      next.addEventListener("click", () => {
        index += 1;
        image.src = sources[index];
        if (index === sources.length - 1) {
          next.disabled = true;
          next.classList.add("swiper-button-disabled");
        }
      });
    </script>
  `);
  await page.locator("#slide").waitFor({ state: "visible" });

  const result = await collectRenderedImages({
    scope: page,
    root,
    runId: "carousel-fixture",
    platform: "xiaohongshu",
    itemId: "xiaohongshu:fixture",
    title: "fixture",
    platformText: "",
    sourcePageUrl: "https://www.xiaohongshu.com/explore/fixture",
    discoverySource: {
      source_mode: "topics",
      search_keyword: "fixture keyword",
      url: "https://www.xiaohongshu.com/search_result?keyword=fixture%20keyword",
    },
    config: { max_images_per_post: 100, recognition_mode: "metadata_only" },
    discoveredAt: "2026-08-10T00:00:00.000Z",
  });

  assert.equal(result.records.length, 3);
  assert.deepEqual(result.records.map((record) => record.image_sequence), [1, 2, 3]);
  assert.deepEqual(result.records.map((record) => record.search_keyword), [
    "fixture keyword",
    "fixture keyword",
    "fixture keyword",
  ]);
  assert.equal(
    result.records.every((record) => record.discovery_source_url.includes("search_result")),
    true,
  );
  assert.equal(new Set(result.records.map((record) => record.image_url)).size, 3);
  assert.equal(result.records.some((record) => record.collection_truncated), false);
  for (const screenshotPath of result.temporaryPaths) {
    assert.equal((await fs.stat(screenshotPath)).size > 512, true);
  }
});

test("browser search results discover ordered platform detail candidates without page-language matching", {
  skip: !RUN_BROWSER_TEST,
}, async (t) => {
  const browser = await chromium.launch({
    executablePath: await browserExecutable(),
    headless: true,
  });
  t.after(() => browser.close());
  const page = await browser.newPage({ viewport: { width: 900, height: 700 } });
  await page.setContent(`
    <main>
      <a href="https://www.xiaohongshu.com/explore/first-item">first</a>
      <a href="https://example.com/explore/not-allowed">external</a>
      <a href="https://www.xiaohongshu.com/explore/second-item?source=fixture">second</a>
      <a href="https://www.xiaohongshu.com/explore/first-item">duplicate</a>
    </main>
  `);

  const candidates = await discoverCandidates(
    page,
    "xiaohongshu",
    "https://www.xiaohongshu.com/search_result?keyword=fixture",
    0,
    10,
    async () => false,
    { pacing_min_delay_ms: 1, pacing_max_delay_ms: 1 },
  );

  assert.deepEqual(candidates, [
    "https://www.xiaohongshu.com/explore/first-item",
    "https://www.xiaohongshu.com/explore/second-item?source=fixture",
  ]);
});
