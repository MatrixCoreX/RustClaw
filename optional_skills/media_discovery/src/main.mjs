import fs from "node:fs/promises";
import path from "node:path";
import readline from "node:readline";
import { fileURLToPath } from "node:url";

import { browserCapability, collectPlatform } from "./browser.mjs";
import { sourceUrls, SUPPORTED_PLATFORMS } from "./platforms.mjs";
import { createBackgroundProgressReporter } from "./progress.mjs";
import {
  beginRun,
  cleanupExpiredDiagnostics,
  commitPageRecords,
  configurePlatforms,
  copyExportsTo,
  finishRun,
  heartbeat,
  readRecords,
  readState,
  requestStop,
  setPlatformControl,
  storageRoot,
} from "./storage.mjs";

const SKILL_NAME = "media_discovery";
const ERROR_CODES = new Set([
  "action_unsupported",
  "browser_missing",
  "browser_mode_invalid",
  "collection_already_enabled",
  "challenge_required",
  "confirmation_required",
  "display_unavailable",
  "invalid_args",
  "login_required",
  "media_element_not_found",
  "platform_required",
  "platform_not_configured",
  "platform_unsupported",
  "rate_limited",
  "recognition_mode_invalid",
  "run_already_active",
  "screenshot_empty",
  "selector_drift",
  "skill_storage_invalid",
  "skill_storage_required",
  "source_host_not_allowed",
  "source_mode_invalid",
  "source_scope_empty",
  "source_url_invalid",
  "storage_lock_timeout",
]);
const ACTIONS = new Set([
  "capabilities",
  "preview_enable",
  "enable",
  "disable",
  "run_once",
  "run_enabled_once",
  "status",
  "pause",
  "resume",
  "stop_current",
  "list_runs",
  "export_results",
]);
const RUN_CONFIG_FIELDS = Object.freeze([
  "source_mode",
  "topics",
  "seed_urls",
  "max_items_per_run",
  "max_images_per_post",
  "max_run_minutes",
  "max_scrolls_per_source",
  "interval_minutes",
  "retain_diagnostics_hours",
  "recognition_mode",
  "browser_mode",
  "pacing_min_delay_ms",
  "pacing_max_delay_ms",
]);

function integer(value, fallback, minimum, maximum) {
  const parsed = Number(value ?? fallback);
  if (!Number.isInteger(parsed) || parsed < minimum || parsed > maximum) throw new Error("invalid_args");
  return parsed;
}

export function requestedPlatforms(args, allowEmpty = false) {
  const raw = Array.isArray(args.platforms)
    ? args.platforms
    : typeof args.platform === "string"
      ? [args.platform]
      : [];
  const values = [...new Set(raw.map((value) => String(value).trim()).filter(Boolean))];
  if (!allowEmpty && values.length === 0) throw new Error("platform_required");
  if (values.some((value) => !SUPPORTED_PLATFORMS.includes(value))) throw new Error("platform_unsupported");
  return values;
}

export function normalizedConfig(args) {
  const sourceMode = String(args.source_mode || "home_feed");
  if (!new Set(["home_feed", "topics", "seed_urls"]).has(sourceMode)) throw new Error("source_mode_invalid");
  const recognitionMode = String(args.recognition_mode || "ocr_reviewed");
  if (!new Set(["ocr_reviewed", "local_ocr", "metadata_only"]).has(recognitionMode)) {
    throw new Error("recognition_mode_invalid");
  }
  const browserMode = String(args.browser_mode || "visible");
  if (!new Set(["visible", "silent"]).has(browserMode)) throw new Error("browser_mode_invalid");
  const pacingMinDelayMs = integer(args.pacing_min_delay_ms, 700, 200, 5000);
  const pacingMaxDelayMs = integer(args.pacing_max_delay_ms, 1800, 200, 8000);
  if (pacingMaxDelayMs < pacingMinDelayMs) throw new Error("invalid_args");
  const config = {
    source_mode: sourceMode,
    topics: Array.isArray(args.topics) ? args.topics.map(String).map((value) => value.trim()).filter(Boolean) : [],
    seed_urls: Array.isArray(args.seed_urls) ? args.seed_urls.map(String) : [],
    max_items_per_run: integer(args.max_items_per_run, 20, 1, 100),
    max_images_per_post: integer(args.max_images_per_post, 100, 1, 100),
    max_run_minutes: integer(args.max_run_minutes, 30, 5, 180),
    max_scrolls_per_source: integer(args.max_scrolls_per_source, 10, 1, 100),
    interval_minutes: integer(args.interval_minutes, 60, 10, 1440),
    retain_diagnostics_hours: integer(args.retain_diagnostics_hours, 24, 1, 168),
    recognition_mode: recognitionMode,
    browser_mode: browserMode,
    pacing_min_delay_ms: pacingMinDelayMs,
    pacing_max_delay_ms: pacingMaxDelayMs,
    capture_mode: "browser_element_screenshot",
  };
  return config;
}

