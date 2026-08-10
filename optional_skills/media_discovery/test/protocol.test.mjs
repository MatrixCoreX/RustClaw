import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import { handleRequest, normalizedConfig, requestedPlatforms } from "../src/main.mjs";

async function requestContext(t) {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "media-discovery-protocol-"));
  const artifacts = path.join(root, "artifacts");
  await fs.mkdir(artifacts);
  t.after(() => fs.rm(root, { recursive: true, force: true }));
  return {
    skill_storage: { storage_kind: "directory", directory_path: path.join(root, "storage") },
    artifact_output_directory: artifacts,
  };
}

test("schema normalization accepts singular platform without natural-language parsing", () => {
  assert.deepEqual(requestedPlatforms({ platform: "douyin" }), ["douyin"]);
  assert.equal(normalizedConfig({}).source_mode, "home_feed");
  assert.equal(normalizedConfig({}).max_images_per_post, 100);
  assert.equal(normalizedConfig({}).browser_mode, "visible");
  assert.equal(normalizedConfig({ browser_mode: "silent" }).browser_mode, "silent");
  assert.throws(() => normalizedConfig({ browser_mode: "hidden" }));
  assert.throws(() => requestedPlatforms({ platform: "unknown" }));
});

test("keyword search preview uses topics as the only structured search input", async (t) => {
  const context = await requestContext(t);
  const result = await handleRequest({
    args: {
      action: "preview_enable",
      platform: "xiaohongshu",
      source_mode: "topics",
      topics: ["AI agent", "机器人"],
      browser_mode: "silent",
    },
    context,
  });
  assert.equal(result.status, "ok");
  assert.equal(result.extra.config.source_mode, "topics");
  assert.deepEqual(result.extra.config.topics, ["AI agent", "机器人"]);
  assert.equal(result.extra.config.browser_mode, "silent");
  assert.equal(result.extra.side_effect_applied, false);
});

test("enable, status, disable, and disabled run_once form a durable control loop", async (t) => {
  const context = await requestContext(t);
  const enabled = await handleRequest({
    args: { action: "enable", platform: "douyin", confirm: true },
    context,
  });
  assert.equal(enabled.status, "ok");
  assert.equal(enabled.extra.schedule_spec.capability, "schedule.create_structured");
  assert.equal(enabled.extra.schedule_spec.completion_required, true);
  assert.equal(enabled.extra.next_capability, "schedule.create_structured");
  const scheduleIntent = JSON.parse(enabled.extra.schedule_spec.args.intent_json);
  assert.equal(scheduleIntent.task.payload.skill_name, "media_discovery");
  assert.deepEqual(scheduleIntent.task.payload.args.platforms, ["douyin"]);
  assert.equal(scheduleIntent.task.payload.args.scheduled_run, true);

  const current = await handleRequest({ args: { action: "status" }, context });
  assert.equal(current.extra.platforms.douyin.enabled, true);

  const disabled = await handleRequest({ args: { action: "disable", platform: "douyin" }, context });
  assert.equal(disabled.extra.platform_states.douyin.enabled, false);
  assert.equal(disabled.extra.lifecycle_state, "idle");
  assert.equal(disabled.extra.schedule_cleanup_required, true);
  assert.equal(disabled.extra.schedule_cleanup_spec.capability, "schedule.delete_matching");
  assert.deepEqual(disabled.extra.schedule_cleanup_spec.args, {
    match_task_kind: "run_skill",
    match_skill_name: "media_discovery",
    match_task_action: "run_once",
    match_platforms: ["douyin"],
  });

  const run = await handleRequest({
    args: { action: "run_once", platform: "douyin", scheduled_run: true },
    context,
  });
  assert.equal(run.status, "ok");
  assert.equal(run.extra.state, "disabled_or_paused");

  const enabledBatch = await handleRequest({ args: { action: "run_enabled_once" }, context });
  assert.equal(enabledBatch.status, "ok");
  assert.equal(enabledBatch.extra.state, "disabled_or_paused");
});

