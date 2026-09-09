import assert from "node:assert/strict";
import test from "node:test";
import { chromium } from "playwright";
import { browserCapability } from "../src/browser.mjs";
import { withSearchResult } from "../src/browser_search_results.mjs";

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

test("a manual access barrier does not collect or navigate away from its prompt", { skip: !enabled }, async t => {
  const { page } = await fixture(t, { barrier: true });
  await assert.rejects(withSearchResult(page, "xiaohongshu", item, source,
    () => assert.fail("must not collect"), {
      timeoutMs: 3000,
      checkAccess: async candidate => await candidate.locator('[data-blocked="true"]').count() ? "login_required" : null,
    }), { message: "login_required" });
  assert.equal(await page.locator('[data-blocked="true"]').count(), 1);
});
