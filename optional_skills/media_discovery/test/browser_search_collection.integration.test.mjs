import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { chromium } from "playwright";
import { collectPlatform } from "../src/browser.mjs";
import { handleRequest } from "../src/main.mjs";
import { sourceTargets } from "../src/platforms.mjs";
import { readRecords } from "../src/storage.mjs";

const enabled = process.env.MEDIA_DISCOVERY_BROWSER_TEST === "1";
const config = { source_mode: "topics", topics: ["财经"], browser_mode: "silent",
  max_scrolls_per_source: 1, pacing_min_delay_ms: 200, pacing_max_delay_ms: 200 };

for (const [platform, prefix] of [
  ["douyin", "https://www.douyin.com/video/"],
  ["xiaohongshu", "https://www.xiaohongshu.com/explore/"],
  ["kuaishou", "https://www.kuaishou.com/short-video/"],
]) {
  test(`${platform} search commits details in order with keyword provenance and skips a missing post`, { skip: !enabled }, async t => {
    const root = await fs.mkdtemp(path.join(os.tmpdir(), "media-discovery-search-"));
    t.after(() => fs.rm(root, { recursive: true, force: true }));
    const source = sourceTargets(platform, config)[0].url;
    const urls = ["73100000", "73100001", "73100002", "73100003"].map(id => `${prefix}${id}?source=fixture`);
    const navigations = [];
    const originalLaunch = chromium.launchPersistentContext.bind(chromium);
    t.mock.method(chromium, "launchPersistentContext", async (profile, options) => {
      const context = await originalLaunch(profile, options);
      await context.route("**/*", async route => {
        const url = route.request().url();
        if (!route.request().isNavigationRequest()) return route.abort();
        navigations.push(url);
        if (url === source) return route.fulfill({ contentType: "text/html", body: `<main>
          <a hidden href="${prefix}99999999">hidden</a>
          ${urls.map(href => `<a href="${href}">post</a>`).join("")}
          <a href="${urls[1]}">duplicate</a></main>` });
        if (url === urls[0]) return route.fulfill({ status: 404, contentType: "text/html", body: "missing" });
        assert.ok(urls.includes(url), `unexpected navigation ${url}`);
        const caption = `Caption ${urls.indexOf(url)}`;
        const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="500" height="500"><rect width="500" height="500" fill="#278b9a"/><text x="50" y="250" font-size="42">${caption}</text></svg>`;
        return route.fulfill({ contentType: "text/html", body: `<head><title>${caption}</title>
          <meta property="og:description" content="${caption}"></head><body>
          <time datetime="2026-09-09">2026-09-09</time>
          <img width="500" height="500" src="data:image/svg+xml;base64,${Buffer.from(svg).toString("base64")}"></body>` });
      });
      return context;
    });
    const result = await handleRequest({ args: { ...config, action: "run_once", platform,
      max_items_per_run: 3, max_run_minutes: 5 },
      context: { skill_storage: { storage_kind: "directory", directory_path: root } } }, {
      collectPlatform: request => collectPlatform({ ...request, config: { ...request.config,
        pacing_min_delay_ms: 1, pacing_max_delay_ms: 1 } }),
    });
    assert.equal(result.status, "ok", JSON.stringify(result));
    assert.equal(result.extra.run.counts.items, 3);
    assert.equal(result.extra.run.counts.failures, 1);
    assert.equal(result.extra.run.capture_summary.records_saved, 3);
    assert.equal(result.extra.run.browser_session_open, false);
    assert.deepEqual(navigations, [source, ...urls]);
    const records = await readRecords(root);
    assert.deepEqual(records.map(r => r.source_page_url), urls.slice(1));
    assert.deepEqual(records.map(r => r.global_sequence), [1, 2, 3]);
    for (const record of records) {
      assert.equal(record.search_keyword, "财经");
      assert.equal(record.discovery_source_url, source);
      assert.equal(record.published_at, "2026-09-09");
      assert.ok(record.platform_text.startsWith("Caption"));
      assert.ok((await fs.stat(path.join(root, "exports", record.image_screenshot_path))).size > 512);
    }
  });
}