function success(action, extra = {}) {
  return {
    status: "ok",
    text: "",
    error_text: null,
    extra: { ...extra, schema_version: 1, source_skill: SKILL_NAME, status: "ok", action },
  };
}

function errorResponse(action, error) {
  const errorText = String(error?.message || error || "execution_failed").split("\n", 1)[0];
  const errorCode = ERROR_CODES.has(errorText) ? errorText : "execution_failed";
  const retryable = new Set([
    "display_unavailable",
    "browser_missing",
    "login_required",
    "challenge_required",
    "rate_limited",
    "storage_lock_timeout",
  ]).has(errorCode);
  const preDispatch = new Set([
    "action_unsupported",
    "browser_mode_invalid",
    "collection_already_enabled",
    "confirmation_required",
    "invalid_args",
    "platform_required",
    "platform_not_configured",
    "platform_unsupported",
    "recognition_mode_invalid",
    "run_already_active",
    "skill_storage_invalid",
    "skill_storage_required",
    "source_host_not_allowed",
    "source_mode_invalid",
    "source_scope_empty",
    "source_url_invalid",
  ]).has(errorCode);
  return {
    status: "error",
    text: "",
    error_text: errorText,
    extra: {
      ...(error?.extra && typeof error.extra === "object" ? error.extra : {}),
      schema_version: 1,
      source_skill: SKILL_NAME,
      status: "error",
      action,
      error_code: errorCode,
      message_key: `skill.${SKILL_NAME}.${errorCode}`,
      retryable,
      ...(preDispatch ? { failure_phase: "pre_dispatch", side_effect_applied: false } : {}),
    },
  };
}

function scheduleSpec(platforms, intervalMinutes) {
  const schedule = { type: "interval", every_minutes: intervalMinutes };
  const task = {
    kind: "run_skill",
    payload: { skill_name: SKILL_NAME, args: { action: "run_once", platforms, scheduled_run: true } },
  };
  const intentJson = JSON.stringify({ kind: "create", schedule, task });
  return {
    capability: "schedule.create_structured",
    args: { intent_json: intentJson },
    owner: { skill: SKILL_NAME, platforms },
    completion_required: true,
  };
}

async function preview(args) {
  const platforms = requestedPlatforms(args);
  const config = normalizedConfig(args);
  for (const platform of platforms) sourceUrls(platform, config);
  return success("preview_enable", {
    platforms,
    config,
    browser: await browserCapability(),
    schedule_spec: scheduleSpec(platforms, config.interval_minutes),
    side_effect_applied: false,
  });
}

async function enable(request, args) {
  if (args.confirm !== true) throw new Error("confirmation_required");
  const root = storageRoot(request);
  const platforms = requestedPlatforms(args);
  const config = normalizedConfig(args);
  for (const platform of platforms) sourceUrls(platform, config);
  const state = await configurePlatforms(root, platforms, config);
  return success("enable", {
    platforms,
    platform_states: state.platforms,
    schedule_spec: scheduleSpec(platforms, config.interval_minutes),
    next_capability: "schedule.create_structured",
    side_effect_applied: true,
  });
}

async function control(request, args, action) {
  const root = storageRoot(request);
  const platforms = requestedPlatforms(args, true);
  const state = await setPlatformControl(root, platforms, action);
  const affectedPlatforms = platforms.length > 0 ? platforms : Object.keys(state.platforms);
  return success(action, {
    platforms: affectedPlatforms,
    platform_states: state.platforms,
    lifecycle_state: state.active_run?.lifecycle_state || "idle",
    drain_run_id: state.stop_after_item_run_id,
    stop_mode: state.stop_after_item_run_id ? "after_current_item" : null,
    schedule_cleanup_required: action === "disable",
    schedule_cleanup_spec: action === "disable" ? {
      capability: "schedule.delete_matching",
      args: {
        match_task_kind: "run_skill",
        match_skill_name: SKILL_NAME,
        match_task_action: "run_once",
        match_platforms: affectedPlatforms,
      },
      completion_required: true,
    } : null,
    side_effect_applied: true,
  });
}

