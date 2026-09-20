import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { handleRequest, normalizedConfig } from "../src/main.mjs";
import { browserCapability } from "../src/browser.mjs";

const defaults = { douyin: "silent", xiaohongshu: "silent", kuaishou: "silent" };

async function contextFor(t) {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "discovery-mode-"));
  t.after(() => fs.rm(root, { recursive: true, force: true }));
  return { skill_storage: { storage_kind: "directory", directory_path: root } };
}

test("Xiaohongshu uses slower default pacing and rest than other platforms", () => {
  const xiaohongshu = normalizedConfig({ platform: "xiaohongshu" });
  const douyin = normalizedConfig({ platform: "douyin" });
  assert.equal(xiaohongshu.browser_mode, "silent");
  assert.equal(xiaohongshu.pacing_min_delay_ms, 2400);
  assert.equal(xiaohongshu.pacing_max_delay_ms, 5200);
  assert.equal(xiaohongshu.rest_min_seconds, 360);
  assert.equal(xiaohongshu.rest_max_seconds, 720);
  assert.equal(douyin.pacing_min_delay_ms, 1000);
  assert.equal(douyin.pacing_max_delay_ms, 2800);
  assert.equal(douyin.rest_min_seconds, 180);
});

test("browser capability and normalization agree on platform defaults and overrides", async () => {
  assert.deepEqual((await browserCapability()).default_modes, defaults);
  for (const [platform, mode] of Object.entries(defaults)) {
    assert.equal(normalizedConfig({ platform }).browser_mode, mode);
    for (const explicit of ["silent", "visible"]) {
      assert.equal(normalizedConfig({ platform, browser_mode: explicit }).browser_mode, explicit);
    }
  }
});

test("mixed-platform preview and enable preserve independent browser defaults", async t => {
  const context = await contextFor(t);
  const platforms = Object.keys(defaults);
  const preview = await handleRequest({ args: { action: "preview_enable", platforms }, context });
  assert.equal(preview.status, "ok");
  assert.equal(preview.extra.config, undefined);
  const enabled = await handleRequest({ args: { action: "enable", platforms, confirm: true }, context });
  assert.equal(enabled.status, "ok");
  for (const platform of platforms) {
    assert.equal(preview.extra.platform_configs[platform].browser_mode, defaults[platform]);
    assert.deepEqual(enabled.extra.platform_states[platform].config, preview.extra.platform_configs[platform]);
  }
});

for (const explicit of [undefined, "silent", "visible"]) {
  test(`one-shot modes persist in receipts: ${explicit || "platform defaults"}`, async t => {
    const context = await contextFor(t);
    const seen = {};
    const result = await handleRequest({
      args: { action: "run_once", platforms: Object.keys(defaults), max_items_per_run: 3,
        ...(explicit ? { browser_mode: explicit } : {}) }, context,
    }, {
      collectPlatform: async ({ platform, config, onPage }) => {
        seen[platform] = config.browser_mode;
        await onPage({ records: [{ kind: "video", platform, dedup_key: platform }], temporaryPaths: [] });
      },
    });
    const expected = Object.fromEntries(Object.entries(defaults).map(([platform, mode]) => [platform, explicit || mode]));
    assert.equal(result.status, "ok");
    assert.deepEqual(seen, expected);
    assert.equal(result.extra.runs.length, 3);
    assert.deepEqual(Object.assign({}, ...result.extra.runs.map(run => run.browser_modes)), expected);
  });
}

test("resume keeps an explicitly configured visible Xiaohongshu session", async t => {
  const context = await contextFor(t);
  await handleRequest({ args: { action: "enable", platform: "xiaohongshu", browser_mode: "visible", confirm: true }, context });
  await handleRequest({ args: { action: "pause", platform: "xiaohongshu" }, context });
  const result = await handleRequest({ args: { action: "resume", platform: "xiaohongshu" }, context });
  assert.equal(result.status, "ok");
  assert.equal(result.extra.platform_states.xiaohongshu.config.browser_mode, "visible");
});
