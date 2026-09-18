import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { chromium } from "playwright";
import { browserCapability, collectPlatform } from "../src/browser.mjs";
import {
  clickKuaishouSearchCard,
  collectKuaishouSearchResults,
  kuaishouSearchCards,
} from "../src/browser_kuaishou_search.mjs";
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
            '<div class="photo-card" style="width:300px;height:240px"><img class="cover-img" width="300" height="220" src="'+p.coverUrl+'"><div class="caption">card '+i+'</div></div>').join('')+'</div>';
          document.querySelectorAll('.photo-card img.cover-img').forEach((node,i)=>node.onclick=async()=>{
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
    assert.equal(record.cover_capture_source, "search_tile");
    assert.ok((await fs.stat(path.join(root, "exports", record.cover_screenshot_path))).size > 512);
  }
});

test("Kuaishou search cards click the cover image when the cover wrapper is absent", { skip: !enabled }, async t => {
  const browser = await chromium.launch({ executablePath: (await browserCapability()).chromium_executable, headless: true });
  t.after(() => browser.close());
  const page = await browser.newPage();
  await page.route("**/*", route => {
    if (route.request().url().endsWith(".svg")) {
      return route.fulfill({
        contentType: "image/svg+xml",
        body: '<svg xmlns="http://www.w3.org/2000/svg" width="500" height="400"><rect width="500" height="400" fill="#167d87"/></svg>',
      });
    }
    return route.abort();
  });
  await page.setContent(`<div class="video-list"><div class="photo-card" style="width:300px;height:240px;position:relative">
    <div class="cover" style="position:absolute;inset:0;pointer-events:none"></div>
    <img class="cover-img" width="300" height="220" src="${photos[0].coverUrl}"></div></div>`);
  await page.evaluate(photos => { window.INIT_STATE={search:{feeds:photos.map(photo=>({photo}))}}; }, photos);
  const clicked = [];
  await page.exposeFunction("recordClick", selector => clicked.push(selector));
  await page.locator("img.cover-img").evaluate(node => {
    node.onclick = () => window.recordClick("cover-img");
  });
  const card = page.locator(".video-list .photo-card").first();
  const tile = await card.locator("img.cover-img").screenshot({ type: "png" });
  assert.equal(await clickKuaishouSearchCard(card), "img.cover-img:visible");
  assert.deepEqual(clicked, ["cover-img"]);
  assert.ok(tile.length > 512);
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

test("empty Kuaishou search results wait after opening the load-more login modal", { skip: !enabled }, async t => {
  const browser = await chromium.launch({ executablePath: (await browserCapability()).chromium_executable, headless: true });
  t.after(() => browser.close());
  const page = await browser.newPage();
  await page.route("**/*", route => route.abort());
  await page.setContent(`<div class="video-list"><div class="loading-more">
    <span class="login-link" style="display:inline-block;width:80px;height:20px">login</span></div></div>
    <script>document.querySelector(".login-link").onclick=()=>{
      const popup=document.createElement("div");
      popup.className="popup login-popup";
      popup.innerHTML='<div class="login-modal login-modal-v2" style="width:120px;height:120px"></div>';
      document.body.append(popup);
    }</script>`);
  const checks = [];
  await assert.rejects(collectKuaishouSearchResults({
    page,
    config: { max_scrolls_per_source: 0, pacing_min_delay_ms: 1, pacing_max_delay_ms: 1 },
    limit: 1,
    shouldStop: async () => false,
    checkAccess: async () => {
      const blocked = await page.locator(".login-popup .login-modal:visible").count() ? "login_required" : null;
      checks.push(blocked);
      return blocked;
    },
    settle: async () => {},
    scrollPage: async () => {},
    collect: async () => { throw new Error("must not collect"); },
    onPage: async () => {},
    onFailure: async () => {},
    detailed: true,
  }), error => error.message === "login_required" && error.discovery_stage === "search_results_access");
  assert.ok(checks.includes("login_required"));
  assert.equal(await page.locator(".login-popup .login-modal:visible").count(), 1);
});