async function runOnce(request, args, runtime = {}) {
  const root = storageRoot(request);
  const requested = requestedPlatforms(args, true);
  const scheduledRun = args.scheduled_run === true;
  const directOneShot = !scheduledRun && requested.length > 0;
  const configExplicit = RUN_CONFIG_FIELDS.some((field) => Object.hasOwn(args, field));
  const oneShotConfig = directOneShot ? normalizedConfig(args) : null;
  if (oneShotConfig) {
    for (const platform of requested) sourceUrls(platform, oneShotConfig);
  }
  const { run } = await beginRun(root, requested, {
    mode: directOneShot ? "one_shot" : scheduledRun ? "scheduled" : "enabled_manual",
    config: oneShotConfig,
    config_explicit: configExplicit,
  });
  if (!run) return success("run_once", { state: "disabled_or_paused", side_effect_applied: false });
  const counts = { items: 0, videos: 0, images: 0, duplicates: 0, failures: 0 };
  const progressReporter = scheduledRun
    ? createBackgroundProgressReporter({
        requestId: request?.request_id,
        run,
        counts,
        writeFrame: runtime.writeProgress,
      })
    : { emitIfDue: () => false, stop: () => {} };
  let status = "completed_batch";
  let errorCode = null;
  const leaseHeartbeat = setInterval(() => {
    heartbeat(root, run.run_id, counts).catch(() => {});
  }, 30_000);
  leaseHeartbeat.unref?.();
  try {
    const deadline = Date.now() + Math.min(
      ...run.platforms.map((platform) => run.platform_configs[platform].max_run_minutes),
    ) * 60 * 1000;
    for (const platform of run.platforms) {
      const config = run.platform_configs[platform];
      await cleanupExpiredDiagnostics(root, config.retain_diagnostics_hours);
      const remaining = Math.max(0, config.max_items_per_run - counts.items);
      if (remaining === 0) break;
      await collectPlatform({
        root,
        runId: run.run_id,
        platform,
        config,
        limit: remaining,
        shouldStop: async () => Date.now() >= deadline || (await heartbeat(root, run.run_id, counts)),
        onPage: async ({ records, temporaryPaths }) => {
          try {
            const result = await commitPageRecords(root, records);
            counts.items += 1;
            for (const record of result.committed) counts[record.kind === "video" ? "videos" : "images"] += 1;
            counts.duplicates += result.duplicateCount;
          } finally {
            await Promise.all(temporaryPaths.map((file) => fs.unlink(file).catch(() => {})));
          }
          await heartbeat(root, run.run_id, counts);
          progressReporter.emitIfDue();
        },
        onFailure: async () => {
          counts.failures += 1;
          await heartbeat(root, run.run_id, counts);
          progressReporter.emitIfDue();
        },
      });
      if (await heartbeat(root, run.run_id, counts)) {
        status = "stopped_after_current_item";
        break;
      }
    }
  } catch (error) {
    const waitingStates = {
      display_unavailable: "waiting_for_display",
      login_required: "waiting_for_login",
      rate_limited: "rate_limited",
      challenge_required: "waiting_for_login",
    };
    status = waitingStates[String(error?.message)] || "failed";
    errorCode = String(error?.message || "execution_failed");
    if (counts.failures === 0) counts.failures = 1;
    const runTemporary = path.join(root, "tmp", run.run_id);
    const diagnostic = path.join(root, "diagnostics", `${Date.now()}-${run.run_id}`);
    await fs.rename(runTemporary, diagnostic).catch(() => {});
  } finally {
    clearInterval(leaseHeartbeat);
    progressReporter.stop();
  }
  run.counts = counts;
  const completed = await finishRun(root, run, status, errorCode);
  if (status !== "failed") {
    await fs.rm(path.join(root, "tmp", run.run_id), { recursive: true, force: true }).catch(() => {});
  }
  if (status === "failed") throw new Error(errorCode);
  return success("run_once", {
    state: status,
    run: completed,
    exports: {
      videos_csv: path.join(root, "exports", "videos.csv"),
      images_csv: path.join(root, "exports", "images.csv"),
    },
    side_effect_applied: counts.videos + counts.images > 0,
  });
}

