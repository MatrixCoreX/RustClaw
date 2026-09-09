import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import {
  backgroundRetryDelayMs,
  backgroundRestDelayMs,
  handleRequest,
  normalizedConfig,
  requestedPlatforms,
} from "../src/main.mjs";

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
  assert.equal(normalizedConfig({}).max_items_per_run, 5);
  assert.equal(normalizedConfig({}).max_images_per_post, 100);
  assert.equal(normalizedConfig({}).browser_mode, "silent");
  assert.equal(normalizedConfig({}).rest_min_seconds, 180);
  assert.equal(normalizedConfig({}).rest_max_seconds, 420);
  assert.equal(normalizedConfig({}).pacing_min_delay_ms, 1000);
  assert.equal(normalizedConfig({}).pacing_max_delay_ms, 2800);
  assert.equal(backgroundRestDelayMs({ douyin: { rest_min_seconds: 10, rest_max_seconds: 20 } }, () => 0.5), 15_000);
  assert.equal(backgroundRetryDelayMs({}, "challenge_required", 1, () => 0.5), 15 * 60 * 1000);
  assert.equal(backgroundRetryDelayMs({}, "challenge_required", 2, () => 0.5), 30 * 60 * 1000);
  assert.equal(backgroundRetryDelayMs({}, "challenge_required", 20, () => 0.5), 6 * 60 * 60 * 1000);
  assert.equal(backgroundRetryDelayMs({ douyin: { rest_min_seconds: 10, rest_max_seconds: 20 } }, "selector_drift", 2, () => 0.5), 15_000);
  assert.equal(normalizedConfig({ browser_mode: "visible" }).browser_mode, "visible");
  assert.throws(() => normalizedConfig({ rest_min_seconds: 30, rest_max_seconds: 20 }));
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

test("a completed collection with no items returns a structured retryable error", async (t) => {
  const context = await requestContext(t);
  const result = await handleRequest({
    request_id: "empty-topic-run",
    args: {
      action: "run_once",
      platform: "douyin",
      source_mode: "topics",
      topics: ["finance"],
      browser_mode: "silent",
    },
    context,
  }, {
    collectPlatform: async () => ({ handled: 0 }),
  });

  assert.equal(result.status, "error");
  assert.equal(result.extra.status, "error");
  assert.equal(result.extra.action, "run_once");
  assert.equal(result.extra.error_code, "no_items_collected");
  assert.equal(result.extra.message_key, "skill.media_discovery.no_items_collected");
  assert.equal(result.extra.retryable, true);
});

test("a silent challenge opens one manual verification session per batch", async (t) => {
  const context = await requestContext(t);
  await handleRequest({
    args: { action: "enable", platform: "douyin", confirm: true, browser_mode: "silent" },
    context,
  });

  let loginSessions = 0;
  const result = await handleRequest({
    request_id: "silent-challenge",
    args: { action: "run_enabled_once" },
    context,
  }, {
    maxContinuousCycles: 1,
    sleep: async () => {},
    collectPlatform: async ({ config }) => {
      assert.equal(config.browser_mode, "silent");
      throw new Error("challenge_required");
    },
    waitForInteractiveLogin: async () => {
      loginSessions += 1;
      return { ready: true, reason_code: "interactive_access_ready" };
    },
  });

  assert.equal(result.status, "ok");
  assert.equal(result.extra.state, "stopped");
  assert.equal(result.extra.background_worker.counts.items, 0);
  assert.equal(result.extra.background_worker.last_error_code, "manual_verification_not_restored");
  assert.equal(loginSessions, 1);
});

test("a silent login barrier opens one login session and retries silently", async (t) => {
  const context = await requestContext(t);
  await handleRequest({
    args: { action: "enable", platform: "douyin", confirm: true, browser_mode: "silent" },
    context,
  });

  let collectionAttempts = 0;
  let loginSessions = 0;
  const result = await handleRequest({
    request_id: "interactive-login-wait",
    args: { action: "run_enabled_once" },
    context,
  }, {
    maxContinuousCycles: 1,
    sleep: async () => {},
    collectPlatform: async ({ config, onPage }) => {
      collectionAttempts += 1;
      assert.equal(config.browser_mode, "silent");
      if (collectionAttempts === 1) throw new Error("login_required");
      await onPage({
        records: [{
          kind: "video",
          dedup_key: "douyin:interactive-login-wait:video",
          platform: "douyin",
          title: "fixture",
          video_page_url: "https://www.douyin.com/video/1234567891",
          discovered_at: "2026-09-08T00:00:00Z",
        }],
        temporaryPaths: [],
      });
      return { handled: 1 };
    },
    waitForInteractiveLogin: async () => {
      loginSessions += 1;
      return { ready: true, reason_code: "interactive_authentication_ready" };
    },
  });

  assert.equal(result.status, "ok");
  assert.equal(result.extra.state, "stopped");
  assert.equal(result.extra.background_worker.counts.items, 1);
  assert.equal(collectionAttempts, 2);
  assert.equal(loginSessions, 1);
});

