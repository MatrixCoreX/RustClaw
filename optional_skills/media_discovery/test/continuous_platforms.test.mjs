import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import { handleRequest } from "../src/main.mjs";

async function requestContext(t) {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "media-discovery-platforms-"));
  t.after(() => fs.rm(root, { recursive: true, force: true }));
  return { skill_storage: { storage_kind: "directory", directory_path: root } };
}

for (const barrier of [null, "challenge_required", "network_access_restricted", "rate_limited", "selector_drift"]) {
  test(`continuous platforms receive independent batches after ${barrier || "a full batch"}`, async (t) => {
    const context = await requestContext(t);
    const platforms = ["douyin", "xiaohongshu", "kuaishou"];
    const enabled = await handleRequest({
      args: { action: "enable", platforms, max_items_per_run: 1, confirm: true }, context,
    });
    assert.equal(enabled.status, "ok");
    const visited = [];
    const result = await handleRequest({
      request_id: "multi-platform", args: { action: "run_enabled_once" }, context,
    }, {
      maxContinuousCycles: 3,
      sleep: async () => {},
      collectPlatform: async ({ platform, limit, config, onPage }) => {
        visited.push(platform);
        assert.equal(limit, 1);
        assert.equal(config.browser_mode, "silent");
        if (platform === "douyin" && barrier) throw new Error(barrier);
        await onPage({
          records: [{ kind: "video", dedup_key: `${platform}:fixture:video`, platform,
            title: "fixture", video_page_url: "https://example.test/item/1",
            discovered_at: "2026-09-09T00:00:00Z" }],
          temporaryPaths: [],
        });
        return { handled: 1 };
      },
    });
    assert.deepEqual(visited, platforms);
    assert.equal(result.status, "ok");
    assert.equal(result.extra.background_worker.counts.items, barrier ? 2 : 3);
    const outcomes = result.extra.background_worker.platform_outcomes;
    assert.equal(outcomes.douyin.last_error_code, barrier);
    assert.equal(outcomes.kuaishou.completed_batches, 1);
    assert.equal(outcomes.xiaohongshu.completed_batches, 1);
    if (barrier === "challenge_required") {
      assert.ok(Date.parse(outcomes.douyin.retry_not_before) > Date.parse(outcomes.kuaishou.retry_not_before));
    }
  });
}

test("cooldowns, partial commits and platform disable are independent", async (t) => {
  const context = await requestContext(t);
  await handleRequest({ args: { action: "enable", platforms: ["xiaohongshu", "kuaishou"],
    rest_min_seconds: 5, rest_max_seconds: 5, confirm: true }, context });
  let clock = Date.now();
  let attempts = 0;
  const visited = [];
  const result = await handleRequest({ args: { action: "run_enabled_once" }, context }, {
    now: () => clock,
    random: () => 0.5,
    sleep: async (milliseconds) => { clock += milliseconds; },
    collectPlatform: async ({ platform, onPage }) => {
      visited.push(platform);
      await onPage({ records: [{ kind: "video", platform, dedup_key: `partial:${++attempts}`,
        title: "fixture", platform_text: "caption", engagement: { metrics: { likes: { value: 4 } } } }],
      temporaryPaths: [] });
      if (platform === "xiaohongshu") throw new Error("selector_drift");
      await handleRequest({ args: { action: "disable", platform: "xiaohongshu" }, context });
      if (attempts === 3) await handleRequest({ args: { action: "disable", platform: "kuaishou" }, context });
    },
  });
  assert.deepEqual(visited, ["xiaohongshu", "kuaishou", "kuaishou"]);
  assert.equal(result.extra.background_worker.counts.items, 3);
  assert.equal(result.extra.background_worker.counts.videos, 3);
  assert.equal(result.extra.background_worker.counts.failures, 1);
  assert.equal(result.extra.background_worker.platform_outcomes.xiaohongshu.counts.items, 1);
  assert.equal(result.extra.background_worker.platform_outcomes.kuaishou.completed_batches, 2);
});

test("a paused platform does not block another platform or create a second browser", async (t) => {
  const context = await requestContext(t);
  await handleRequest({ args: { action: "enable", platforms: ["xiaohongshu", "kuaishou"], confirm: true }, context });
  await handleRequest({ args: { action: "pause", platform: "xiaohongshu" }, context });
  let active = 0;
  const visited = [];
  const result = await handleRequest({ args: { action: "run_enabled_once" }, context }, {
    maxContinuousCycles: 2,
    collectPlatform: async ({ platform, onPage }) => {
      assert.equal(++active, 1);
      visited.push(platform);
      await onPage({ records: [{ kind: "video", platform, dedup_key: `${platform}:pause-test` }], temporaryPaths: [] });
      if (platform === "kuaishou") await handleRequest({ args: { action: "resume", platform: "xiaohongshu" }, context });
      active -= 1;
    },
  });
  assert.equal(result.status, "ok");
  assert.deepEqual(visited, ["kuaishou", "xiaohongshu"]);
});
