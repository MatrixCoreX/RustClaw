import assert from "node:assert/strict";
import test from "node:test";
import { chromium } from "playwright";
import { browserCapability } from "../src/browser.mjs";
import { openKeywordSearch, normalizeBrowserError } from "../src/browser_search.mjs";
import { platformSpec, sourceTargets } from "../src/platforms.mjs";

const enabled = process.env.MEDIA_DISCOVERY_BROWSER_TEST === "1";
const keyword = "财经";
const controls = {
  douyin: '<input data-e2e="searchbar-input"><button data-e2e="searchbar-button">search</button>',
  xiaohongshu: '<input id="search-input" hidden><div class="textarea-container"><textarea id="search-input-in-feeds"></textarea><button class="submit-button-wrapper">search</button></div>',
  kuaishou: '<input class="search-input"><button class="search-button">search</button>',
};

async function pageFixture(t, platform, { popup = false, noSubmit = false, json = false, delay = 0, wrongKeyword = false, newKuaishou = false } = {}) {
  const browser = await chromium.launch({ executablePath: (await browserCapability()).chromium_executable, headless: true });
  t.after(() => browser.close());
  const context = await browser.newContext();
  const page = await context.newPage();
  const source = sourceTargets(platform, { source_mode: "topics", topics: [keyword] })[0];
  const target = platform === "xiaohongshu"
    ? `https://www.xiaohongshu.com/search_result_ai?keyword=${encodeURIComponent(encodeURIComponent(keyword))}` : source.url;
  const resultTarget = wrongKeyword ? target.replace(/%E8%B4%A2%E7%BB%8F|%25E8%25B4%25A2%25E7%25BB%258F/u, "other") : target;
  const submitted = [];
  const navigations = [];
  await page.exposeFunction("recordSearch", value => submitted.push(value));
  await context.route("**/*", async route => {
    const url = route.request().url();
    navigations.push(url);
    const markup = newKuaishou ? '<div class="search-container"><div class="search"><input class="input"><input class="input-mini" hidden><div class="search-text">search</div></div></div>' : controls[platform];
    if (url === platformSpec(platform).homeUrl) return route.fulfill({ contentType: "text/html", body: `${markup}
      <a href="https://www.douyin.com/video/99999999">recommendation must not qualify</a>
      <script>document.querySelector('button,.search-text').onclick=async()=>{
        const input=document.querySelector('textarea')||document.querySelector('input');
        await window.recordSearch(input.value);
        ${noSubmit ? "" : popup ? `window.open(${JSON.stringify(resultTarget)})` : `location.href=${JSON.stringify(resultTarget)}`};
      }</script>` });
    assert.equal(url, resultTarget);
    if (json) return route.fulfill({ contentType: "application/json", body: '{"result":2}' });
    const link = { douyin: "https://www.douyin.com/video/12345678", xiaohongshu: "https://www.xiaohongshu.com/explore/12345678", kuaishou: "https://www.kuaishou.com/short-video/12345678" }[platform];
    return route.fulfill({ contentType: "text/html", body: `<body><script>setTimeout(()=>{document.body.innerHTML='<a href="${link}">result</a>'},${delay})</script></body>` });
  });
  return { page, source, submitted, navigations, target: resultTarget };
}

test("new Kuaishou search uses its visible full-size input and search text control", { skip: !enabled }, async t => {
  const f = await pageFixture(t, "kuaishou", { newKuaishou: true });
  const result = await openKeywordSearch(f.page, "kuaishou", f.source, { timeoutMs: 4000 });
  assert.deepEqual(f.submitted, [keyword]);
  assert.equal(result.page.url(), f.target);
});

for (const platform of Object.keys(controls)) {
  test(`${platform} types the keyword and clicks search before consuming results`, { skip: !enabled }, async t => {
    const f = await pageFixture(t, platform, { popup: platform === "kuaishou", delay: 300 });
    const result = await openKeywordSearch(f.page, platform, f.source, { timeoutMs: 4000 });
    assert.deepEqual(f.submitted, [keyword]);
    assert.deepEqual(f.navigations, [platformSpec(platform).homeUrl, f.target]);
    assert.equal(result.source.url, f.target);
    assert.equal(result.source.search_keyword, keyword);
    assert.equal(result.page.url(), f.target);
    assert.equal(result.page === f.page, platform !== "kuaishou");
  });
}

test("an unsubmitted search never qualifies on recommendation cards", { skip: !enabled }, async t => {
  const f = await pageFixture(t, "douyin", { noSubmit: true });
  await assert.rejects(openKeywordSearch(f.page, "douyin", f.source, { timeoutMs: 600 }), { message: "search_not_submitted" });
  assert.deepEqual(f.submitted, [keyword]);
});

test("a new search tab returning JSON fails structurally", { skip: !enabled }, async t => {
  const f = await pageFixture(t, "kuaishou", { popup: true, json: true });
  await assert.rejects(openKeywordSearch(f.page, "kuaishou", f.source, { timeoutMs: 3000 }), { message: "unexpected_page_response" });
});

test("results for a different keyword are never accepted", { skip: !enabled }, async t => {
  const f = await pageFixture(t, "xiaohongshu", { wrongKeyword: true });
  await assert.rejects(openKeywordSearch(f.page, "xiaohongshu", f.source, { timeoutMs: 600 }), { message: "search_not_submitted" });
});

test("manual barriers and cancellation stop before entering a query", { skip: !enabled }, async t => {
  const f = await pageFixture(t, "douyin");
  await assert.rejects(openKeywordSearch(f.page, "douyin", f.source, { shouldStop: async () => true }), { message: "collection_stopped" });
  await assert.rejects(openKeywordSearch(f.page, "douyin", f.source, { checkAccess: async () => "challenge_required" }), { message: "challenge_required" });
  assert.deepEqual(f.submitted, []);
});

test("typed timeouts are normalized without inspecting message prose", () => {
  const error = Object.assign(new Error("任意错误文字"), { name: "TimeoutError" });
  assert.equal(normalizeBrowserError(error, "search_submit").message, "browser_timeout");
  assert.equal(normalizeBrowserError(new Error("TimeoutError in plain text"), "search_submit").message, "TimeoutError in plain text");
});