test("a second start is rejected while the current run owns the lease and disable drains it", async (t) => {
  const context = await requestContext(t);
  await handleRequest({
    args: { action: "enable", platform: "douyin", confirm: true, recognition_mode: "metadata_only" },
    context,
  });
  const storage = await import("../src/storage.mjs");
  const root = context.skill_storage.directory_path;
  const { run } = await storage.beginRun(root, ["douyin"]);

  const secondStart = await handleRequest({
    args: { action: "enable", platform: "xiaohongshu", confirm: true },
    context,
  });
  assert.equal(secondStart.status, "error");
  assert.equal(secondStart.extra.error_code, "run_already_active");
  await assert.rejects(() => storage.beginRun(root, ["douyin"]), /run_already_active/u);

  const disabled = await handleRequest({ args: { action: "disable", platform: "douyin" }, context });
  assert.equal(disabled.extra.lifecycle_state, "draining");
  assert.equal(disabled.extra.drain_run_id, run.run_id);
  assert.equal(disabled.extra.stop_mode, "after_current_item");
  assert.equal(await storage.heartbeat(root, run.run_id, { items: 1 }), true);

  const completed = await storage.finishRun(root, run, "stopped_after_current_item");
  assert.equal(completed.status, "stopped_after_current_item");
  const finalState = await storage.readState(root);
  assert.equal(finalState.active_run, null);
  assert.equal(finalState.stop_after_item_run_id, null);
});

test("continuous enabled state rejects a queued duplicate start without mutation", async (t) => {
  const context = await requestContext(t);
  await handleRequest({
    args: { action: "enable", platform: "douyin", confirm: true },
    context,
  });
  const duplicate = await handleRequest({
    args: { action: "enable", platform: "xiaohongshu", confirm: true },
    context,
  });
  assert.equal(duplicate.status, "error");
  assert.equal(duplicate.extra.error_code, "collection_already_enabled");
  assert.equal(duplicate.extra.failure_phase, "pre_dispatch");
  assert.equal(duplicate.extra.side_effect_applied, false);

  const current = await handleRequest({ args: { action: "status" }, context });
  assert.equal(current.extra.platforms.douyin.enabled, true);
  assert.equal(current.extra.platforms.xiaohongshu, undefined);
});

test("resume rejects an unconfigured platform without creating empty state", async (t) => {
  const context = await requestContext(t);
  const resumed = await handleRequest({ args: { action: "resume", platform: "douyin" }, context });
  assert.equal(resumed.status, "error");
  assert.equal(resumed.extra.error_code, "platform_not_configured");
  assert.equal(resumed.extra.failure_phase, "pre_dispatch");
  const current = await handleRequest({ args: { action: "status" }, context });
  assert.equal(current.extra.platforms.douyin, undefined);
});

test("pause requests a graceful drain of an active platform batch", async (t) => {
  const context = await requestContext(t);
  await handleRequest({
    args: { action: "enable", platform: "douyin", confirm: true },
    context,
  });
  const storage = await import("../src/storage.mjs");
  const root = context.skill_storage.directory_path;
  const { run } = await storage.beginRun(root, ["douyin"]);

  const paused = await handleRequest({ args: { action: "pause", platform: "douyin" }, context });
  assert.equal(paused.status, "ok");
  assert.equal(paused.extra.platform_states.douyin.state, "paused");
  assert.equal(paused.extra.lifecycle_state, "draining");
  assert.equal(paused.extra.drain_run_id, run.run_id);
});

test("export_results returns exactly two CSV artifacts", async (t) => {
  const context = await requestContext(t);
  const result = await handleRequest({ args: { action: "export_results" }, context });
  assert.equal(result.status, "ok");
  assert.deepEqual(result.extra.artifacts.map((artifact) => artifact.filename), ["videos.csv", "images.csv"]);
  for (const artifact of result.extra.artifacts) assert.equal(await fs.stat(artifact.path).then((stat) => stat.isFile()), true);
});

test("asynchronous failures preserve their requested action", async () => {
  const result = await handleRequest({
    args: { action: "enable", platform: "douyin", confirm: true },
    context: {},
  });
  assert.equal(result.status, "error");
  assert.equal(result.extra.action, "enable");
  assert.equal(result.extra.error_code, "skill_storage_required");
});

test("canonical envelope fields cannot be overridden by business extras", async (t) => {
  const context = await requestContext(t);
  const result = await handleRequest({ args: { action: "status" }, context });
  assert.equal(result.extra.schema_version, 1);
  assert.equal(result.extra.source_skill, "media_discovery");
  assert.equal(result.extra.status, "ok");
  assert.equal(result.extra.action, "status");
});
