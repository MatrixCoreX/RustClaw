import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { chromium } from "playwright";
import { browserCapability, waitForInteractiveLogin } from "../src/browser.mjs";
import { launchPlatformBrowser, nativeBrowserEnvironment } from "../src/browser_environment.mjs";

const enabled = process.env.MEDIA_DISCOVERY_BROWSER_TEST === "1";
const source = "https://www.douyin.com/video/73100000";

async function readEnvironment(page) {
  return page.evaluate(() => ({ locale: navigator.language,
    timezone: Intl.DateTimeFormat().resolvedOptions().timeZone,
    width: innerWidth, height: innerHeight, platform: navigator.platform,
    webdriver: navigator.webdriver }));
}

test("real browser reuses locale, timezone, viewport and session across restart", { skip: !enabled }, async t => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "discovery-environment-browser-"));
  t.after(() => fs.rm(root, { recursive: true, force: true }));
  const options = { chromium, root, platform: "douyin", headless: true,
    executablePath: (await browserCapability()).chromium_executable,
    native: nativeBrowserEnvironment({ platform: process.platform, arch: process.arch,
      locale: "zh-CN", timeZone: "Asia/Shanghai" }) };
  let original;
  for (let i = 0; i < 2; i += 1) {
    const context = await launchPlatformBrowser({ ...options, ...(i ? {
      native: nativeBrowserEnvironment({ platform: process.platform, arch: process.arch,
        locale: "en-US", timeZone: "UTC" }) } : {}) });
    try {
      let language;
      await context.route("**/*", route => {
        language = route.request().headers()["accept-language"];
        return route.fulfill({ contentType: "text/html", body: "<main>fixture</main>" });
      });
      const page = context.pages()[0];
      await page.goto(source);
      const actual = await readEnvironment(page);
      assert.match(language, /^zh-CN/);
      assert.equal(actual.locale, "zh-CN");
      assert.equal(actual.timezone, "Asia/Shanghai");
      assert.equal(actual.webdriver, true); // No automation-marker suppression.
      if (i === 0) {
        original = actual;
        await context.addCookies([{ name: "fixture_session", value: "retained", url: source,
          expires: Math.floor(Date.now() / 1000) + 3600 }]);
      } else {
        assert.deepEqual(actual, original);
        assert.equal((await context.cookies()).find(cookie => cookie.name === "fixture_session")?.value, "retained");
      }
    } finally { await context.close(); }
  }
});

test("actual manual verification popup uses the same environment and still requires confirmation", { skip: !enabled }, async t => {
  const capability = await browserCapability();
  if (!capability.gui_available) return t.skip("display_unavailable");
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "discovery-environment-handoff-"));
  t.after(() => fs.rm(root, { recursive: true, force: true }));
  const native = nativeBrowserEnvironment();
  const context = await launchPlatformBrowser({ chromium, root, platform: "douyin", headless: true,
    executablePath: capability.chromium_executable });
  let silent;
  try {
    silent = await readEnvironment(context.pages()[0]);
    await context.addCookies([{ name: "sessionid", value: "fixture-only", url: source,
      expires: Math.floor(Date.now() / 1000) + 3600 }]);
  } finally { await context.close(); }
  const saved = await fs.readFile(path.join(root, "browser-profile", "douyin", "browser-environment.json"), "utf8");
  const originalLaunch = chromium.launchPersistentContext.bind(chromium);
  let manual;
  t.mock.method(chromium, "launchPersistentContext", async (profile, options) => {
    assert.equal(options.headless, false);
    assert.equal(options.locale, native.locale);
    assert.equal(options.timezoneId, native.timezone_id);
    assert.equal(options.userAgent, undefined);
    manual = await originalLaunch(profile, options);
    await manual.route("**/*", route => route.fulfill({ contentType: "text/html",
      body: '<video width="400" height="300"></video>' }));
    return manual;
  });
  const result = await waitForInteractiveLogin({ root, platform: "douyin", config: {},
    errorCode: "login_required", targetUrl: source, timeoutMs: 5000,
    onOpened: async () => {
      const page = manual.pages().find(candidate => candidate.url() === source);
      assert.deepEqual(await readEnvironment(page), silent);
      const control = manual.pages().find(candidate => candidate !== page);
      // Test fixture only: production waits for a human to press this control.
      await control.locator('[data-action="continue"]').click();
    },
  });
  assert.equal(result.ready, true);
  assert.equal(result.user_confirmed, true);
  assert.deepEqual(manual.pages(), []);
  assert.equal(await fs.readFile(path.join(root, "browser-profile", "douyin", "browser-environment.json"), "utf8"), saved);
});