test("one-shot challenge permits a manual popup and resumes the original silent mode", async (t) => {
  const context = await requestContext(t);
  await handleRequest({
    args: { action: "enable", platform: "douyin", confirm: true, browser_mode: "silent" },
    context,
  });

  let loginSessions = 0;
  const result = await handleRequest({
    request_id: "one-shot-login-required",
    args: { action: "run_once", platforms: ["douyin"] },
    context,
  }, {
    collectPlatform: async () => {
      throw new Error("challenge_required");
    },
    waitForInteractiveLogin: async () => {
      loginSessions += 1;
      return { ready: true };
    },
  });

  assert.equal(result.status, "ok");
  assert.equal(result.extra.state, "waiting_for_manual_verification");
  assert.equal(loginSessions, 1);
});

test("enable, status, disable, and disabled run_once form a durable control loop", async (t) => {
  const context = await requestContext(t);
  const enabled = await handleRequest({
    args: { action: "enable", platform: "douyin", confirm: true },
    context,
  });
  assert.equal(enabled.status, "ok");
  assert.equal(enabled.extra.background_start_spec.capability, "media_discovery.run_enabled_once");
  assert.equal(enabled.extra.background_start_spec.completion_required, true);
  assert.equal(enabled.extra.next_capability, "media_discovery.run_enabled_once");
  assert.equal(enabled.extra.schedule_spec, undefined);

  const current = await handleRequest({ args: { action: "status" }, context });
  assert.equal(current.extra.platforms.douyin.enabled, true);

  const disabled = await handleRequest({ args: { action: "disable", platform: "douyin" }, context });
  assert.equal(disabled.extra.platform_states.douyin.enabled, false);
  assert.equal(disabled.extra.lifecycle_state, "idle");
  assert.equal(disabled.extra.schedule_cleanup_required, undefined);

  const run = await handleRequest({
    args: { action: "run_once" },
    context,
  });
  assert.equal(run.status, "ok");
  assert.equal(run.extra.state, "disabled_or_paused");

  const enabledBatch = await handleRequest({ args: { action: "run_enabled_once" }, context });
  assert.equal(enabledBatch.status, "error");
  assert.equal(enabledBatch.extra.error_code, "background_collection_not_enabled");
});

test("manual verification stop is a graceful result and diagnostics survive a failed run", async (t) => {
  const context = await requestContext(t);
  const request = { args: { action: "run_once", platform: "douyin", source_mode: "home_feed" }, context };
  const stopped = await handleRequest(request, { collectPlatform: async () => { throw new Error("collection_stopped"); } });
  assert.equal(stopped.status, "ok");
  assert.equal(stopped.extra.state, "stopped_after_current_item");
  assert.equal(stopped.extra.run.counts.failures, 0);
  assert.equal(stopped.extra.run.error_code, null);

  const diagnostic = { schema_version: 1, platform: "douyin", stage: "recommendation_ready", document: null };
  const failed = await handleRequest(request, { collectPlatform: async () => {
    throw Object.assign(new Error("selector_drift"), { discovery_diagnostic: diagnostic });
  } });
  assert.equal(failed.extra.error_code, "selector_drift");
  assert.deepEqual(failed.extra.failure_diagnostic, diagnostic);
  const state = JSON.parse(await fs.readFile(path.join(context.skill_storage.directory_path, "state.json"), "utf8"));
  assert.deepEqual(state.runs[0].failure_diagnostic, diagnostic);
  assert.equal(state.active_run, null);
});

test("continuous background collection stops gracefully through disable", async (t) => {
  const context = await requestContext(t);
  await handleRequest({
    args: {
      action: "enable",
      platform: "douyin",
      source_mode: "topics",
      topics: ["market research"],
      confirm: true,
    },
    context,
  });

  let collectionAttempts = 0;
  const result = await handleRequest({
    request_id: "background-graceful-stop",
    args: { action: "run_enabled_once" },
    context,
  }, {
    sleep: async () => {},
    collectPlatform: async ({ config, onPage }) => {
      collectionAttempts += 1;
      assert.equal(config.source_mode, "topics");
      assert.deepEqual(config.topics, ["market research"]);
      await onPage({
        records: [{
          kind: "video",
          dedup_key: "douyin:background-stop:video",
          platform: "douyin",
          title: "fixture",
          video_page_url: "https://www.douyin.com/video/1234567892",
          discovered_at: "2026-09-08T00:00:00Z",
        }],
        temporaryPaths: [],
      });
      const disabled = await handleRequest({
        args: { action: "disable", platform: "douyin" },
        context,
      });
      assert.equal(disabled.extra.lifecycle_state, "draining");
      return { handled: 1 };
    },
  });

  assert.equal(result.status, "ok");
  assert.equal(result.extra.state, "stopped");
  assert.equal(result.extra.background_worker.counts.items, 1);
  assert.equal(collectionAttempts, 1);
  const current = await handleRequest({ args: { action: "status" }, context });
  assert.equal(current.extra.platforms.douyin.enabled, false);
  assert.equal(current.extra.background_worker, null);
  assert.equal(current.extra.active_run, null);
});

