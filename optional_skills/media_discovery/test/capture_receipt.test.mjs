import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { handleRequest, backgroundRetryDelayMs } from "../src/main.mjs";

async function contextFor(t) {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "media-discovery-receipt-"));
  t.after(() => fs.rm(root, { recursive: true, force: true }));
  return { skill_storage: { storage_kind: "directory", directory_path: root } };
}

test("receipt reports only committed captions, covers and observed metric names", async (t) => {
  const context = await contextFor(t);
  const request = { args: { action: "run_once", platform: "kuaishou" }, context };
  const runtime = { collectPlatform: async ({ onPage }) => {
    await onPage({ records: [{ kind: "video", platform: "kuaishou", dedup_key: "receipt:video",
      platform_text: "caption", cover_screenshot_path: "video_covers/fixture.png",
      engagement: { metrics: { likes: { display: "123" } } } }], temporaryPaths: [] });
  } };
  const result = await handleRequest(request, runtime);
  assert.deepEqual(result.extra.run.capture_summary, {
    records_saved: 1, captions_saved: 1, covers_saved: 1, engagement_metrics: ["likes"],
  });
  assert.equal(result.extra.exports.storage, "local_persistent_csv");
  assert.equal(result.extra.exports.delivery_requested, false);
  await fs.access(result.extra.exports.videos_csv);
  const duplicate = await handleRequest(request, runtime);
  assert.equal(duplicate.extra.run.capture_summary.records_saved, 0);
  assert.deepEqual(duplicate.extra.run.capture_summary.engagement_metrics, []);
});

test("network restriction keeps an explicit waiting receipt without login popup", async (t) => {
  const context = await contextFor(t);
  const result = await handleRequest({ args: { action: "run_once", platform: "xiaohongshu" }, context }, {
    collectPlatform: async () => { throw new Error("network_access_restricted"); },
    waitForInteractiveLogin: async () => { throw new Error("must not open login"); },
  });
  assert.equal(result.extra.state, "waiting_for_network_access");
  assert.equal(result.extra.run.error_code, "network_access_restricted");
  assert.equal(result.extra.run.capture_summary.records_saved, 0);
  assert.equal(backgroundRetryDelayMs({}, "network_access_restricted", 1, () => 0.5), 30 * 60_000);
  assert.equal(backgroundRetryDelayMs({}, "network_access_restricted", 20, () => 0.5), 6 * 60 * 60_000);
});
