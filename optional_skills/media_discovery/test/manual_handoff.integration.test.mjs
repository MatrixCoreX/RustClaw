import assert from "node:assert/strict";
import test from "node:test";
import { chromium } from "playwright";
import { browserCapability, waitForManualAccess } from "../src/browser.mjs";
import { createManualConfirmation } from "../src/manual_handoff.mjs";

const enabled = process.env.MEDIA_DISCOVERY_BROWSER_TEST === "1";

async function fixture(t, locale = "en") {
  const browser = await chromium.launch({ executablePath: (await browserCapability()).chromium_executable, headless: true });
  t.after(() => browser.close());
  const context = await browser.newContext();
  const page = await context.newPage();
  await page.route("**/*", route => route.fulfill({ contentType: "text/html",
    body: '<video width="400" height="300"></video>' }));
  await page.goto("https://www.douyin.com/");
  const confirmation = await createManualConfirmation(context, page, { platform: "douyin", locale });
  return { context, page, confirmation };
}

test("local confirmation ignores synthetic clicks and is absent from the platform page", { skip: !enabled }, async t => {
  const { page, confirmation } = await fixture(t);
  assert.equal(await page.evaluate(() => typeof window.manualCollectionAction), "undefined");
  await confirmation.page.locator('[data-action="continue"]').evaluate(node => node.click());
  assert.equal(await confirmation.readAction(), "waiting");
  await confirmation.page.locator('[data-action="continue"]').click();
  assert.equal(await confirmation.readAction(), "continue");
});

test("ready platform remains open until confirmation; hidden content stays blocked", { skip: !enabled }, async t => {
  const { context, page, confirmation } = await fixture(t);
  let polls = 0;
  const stopped = await waitForManualAccess({ context, page, confirmation, platform: "douyin",
    errorCode: "challenge_required", timeoutMs: 10000, shouldStop: async () => ++polls > 3 });
  assert.equal(stopped.error_code, "collection_stopped");
  assert.equal(page.isClosed(), false);
  assert.equal(confirmation.page.isClosed(), false);
  await confirmation.page.locator('[data-action="continue"]').click();
  const ready = await waitForManualAccess({ context, page, confirmation, platform: "douyin",
    errorCode: "challenge_required", timeoutMs: 4000 });
  assert.equal(ready.user_confirmed, true);
  await page.locator("video").evaluate(node => { node.style.display = "none"; });
  const hidden = await waitForManualAccess({ context, page, confirmation, platform: "douyin",
    errorCode: "challenge_required", timeoutMs: 1000 });
  assert.equal(hidden.error_code, "interactive_verification_timeout");
});

test("closing the control tab cancels the manual handoff", { skip: !enabled }, async t => {
  const { context, page, confirmation } = await fixture(t);
  await confirmation.page.close();
  const result = await waitForManualAccess({ context, page, confirmation, platform: "douyin",
    errorCode: "challenge_required", timeoutMs: 3000 });
  assert.equal(result.error_code, "interactive_verification_cancelled");
});

test("English and Chinese controls fit compact desktop and mobile viewports", { skip: !enabled }, async t => {
  for (const locale of ["en", "zh-CN"]) {
    const { confirmation } = await fixture(t, locale);
    for (const viewport of [{ width: 1280, height: 900 }, { width: 375, height: 760 }]) {
      await confirmation.page.setViewportSize(viewport);
      assert.equal(await confirmation.page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
      const boxes = await confirmation.page.locator("button").evaluateAll(nodes => nodes.map(node => {
        const { left, right, top, bottom } = node.getBoundingClientRect();
        return { left, right, top, bottom };
      }));
      for (const box of boxes) assert.ok(box.left >= 0 && box.right <= viewport.width && box.bottom <= viewport.height);
      for (let i = 0; i < boxes.length; i += 1) for (let j = i + 1; j < boxes.length; j += 1) {
        const a = boxes[i], b = boxes[j];
        assert.ok(a.right <= b.left || b.right <= a.left || a.bottom <= b.top || b.bottom <= a.top);
      }
    }
    await confirmation.page.locator('[data-action="pause"]').click();
    assert.equal(await confirmation.readAction(), "pause");
  }
});
