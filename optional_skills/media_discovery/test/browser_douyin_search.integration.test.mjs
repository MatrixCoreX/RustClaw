import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { chromium } from "playwright";
import { collectPlatform, screenshotLooksBlankFromBytes } from "../src/browser.mjs";
import { handleRequest } from "../src/main.mjs";
import { readRecords } from "../src/storage.mjs";

const enabled = process.env.MEDIA_DISCOVERY_BROWSER_TEST === "1";
const posts = [1, 2, 3].map((n) => ({
  id: `768528349455820913${n}`,
  coverUrl: `https://www.douyin.com/cover-${n}.svg`,
}));

test("Douyin jingxuan searches collect video-image cards through modal_id overlays", { skip: !enabled }, async (t) => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "discovery-douyin-search-"));
  t.after(() => fs.rm(root, { recursive: true, force: true }));
  const clicked = [];
  const original = chromium.launchPersistentContext.bind(chromium);
  t.mock.method(chromium, "launchPersistentContext", async (...args) => {
    const context = await original(...args);
    await context.exposeFunction("recordClick", (index) => clicked.push(index));
    await context.route("**/*", async (route) => {
      if (route.request().url().endsWith(".svg")) {
        return route.fulfill({
          contentType: "image/svg+xml",
          body: '<svg xmlns="http://www.w3.org/2000/svg" width="500" height="400"><rect width="500" height="400" fill="#167d87"/><text x="30" y="200" font-size="60" fill="white">video cover</text></svg>',
        });
      }
      return route.fulfill({
        contentType: "text/html",
        body: `<head>
        <title>发现更多精彩视频 - 抖音搜索</title>
        <link rel="canonical" href="https://www.douyin.com/jingxuan/search/%E8%B4%A2%E7%BB%8F?modal_id=9999999999999999999">
        <meta property="og:title" content="wrong leftover og title">
        <meta property="og:url" content="https://www.douyin.com/video/9999999999999999999">
        </head><body>
        <p data-e2e="feed-video-desc" style="width:200px;height:40px">wrong leftover title</p>
        <input data-e2e="searchbar-input">
        <div data-e2e="searchbar-button">search</div>
        <main></main>
        <script>
        const posts=${JSON.stringify(posts)};
        document.querySelector('[data-e2e="searchbar-button"]').onclick=()=>{
          const keyword=document.querySelector('[data-e2e="searchbar-input"]').value;
          history.pushState({},'', '/jingxuan/search/'+encodeURIComponent(keyword)+'?type=general');
          document.querySelector('main').innerHTML='<div style="display:flex;gap:10px">'+posts.map((p)=>
            '<div class="search-result-card" style="width:240px;height:320px"><div class="videoImage" style="width:240px;height:320px"><img width="240" height="320" src="'+p.coverUrl+'"></div><p data-e2e="video-desc">wrong card '+p.id+'</p></div>'
          ).join('')+'</div>';
          document.querySelectorAll('.search-result-card .videoImage').forEach((node,i)=>node.onclick=async()=>{
            await window.recordClick(i);
            const url=new URL(location.href);
            url.searchParams.set('modal_id', posts[i].id);
            history.pushState({},'', url);
            document.getElementById('overlay')?.remove();
            const modal=document.createElement('div');
            modal.id='overlay';
            modal.style.cssText='position:fixed;inset:0;background:white;z-index:9';
            modal.innerHTML='<video width="500" height="400"></video><h1 data-e2e="video-desc">author title '+i+'</h1><div data-e2e="video-like-count">'+(10+i)+'</div>';
            document.body.append(modal);
          });
        };
        document.addEventListener('keydown', event=>{
          if (event.key!=='Escape') return;
          const url=new URL(location.href);
          url.searchParams.delete('modal_id');
          history.pushState({},'', url);
          document.getElementById('overlay')?.remove();
        });
        </script></body>`,
      });
    });
    return context;
  });
  const result = await handleRequest({
    args: {
      action: "run_once",
      platform: "douyin",
      source_mode: "topics",
      topics: ["财经"],
      max_items_per_run: 3,
      max_run_minutes: 5,
      browser_mode: "silent",
    },
    context: { skill_storage: { storage_kind: "directory", directory_path: root } },
  }, {
    collectPlatform: (request) => collectPlatform({
      ...request,
      config: { ...request.config, pacing_min_delay_ms: 1, pacing_max_delay_ms: 1 },
    }),
  });
  assert.equal(result.status, "ok", JSON.stringify(result));
  assert.deepEqual(clicked, [0, 1, 2]);
  assert.equal(result.extra.run.counts.videos, 3);
  assert.equal(result.extra.run.counts.failures, 0);
  const records = await readRecords(root);
  assert.deepEqual(records.map((record) => record.title), ["author title 0", "author title 1", "author title 2"]);
  assert.deepEqual(records.map((record) => record.platform_text), ["", "", ""]);
  assert.deepEqual(records.map((record) => record.item_id), posts.map((post) => `douyin:${post.id}`));
  assert.deepEqual(
    records.map((record) => record.video_page_url),
    posts.map((post) => `https://www.douyin.com/video/${post.id}`),
  );
  for (const [index, record] of records.entries()) {
    assert.equal(record.search_keyword, "财经");
    assert.equal(record.engagement.metrics.likes.value, 10 + index);
    const coverBytes = await fs.readFile(path.join(root, "exports", record.cover_screenshot_path));
    assert.equal(screenshotLooksBlankFromBytes(coverBytes), false);
    assert.ok(coverBytes.length > 512);
    assert.equal(record.cover_capture_source, "search_tile");
  }
});
