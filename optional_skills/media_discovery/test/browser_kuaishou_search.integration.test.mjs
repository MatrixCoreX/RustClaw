import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { chromium } from "playwright";
import { browserCapability, collectPlatform } from "../src/browser.mjs";
import { kuaishouSearchCards } from "../src/browser_kuaishou_search.mjs";
import { handleRequest } from "../src/main.mjs";
import { readRecords } from "../src/storage.mjs";

const enabled = process.env.MEDIA_DISCOVERY_BROWSER_TEST === "1";
const photos = [1, 2, 3].map(n => ({ id: `fixture000${n}`, coverUrl: `https://www.kuaishou.com/cover-${n}.svg`, timestamp: 1788220800000 }));

test("new Kuaishou searches and collects three card modals with isolated caption and counters", { skip: !enabled }, async t => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "discovery-kuaishou-search-"));
  t.after(() => fs.rm(root, { recursive: true, force: true }));
  const clicked = [];
  const original = chromium.launchPersistentContext.bind(chromium);
  t.mock.method(chromium, "launchPersistentContext", async (...args) => {
    const context = await original(...args);
    await context.exposeFunction("recordClick", index => clicked.push(index));
    await context.route("**/*", async route => {
      if (route.request().url().endsWith(".svg")) return route.fulfill({ contentType: "image/svg+xml", body:
        '<svg xmlns="http://www.w3.org/2000/svg" width="500" height="400"><rect width="500" height="400" fill="#167d87"/><text x="30" y="200" font-size="60" fill="white">video cover</text></svg>' });
      return route.fulfill({ contentType: "text/html", body: `<body>
        <div class="search-container"><div class="search"><input class="input"><div class="search-text">search</div></div></div>
        <div class="photo-btns"><div class="like-btn">999999</div></div>
        <main></main><script>
        const photos=${JSON.stringify(photos)};
        window.INIT_STATE={search:{feeds:photos.map(photo=>({photo}))}};
        document.querySelector('.search-text').onclick=()=>{
          history.pushState({},'', '/search/'+encodeURIComponent(document.querySelector('input').value)+'?source=NewReco');
          document.querySelector('main').innerHTML='<div class="video-list" style="display:flex;gap:10px">'+photos.map((p,i)=>
            '<div class="photo-card" style="width:300px"><div class="cover"><img class="cover-img" width="300" height="220" src="'+p.coverUrl+'"></div><div class="caption">card '+i+'</div></div>').join('')+'</div>';
          document.querySelectorAll('.photo-card .cover').forEach((node,i)=>node.onclick=async()=>{
            await window.recordClick(i);
            const modal=document.createElement('div');modal.className='swiper-feed';modal.style.cssText='position:fixed;inset:0;background:white';
            modal.innerHTML='<div class="swiper-slide-active"><video width="500" height="400" poster="'+photos[i].coverUrl+'"></video><div class="caption">post '+i+'</div><div class="photo-btns"><div class="like-btn">'+(10+i)+'</div><div class="commentPanel">0</div><div class="favorite">collect</div></div><button class="close circle-btn">close</button></div>';
            modal.querySelector('button').onclick=()=>modal.remove();document.body.append(modal);
          });
        };</script></body>` });
    });
    return context;
  });
  const result = await handleRequest({ args: { action: "run_once", platform: "kuaishou", source_mode: "topics", topics: ["财经"],
    max_items_per_run: 3, max_run_minutes: 5, browser_mode: "silent" },
  context: { skill_storage: { storage_kind: "directory", directory_path: root } } }, {
    collectPlatform: request => collectPlatform({ ...request, config: { ...request.config, pacing_min_delay_ms: 1, pacing_max_delay_ms: 1 } }),
  });
  assert.equal(result.status, "ok", JSON.stringify(result));
  assert.deepEqual(clicked, [0, 1, 2]);
  assert.equal(result.extra.run.counts.videos, 3);
  assert.equal(result.extra.run.counts.failures, 0);
  const records = await readRecords(root);
  assert.deepEqual(records.map(r => r.platform_text), ["post 0", "post 1", "post 2"]);
  assert.deepEqual(records.map(r => r.video_page_url), photos.map(p => `https://www.kuaishou.com/short-video/${p.id}`));
  for (const [i, record] of records.entries()) {
    assert.equal(record.search_keyword, "财经");
    assert.equal(record.engagement.metrics.likes.value, 10 + i);
    assert.equal(record.engagement.metrics.comments.value, 0);
    assert.equal(record.engagement.metrics.favorites, undefined);
    assert.equal(record.published_at, "2026-09-01T00:00:00.000Z");
    assert.ok((await fs.stat(path.join(root, "exports", record.cover_screenshot_path))).size > 512);
  }
});

test("Kuaishou card identity requires a unique loaded post for its rendered cover", { skip: !enabled }, async t => {
  const browser = await chromium.launch({ executablePath: (await browserCapability()).chromium_executable, headless: true });
  t.after(() => browser.close());
  const page = await browser.newPage();
  await page.route("**/*", route => route.abort());
  await page.setContent(`<div class="video-list"><div class="photo-card" style="width:300px;height:240px"><img class="cover-img" src="${photos[0].coverUrl}"></div></div>`);
  await page.evaluate(photos => { window.INIT_STATE={search:{feeds:photos.map(photo=>({photo}))}}; }, photos);
  assert.deepEqual(await kuaishouSearchCards(page), [{ index: 0, itemId: photos[0].id }]);
  await page.evaluate(photo => window.INIT_STATE.search.feeds.push({photo:{...photo,id:"another0000"}}), photos[0]);
  assert.deepEqual(await kuaishouSearchCards(page), []);
});