async function status(request) {
  const root = storageRoot(request);
  const state = await readState(root);
  const records = await readRecords(root);
  return success("status", {
    platforms: state.platforms,
    active_run: state.active_run,
    counts: {
      videos: records.filter((record) => record.kind === "video").length,
      images: records.filter((record) => record.kind === "image").length,
    },
  });
}

async function listRuns(request, args) {
  const state = await readState(storageRoot(request));
  const limit = integer(args.limit, 20, 1, 100);
  const offset = integer(args.offset, 0, 0, 100_000);
  return success("list_runs", {
    runs: (state.runs || []).slice(offset, offset + limit),
    offset,
    limit,
    total: (state.runs || []).length,
  });
}

async function exportResults(request) {
  const root = storageRoot(request);
  const outputDirectory = request?.context?.artifact_output_directory;
  const exported = await copyExportsTo(root, outputDirectory);
  return success("export_results", {
    counts: { videos: exported.videoCount, images: exported.imageCount, video_covers: exported.coverPaths.length },
    artifacts: [
      { kind: "file", path: exported.videoPath, media_type: "text/csv", filename: "videos.csv" },
      { kind: "file", path: exported.imagePath, media_type: "text/csv", filename: "images.csv" },
      ...exported.coverPaths.map((coverPath) => ({
        kind: "image",
        path: coverPath,
        media_type: "image/png",
        filename: path.basename(coverPath),
        artifact_role: "video_cover_screenshot",
      })),
    ],
  });
}

export async function handleRequest(request, runtime = {}) {
  const args = request?.args;
  const action = typeof args?.action === "string" ? args.action : "";
  if (!ACTIONS.has(action)) return errorResponse(action || "unknown", new Error("action_unsupported"));
  try {
    if (action === "capabilities") {
      return success(action, {
        supported_platforms: SUPPORTED_PLATFORMS,
        browser: await browserCapability(),
        capture_mode: "browser_element_screenshot",
        output_files: ["videos.csv", "images.csv"],
      });
    }
    if (action === "preview_enable") return await preview(args);
    if (action === "enable") return await enable(request, args);
    if (["disable", "pause", "resume"].includes(action)) return await control(request, args, action);
    if (action === "run_once") return await runOnce(request, args, runtime);
    if (action === "run_enabled_once") {
      return await runOnce(request, { action: "run_once", scheduled_run: true }, runtime);
    }
    if (action === "status") return await status(request);
    if (action === "stop_current") {
      const platforms = requestedPlatforms(args, true);
      const state = await requestStop(storageRoot(request), platforms);
      return success(action, {
        lifecycle_state: state.active_run?.lifecycle_state || "idle",
        drain_run_id: state.stop_after_item_run_id,
        stop_mode: state.stop_after_item_run_id ? "after_current_item" : null,
        side_effect_applied: Boolean(state.stop_after_item_run_id),
      });
    }
    if (action === "list_runs") return await listRuns(request, args);
    if (action === "export_results") return await exportResults(request);
    throw new Error("action_unsupported");
  } catch (error) {
    return errorResponse(action, error);
  }
}

function protocolResponse(request, body) {
  return { request_id: request?.request_id || "invalid", ...body };
}

const isEntrypoint = process.argv[1] && path.resolve(process.argv[1]) === path.resolve(fileURLToPath(import.meta.url));
if (isEntrypoint) {
  const lines = readline.createInterface({ input: process.stdin, crlfDelay: Infinity });
  lines.once("line", async (line) => {
    let request = null;
    let response;
    try {
      request = JSON.parse(line);
      response = protocolResponse(request, await handleRequest(request, {
        writeProgress: (frame) => process.stdout.write(`${JSON.stringify(frame)}\n`),
      }));
    } catch (error) {
      response = protocolResponse(request, errorResponse("unknown", error));
    }
    process.stdout.write(`${JSON.stringify(response)}\n`);
    lines.close();
  });
}