test("continuous background collection terminates on a non-retryable failure", async (t) => {
  const context = await requestContext(t);
  await handleRequest({
    args: { action: "enable", platform: "douyin", confirm: true },
    context,
  });

  const result = await handleRequest({
    request_id: "background-fatal-error",
    args: { action: "run_enabled_once" },
    context,
  }, {
    sleep: async () => {},
    collectPlatform: async () => {
      throw new Error("source_url_invalid");
    },
  });

  assert.equal(result.status, "error");
  assert.equal(result.extra.error_code, "source_url_invalid");
  const current = await handleRequest({ args: { action: "status" }, context });
  assert.equal(current.extra.background_worker, null);
  assert.equal(current.extra.active_run, null);
});

test("status distinguishes live leases from expired records without rewriting state", async (t) => {
  const context = await requestContext(t);
  const storage = await import("../src/storage.mjs");
  const root = context.skill_storage.directory_path;
  await storage.configurePlatforms(root, ["douyin"], { douyin: normalizedConfig({}) });
  const worker = await storage.beginBackgroundWorker(root);
  const { run } = await storage.beginRun(root, ["douyin"]);
  const live = await handleRequest({ args: { action: "status" }, context });
  assert.equal(live.extra.background_worker.worker_id, worker.worker_id);
  assert.equal(live.extra.active_run.run_id, run.run_id);
  assert.deepEqual(live.extra.expired_leases, []);

  const state = await storage.readState(root);
  state.background_worker.heartbeat_at = "2000-01-01T00:00:00.000Z";
  state.active_runs[run.run_id].heartbeat_at = "invalid-timestamp";
  state.runs = [{ run_id: "previous", status: "completed_batch", counts: { items: 2 } }];
  const snapshot = JSON.stringify(state);
  await fs.writeFile(path.join(root, "state.json"), snapshot);
  const expired = await handleRequest({ args: { action: "status" }, context });
  assert.equal(expired.status, "ok");
  assert.equal(expired.extra.background_worker, null);
  assert.equal(expired.extra.active_run, null);
  assert.equal(expired.extra.latest_run.run_id, "previous");
  assert.deepEqual(expired.extra.expired_leases.map(({ kind, lifecycle_state }) => ({ kind, lifecycle_state })), [
    { kind: "background_worker", lifecycle_state: "heartbeat_expired" },
    { kind: "active_run", lifecycle_state: "heartbeat_expired" },
  ]);
  assert.equal(await fs.readFile(path.join(root, "state.json"), "utf8"), snapshot);
});

test("a duplicate platform start is rejected while its lease is active and disable drains it", async (t) => {
  const context = await requestContext(t);
  await handleRequest({
    args: { action: "enable", platform: "douyin", confirm: true },
    context,
  });
  const storage = await import("../src/storage.mjs");
  const root = context.skill_storage.directory_path;
  const { run } = await storage.beginRun(root, ["douyin"]);

  const secondStart = await handleRequest({
    args: { action: "enable", platform: "douyin", confirm: true },
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
    args: { action: "enable", platform: "douyin", confirm: true },
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

test("clear_results requires confirmation and clears only collected output", async (t) => {
  const context = await requestContext(t);
  const storage = await import("../src/storage.mjs");
  await storage.commitPageRecords(context.skill_storage.directory_path, [{
    kind: "video",
    dedup_key: "clear:video",
    platform: "douyin",
    title: "clear fixture",
    video_page_url: "https://www.douyin.com/video/1",
    discovered_at: "2026-09-07T00:00:00Z",
  }]);

  const rejected = await handleRequest({ args: { action: "clear_results" }, context });
  assert.equal(rejected.status, "error");
  assert.equal(rejected.extra.error_code, "confirmation_required");
  assert.equal((await storage.readRecords(context.skill_storage.directory_path)).length, 1);

  const cleared = await handleRequest({ args: { action: "clear_results", confirm: true }, context });
  assert.equal(cleared.status, "ok");
  assert.equal(cleared.extra.cleared.records, 1);
  assert.equal((await storage.readRecords(context.skill_storage.directory_path)).length, 0);
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
