import assert from "node:assert/strict";
import test from "node:test";
import { chromium } from "playwright";
import { browserCapability } from "../src/browser.mjs";
import { dismissTikTokConsent, dismissYouTubeConsent, withSearchResult } from "../src/browser_search_results.mjs";

const enabled = process.env.MEDIA_DISCOVERY_BROWSER_TEST === "1";
const source = { url: "https://www.xiaohongshu.com/search_result?keyword=finance" };
const item = "https://www.xiaohongshu.com/search_result/12345678?source=fixture";

async function fixture(t, { popup = false, barrier = false } = {}) {
  const browser = await chromium.launch({ executablePath: (await browserCapability()).chromium_executable, headless: true });
  t.after(() => browser.close());
  const context = await browser.newContext();
  const page = await context.newPage();
  await context.route("**/*", route => route.fulfill({ contentType: "text/html", body: route.request().url() === source.url
    ? `<a href="${item}">post</a><script>
      document.querySelector('a').onclick=e=>{e.preventDefault();
        ${popup ? `window.open(${JSON.stringify(item)})` : barrier ? 'document.body.dataset.blocked="true"' : `history.pushState({},'', '/explore/12345678');document.body.dataset.detail="true"`};
      };
      onpopstate=()=>delete document.body.dataset.detail;
    </script>` : '<main data-detail="true">opened</main>' }));
  await page.goto(source.url);
  return { page, context };
}

test("SPA search details return to the same query without submitting another search", { skip: !enabled }, async t => {
  const { page } = await fixture(t);
  const result = await withSearchResult(page, "xiaohongshu", item.replace("source=fixture", "source=older-render"), source, async detail => {
    assert.equal(new URL(detail.url()).pathname, "/explore/12345678");
    assert.equal(await detail.locator('body[data-detail="true"]').count(), 1);
    return 42;
  }, { timeoutMs: 3000 });
  assert.equal(result, 42);
  assert.equal(page.url(), source.url);
  assert.equal(await page.locator('body[data-detail="true"]').count(), 0);
});

test("a detail popup is collected and closed without changing the search page", { skip: !enabled }, async t => {
  const { page, context } = await fixture(t, { popup: true });
  await withSearchResult(page, "xiaohongshu", item, source, async detail => {
    assert.notEqual(detail, page);
    assert.equal(await detail.locator('[data-detail="true"]').count(), 1);
  }, { timeoutMs: 3000 });
  assert.equal(context.pages().length, 1);
  assert.equal(page.url(), source.url);
});

test("TikTok search opens a validated canonical detail without clicking an obstructed card", {
  skip: !enabled,
}, async t => {
  const browser = await chromium.launch({ executablePath: (await browserCapability()).chromium_executable,
    headless: true });
  t.after(() => browser.close());
  const context = await browser.newContext();
  const page = await context.newPage();
  const searchUrl = "https://www.tiktok.com/search?q=technology";
  const detailUrl = "https://www.tiktok.com/@fixture/video/7512345678901234567";
  await context.route("**/*", route => route.fulfill({ contentType: "text/html", body:
    route.request().url() === searchUrl
      ? `<div style="position:fixed;inset:0;z-index:2"></div><a href="${detailUrl}">video</a>`
      : '<main data-detail="true">opened</main>' }));
  await page.goto(searchUrl);
  const result = await withSearchResult(page, "tiktok", detailUrl, { url: searchUrl }, async detail => {
    assert.equal(detail.url(), detailUrl);
    assert.equal(await detail.locator('[data-detail="true"]').count(), 1);
    return 84;
  }, { timeoutMs: 3000 });
  assert.equal(result, 84);
  assert.equal(page.url(), searchUrl);
});

test("a manual access barrier does not collect or navigate away from its prompt", { skip: !enabled }, async t => {
  const { page } = await fixture(t, { barrier: true });
  await assert.rejects(withSearchResult(page, "xiaohongshu", item, source,
    () => assert.fail("must not collect"), {
      timeoutMs: 3000,
      checkAccess: async candidate => await candidate.locator('[data-blocked="true"]').count() ? "login_required" : null,
    }), { message: "login_required" });
  assert.equal(await page.locator('[data-blocked="true"]').count(), 1);
});

test("YouTube consent selects the structural privacy-minimizing choice without locale text", {
  skip: !enabled,
}, async t => {
  const browser = await chromium.launch({ executablePath: (await browserCapability()).chromium_executable,
    headless: true });
  t.after(() => browser.close());
  const page = await browser.newPage();
  await page.setContent(`<ytd-consent-bump-v2-lightbox id="lightbox" style="display:block">
    <button class="ytSpecButtonShapeNextFilled ytSpecButtonShapeNextMono" data-choice="reject">拒绝</button>
    <button class="ytSpecButtonShapeNextFilled ytSpecButtonShapeNextMono" data-choice="accept">接受</button>
  </ytd-consent-bump-v2-lightbox><script>
    document.querySelectorAll('button').forEach(button => button.onclick=()=>{
      document.body.dataset.choice=button.dataset.choice;
      document.querySelector('#lightbox').style.display='none';
    });
  </script>`);
  assert.equal(await dismissYouTubeConsent(page), true);
  assert.equal(await page.locator("body").getAttribute("data-choice"), "reject");
});

test("TikTok consent prefers the explicit reject action without locale text", {
  skip: !enabled,
}, async t => {
  const browser = await chromium.launch({ executablePath: (await browserCapability()).chromium_executable,
    headless: true });
  t.after(() => browser.close());
  const page = await browser.newPage();
  await page.setContent(`<div id="onetrust-banner-sdk">
    <button id="onetrust-reject-all-handler" data-choice="reject">拒绝可选项</button>
    <button id="onetrust-accept-btn-handler" data-choice="accept">全部同意</button>
  </div><script>
    document.querySelectorAll('button').forEach(button => button.onclick=()=>{
      document.body.dataset.choice=button.dataset.choice;
      document.querySelector('#onetrust-banner-sdk').style.display='none';
    });
  </script>`);
  assert.equal(await dismissTikTokConsent(page), true);
  assert.equal(await page.locator("body").getAttribute("data-choice"), "reject");
});

test("TikTok consent uses an explicit accept action when no reject action exists", {
  skip: !enabled,
}, async t => {
  const browser = await chromium.launch({ executablePath: (await browserCapability()).chromium_executable,
    headless: true });
  t.after(() => browser.close());
  const page = await browser.newPage();
  await page.setContent(`<div data-e2e="cookie-banner">
    <button data-e2e="cookie-banner-accept">Accept</button>
  </div><script>
    document.querySelector('button').onclick=()=>document.querySelector('[data-e2e="cookie-banner"]').style.display='none';
  </script>`);
  assert.equal(await dismissTikTokConsent(page), true);
});
