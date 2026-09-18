import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { chromium } from "playwright";
import { browserCapability, collectPlatform, currentPlatformAccessError } from "../src/browser.mjs";
import { observePlatformBackpressure } from "../src/browser_flow_control.mjs";
import { handleRequest } from "../src/main.mjs";
import { readRecords } from "../src/storage.mjs";

const enabled = process.env.MEDIA_DISCOVERY_BROWSER_TEST === "1";
const platforms = [
  ["douyin", "https://www.douyin.com/video/73100000"],
  ["xiaohongshu", "https://www.xiaohongshu.com/explore/73100000"],
  ["kuaishou", "https://www.kuaishou.com/short-video/73100000"],
];

for (const [platform, source] of platforms) {
  for (const transport of ["document", "fetch"]) {
    test(`${platform} ${transport} 429 stops collection before another source and preserves Retry-After`, { skip: !enabled }, async t => {
      const root = await fs.mkdtemp(path.join(os.tmpdir(), "media-discovery-rate-limit-"));
      t.after(() => fs.rm(root, { recursive: true, force: true }));
      const navigations = [];
      let context;
      const originalLaunch = chromium.launchPersistentContext.bind(chromium);
      t.mock.method(chromium, "launchPersistentContext", async (profile, options) => {
        context = await originalLaunch(profile, options);
        await context.route("**/*", async route => {
          const request = route.request();
          if (request.isNavigationRequest()) navigations.push(request.url());
          if (transport === "document" || request.url().endsWith("/api/fixture-feed")) {
            return route.fulfill({ status: 429, headers: { "Retry-After": "86400" },
              contentType: "text/html", body: "rate limit fixture" });
          }
          return route.fulfill({ contentType: "text/html", body: `<main></main>
            <script>fetch('/api/fixture-feed').catch(()=>{});</script>` });
        });
        return context;
      });
      const started = Date.now();
      let captures = 0;
      await assert.rejects(collectPlatform({ root, runId: "fixture", platform,
        config: { source_mode: "seed_urls", seed_urls: [source, source.replace("73100000", "73100001")],
          browser_mode: "silent", pacing_min_delay_ms: 200, pacing_max_delay_ms: 200 },
        limit: 2, shouldStop: async () => false, onPage: async () => { captures += 1; },
      }), error => error.message === "rate_limited" && error.status_code === 429
        && Date.parse(error.retry_after_at) >= started + 86_400_000);
      assert.equal(captures, 0);
      assert.deepEqual(navigations, [source]);
      assert.equal(context.listenerCount("response"), 0);
      assert.deepEqual(context.pages(), []);
    });
  }
}

test("third-party 429 does not block a real browser; platform API 429 does", { skip: !enabled }, async t => {
  const browser = await chromium.launch({ executablePath: (await browserCapability()).chromium_executable, headless: true });
  t.after(() => browser.close());
  const page = await browser.newPage();
  await page.route("**/*", route => route.fulfill(route.request().isNavigationRequest()
    ? { contentType: "text/html", body: "<main>fixture</main>" }
    : { status: 429, headers: { "access-control-allow-origin": "*", "retry-after": "3600" }, body: "fixture" }));
  await page.goto("https://www.douyin.com/");
  const dispose = observePlatformBackpressure(page.context(), "douyin");
  t.after(dispose);
  await page.evaluate(() => fetch("https://telemetry.example.test/metrics").then(response => response.text()));
  assert.equal(await currentPlatformAccessError(page, "douyin"), null);
  await page.evaluate(() => fetch("/api/fixture-feed").then(response => response.text()));
  await assert.rejects(currentPlatformAccessError(page, "douyin"), error =>
    error.message === "rate_limited" && error.discovery_stage === "access_check");
});

test("a limit received after the final capture is retained with the committed record", { skip: !enabled }, async t => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "media-discovery-partial-limit-"));
  t.after(() => fs.rm(root, { recursive: true, force: true }));
  const originalLaunch = chromium.launchPersistentContext.bind(chromium);
  let context;
  t.mock.method(chromium, "launchPersistentContext", async (profile, options) => {
    context = await originalLaunch(profile, options);
    const svg = '<svg xmlns="http://www.w3.org/2000/svg" width="500" height="500"><rect width="500" height="500" fill="#278b9a"/><text x="50" y="250" font-size="42">fixture</text></svg>';
    await context.route("**/*", route => route.fulfill(route.request().isNavigationRequest()
      ? { contentType: "text/html", body: `<h1>fixture</h1><img width="500" height="500" src="data:image/svg+xml;base64,${Buffer.from(svg).toString("base64")}">` }
      : { status: 429, headers: { "retry-after": "86400" }, body: "fixture" }));
    return context;
  });
  const result = await handleRequest({ args: { action: "run_once", platform: "douyin",
    source_mode: "seed_urls", seed_urls: [platforms[0][1]], max_items_per_run: 1,
    browser_mode: "silent", pacing_min_delay_ms: 200, pacing_max_delay_ms: 200 },
    context: { skill_storage: { storage_kind: "directory", directory_path: root } },
  }, { collectPlatform: request => collectPlatform({ ...request, onPage: async payload => {
    await request.onPage(payload);
    await context.pages()[0].evaluate(() => fetch("/api/fixture-feed").then(response => response.text()));
  } }) });
  assert.equal(result.status, "ok");
  assert.equal(result.extra.run.status, "rate_limited");
  assert.equal(result.extra.run.counts.images, 1);
  assert.equal(result.extra.run.browser_session_open, false);
  assert.ok(Date.parse(result.extra.run.retry_after_at) > Date.now() + 86_000_000);
  assert.equal((await readRecords(root)).length, 1);
});
