import fs from "node:fs/promises";
import path from "node:path";
import readline from "node:readline";
import { fileURLToPath } from "node:url";

import { browserCapability, collectPlatform, waitForInteractiveLogin } from "./browser.mjs";
import { resolveBrowserMode, sourceUrls, SUPPORTED_PLATFORMS } from "./platforms.mjs";
import { createBackgroundProgressReporter } from "./progress.mjs";
import { activeRuns, mapBounded, parallelPlatformLimit } from "./run_leases.mjs";
import {
  beginBackgroundWorker,
  backgroundWorkerIsFresh,
  beginRun,
  clearCollectedData,
  cleanupExpiredDiagnostics,
  commitPageRecords,
  configurePlatforms,
  copyExportsTo,
  finishRun,
  finishBackgroundWorker,
  heartbeat,
  heartbeatBackgroundWorker,
  readRecords,
  readState,
  readStatusState,
  requestStop,
  setPlatformControl,
  storageRoot,
} from "./storage.mjs";

const SKILL_NAME = "media_discovery";
const ERROR_CODES = new Set([
  "action_unsupported",
  "browser_missing",
  "browser_mode_invalid",
  "background_collection_not_enabled",
  "background_worker_already_active",
  "collection_already_enabled",
  "collection_capacity_busy",
  "partial_collection_failed",
  "run_lease_lost",
  "worker_lease_lost",
  "storage_upgrade_requires_idle",
  "challenge_required",
  "confirmation_required",
  "display_unavailable",
  "invalid_args",
  "interactive_verification_cancelled",
  "interactive_verification_timeout",
  "login_required",
  "network_access_restricted",
  "media_element_not_found",
  "media_not_ready",
  "no_items_collected",
  "platform_required",
  "platform_not_configured",
  "platform_unsupported",
  "rate_limited",
  "run_already_active",
  "screenshot_empty",
  "screenshot_obscured",
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
  "clear_results",
]);

const CONTINUOUS_RETRYABLE_ERROR_CODES = new Set([
  "network_access_restricted",
  "challenge_required",
  "display_unavailable",
  "login_required",
  "media_element_not_found",
  "media_not_ready",
  "no_items_collected",
  "rate_limited",
  "selector_drift",
]);
const RETRY_BACKOFF_MS = Object.freeze({
  network_access_restricted: Object.freeze({ base: 30 * 60 * 1000, maximum: 6 * 60 * 60 * 1000 }),
  challenge_required: Object.freeze({ base: 15 * 60 * 1000, maximum: 6 * 60 * 60 * 1000 }),
  login_required: Object.freeze({ base: 5 * 60 * 1000, maximum: 30 * 60 * 1000 }),
  rate_limited: Object.freeze({ base: 10 * 60 * 1000, maximum: 2 * 60 * 60 * 1000 }),
});
const RUN_CONFIG_FIELDS = Object.freeze([
  "source_mode",
  "topics",
  "seed_urls",
  "max_items_per_run",
  "max_images_per_post",
  "max_run_minutes",
  "max_scrolls_per_source",
  "rest_min_seconds",
  "rest_max_seconds",
  "retain_diagnostics_hours",
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

export function normalizedConfig(args, platform = args.platform) {
  const sourceMode = String(args.source_mode || "home_feed");
  if (!new Set(["home_feed", "topics", "seed_urls"]).has(sourceMode)) throw new Error("source_mode_invalid");
  const browserMode = resolveBrowserMode(platform, args.browser_mode);
  const pacingMinDelayMs = integer(args.pacing_min_delay_ms, 1000, 200, 5000);
  const pacingMaxDelayMs = integer(args.pacing_max_delay_ms, 2800, 200, 8000);
  if (pacingMaxDelayMs < pacingMinDelayMs) throw new Error("invalid_args");
  const config = {
    source_mode: sourceMode,
    topics: Array.isArray(args.topics) ? args.topics.map(String).map((value) => value.trim()).filter(Boolean) : [],
    seed_urls: Array.isArray(args.seed_urls) ? args.seed_urls.map(String) : [],
    max_items_per_run: integer(args.max_items_per_run, 5, 1, 100),
    max_images_per_post: integer(args.max_images_per_post, 100, 1, 100),
    max_run_minutes: integer(args.max_run_minutes, 30, 5, 180),
    max_scrolls_per_source: integer(args.max_scrolls_per_source, 10, 1, 100),
    rest_min_seconds: integer(args.rest_min_seconds, 180, 5, 3600),
    rest_max_seconds: integer(args.rest_max_seconds, 420, 5, 7200),
    retain_diagnostics_hours: integer(args.retain_diagnostics_hours, 24, 1, 168),
    browser_mode: browserMode,
    pacing_min_delay_ms: pacingMinDelayMs,
    pacing_max_delay_ms: pacingMaxDelayMs,
    capture_mode: "browser_element_screenshot",
  };
  if (config.rest_max_seconds < config.rest_min_seconds) throw new Error("invalid_args");
  return config;
}

export function backgroundRestDelayMs(configs, random = Math.random) {
  const values = Object.values(configs || {});
  const minimum = values.length > 0
    ? Math.max(...values.map((config) => Number(config?.rest_min_seconds) || 180))
    : 180;
  const maximum = Math.max(minimum, values.length > 0
    ? Math.min(...values.map((config) => Number(config?.rest_max_seconds) || 420))
    : 420);
  const sample = Math.min(0.999999, Math.max(0, Number(random()) || 0));
  return Math.round((minimum + (maximum - minimum) * sample) * 1000);
}

export function backgroundRetryDelayMs(
  configs,
  errorCode,
  consecutiveFailures = 1,
  random = Math.random,
) {
  const ordinaryRest = backgroundRestDelayMs(configs, random);
  const policy = RETRY_BACKOFF_MS[errorCode];
  if (!policy) return ordinaryRest;
  const exponent = Math.max(0, Math.min(8, Number(consecutiveFailures) - 1));
  const backoff = Math.min(policy.maximum, policy.base * (2 ** exponent));
  const sample = Math.min(0.999999, Math.max(0, Number(random()) || 0));
  const jittered = Math.round(backoff * (0.85 + sample * 0.3));
  return Math.max(ordinaryRest, jittered);
}

function waitingLifecycleState(errorCode) {
  return {
    network_access_restricted: "waiting_for_network_access",
    challenge_required: "waiting_for_challenge_resolution",
    login_required: "waiting_for_login",
    rate_limited: "rate_limited",
  }[errorCode] || "resting";
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
    "background_collection_not_enabled",
    "login_required",
    "no_items_collected",
    "challenge_required",
    "rate_limited",
    "storage_lock_timeout",
    "collection_capacity_busy",
  ]).has(errorCode);
  const preDispatch = new Set([
    "action_unsupported",
    "browser_mode_invalid",
    "collection_already_enabled",
    "collection_capacity_busy",
    "worker_lease_lost",
    "storage_upgrade_requires_idle",
    "confirmation_required",
    "invalid_args",
    "platform_required",
    "platform_not_configured",
    "platform_unsupported",
    "run_already_active",
    "background_worker_already_active",
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

function backgroundStartSpec() {
  return {
    capability: "media_discovery.run_enabled_once",
    args: {},
    completion_required: true,
  };
}

async function preview(args) {
  const platforms = requestedPlatforms(args);
  const configs = platformConfigs(args, platforms);
  return success("preview_enable", {
    platforms,
    ...(platforms.length === 1 ? { config: configs[platforms[0]] } : {}),
    platform_configs: configs,
    browser: await browserCapability(),
    background_start_spec: backgroundStartSpec(),
    side_effect_applied: false,
  });
}

function platformConfigs(args, platforms) {
  return Object.fromEntries(platforms.map(platform => {
    const config = normalizedConfig(args, platform);
    sourceUrls(platform, config);
    return [platform, config];
  }));
}

async function enable(request, args) {
  if (args.confirm !== true) throw new Error("confirmation_required");
  const root = storageRoot(request);
  const platforms = requestedPlatforms(args);
  const configs = platformConfigs(args, platforms);
  const state = await configurePlatforms(root, platforms, configs);
  const workerActive = backgroundWorkerIsFresh(state.background_worker);
  return success("enable", {
    platforms,
    platform_states: state.platforms,
    background_worker_active: workerActive,
    background_start_spec: backgroundStartSpec(),
    next_capability: "media_discovery.run_enabled_once",
    ...(workerActive ? { background_worker: state.background_worker } : {}),
    side_effect_applied: true,
  });
}

async function control(request, args, action) {
  const root = storageRoot(request);
  const platforms = requestedPlatforms(args, true);
  const state = await setPlatformControl(root, platforms, action);
  const affectedPlatforms = platforms.length > 0 ? platforms : Object.keys(state.platforms);
  const draining = activeRuns(state).filter(run => run.stop_requested_at
    && run.platforms.some(platform => affectedPlatforms.includes(platform)));
  return success(action, {
    platforms: affectedPlatforms,
    platform_states: state.platforms,
    lifecycle_state: draining.length ? "draining" : "idle",
    drain_run_id: draining[0]?.run_id || null,
    drain_run_ids: draining.map(run => run.run_id),
    stop_mode: draining.length ? "after_current_item" : null,
    side_effect_applied: true,
  });
}

async function runOnce(request, args, runtime = {}) {
  const state = await readState(storageRoot(request));
  const requested = requestedPlatforms(args, true);
  const platforms = requested.length ? requested : Object.keys(state.platforms)
    .filter(platform => state.platforms[platform]?.enabled && !state.platforms[platform]?.paused);
  if (platforms.length <= 1 || runtime.backgroundPlatform) return runPlatformBatch(request, args, runtime);
  const results = await mapBounded(platforms, runtime.parallelLimit || parallelPlatformLimit(), platform => (
    runPlatformBatch(request, { ...args, platforms: [platform] }, runtime)
  ));
  const counts = { items: 0, videos: 0, images: 0, duplicates: 0, failures: 0 };
  const runs = results.map((result, index) => {
    const run = result.value?.extra?.run || result.error?.extra?.run || {
      platforms: [platforms[index]], status: "failed", error_code: result.error?.message || "execution_failed",
    };
    addCounts(counts, run.counts);
    return run;
  });
  const extra = { platforms, runs, counts, side_effect_applied: counts.videos + counts.images > 0 };
  if (runs.some(run => run.error_code)) {
    throw Object.assign(new Error("partial_collection_failed"), { extra });
  }
  return success("run_once", { ...extra, state: runs.some(run => run.status === "stopped_after_current_item")
    ? "stopped_after_current_item" : "completed_batch" });
}

async function runPlatformBatch(request, args, runtime = {}) {
  const root = storageRoot(request);
  const requested = runtime.backgroundPlatform ? [runtime.backgroundPlatform] : requestedPlatforms(args, true);
  const directOneShot = requested.length > 0 && !runtime.backgroundPlatform;
  const configExplicit = RUN_CONFIG_FIELDS.some((field) => Object.hasOwn(args, field));
  const oneShotConfigs = directOneShot ? platformConfigs(args, requested) : null;
  const { run } = await beginRun(root, requested, {
    mode: directOneShot ? "one_shot" : "enabled_background",
    platform_configs: oneShotConfigs,
    config_explicit: configExplicit,
    worker_id: runtime.workerId,
    parallel_limit: runtime.parallelLimit,
  });
  if (!run) return success("run_once", { state: "disabled_or_paused", side_effect_applied: false });
  const counts = { items: 0, videos: 0, images: 0, duplicates: 0, failures: 0 };
  const captureSummary = { records_saved: 0, captions_saved: 0, covers_saved: 0, engagement_metrics: [] };
  const progressReporter = { emitIfDue: () => false, stop: () => {} };
  let status = "completed_batch";
  let errorCode = null;
  let failureDiagnostic = null;
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
      const collectionRequest = {
        root,
        runId: run.run_id,
        platform,
        config,
        limit: remaining,
        shouldStop: async () => Boolean(runtime.shouldShutdown?.()) || Date.now() >= deadline || (await heartbeat(root, run.run_id, counts)),
        onPage: async ({ records, temporaryPaths }) => {
          try {
            const result = await commitPageRecords(root, records, run.run_id);
            counts.items += 1;
            for (const record of result.committed) counts[record.kind === "video" ? "videos" : "images"] += 1;
            for (const record of result.committed) {
              captureSummary.records_saved += 1;
              if (record.platform_text) captureSummary.captions_saved += 1;
              if (record.cover_screenshot_path || record.image_screenshot_path) captureSummary.covers_saved += 1;
              captureSummary.engagement_metrics = [...new Set([
                ...captureSummary.engagement_metrics, ...Object.keys(record.engagement?.metrics || {}),
              ])].sort();
            }
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
      };
      const collect = runtime.collectPlatform || collectPlatform;
      try {
        await collect(collectionRequest);
      } catch (error) {
        const errorCode = String(error?.message || "execution_failed");
        const interactiveLoginAllowed = config.browser_mode === "silent"
          && ["login_required", "challenge_required"].includes(errorCode);
        if (!interactiveLoginAllowed) throw error;

        await heartbeat(root, run.run_id, counts, waitingLifecycleState(errorCode));
        const loginResult = await (runtime.waitForInteractiveLogin || waitForInteractiveLogin)({
          root,
          platform,
          config,
          errorCode,
          shouldStop: collectionRequest.shouldStop,
          timeoutMs: Math.max(1000, Math.min(10 * 60 * 1000, deadline - Date.now())),
        });
        if (!loginResult?.ready) {
          throw new Error(loginResult?.error_code || errorCode);
        }
        const remainingAfterLogin = Math.max(0, config.max_items_per_run - counts.items);
        if (remainingAfterLogin > 0 && !(await collectionRequest.shouldStop())) {
          await heartbeat(root, run.run_id, counts, "running");
          await collect({ ...collectionRequest, limit: remainingAfterLogin });
        }
      }
      if (await heartbeat(root, run.run_id, counts)) {
        status = "stopped_after_current_item";
        break;
      }
    }
  } catch (error) {
    if (["interactive_verification_cancelled", "interactive_verification_timeout"].includes(error?.message)) {
      await setPlatformControl(root, run.platforms, "pause");
    }
    const waitingStates = {
      collection_stopped: "stopped_after_current_item",
      display_unavailable: "waiting_for_display",
      login_required: "waiting_for_login",
      network_access_restricted: "waiting_for_network_access",
      interactive_verification_cancelled: "waiting_for_manual_verification",
      interactive_verification_timeout: "waiting_for_manual_verification",
      rate_limited: "rate_limited",
      challenge_required: "waiting_for_challenge_resolution",
    };
    status = waitingStates[String(error?.message)] || "failed";
    errorCode = status === "stopped_after_current_item" ? null : String(error?.message || "execution_failed");
    failureDiagnostic = error?.discovery_diagnostic || null;
    if (errorCode && counts.failures === 0) counts.failures = 1;
    const runTemporary = path.join(root, "tmp", run.run_id);
    const diagnostic = path.join(root, "diagnostics", `${Date.now()}-${run.run_id}`);
    await fs.rename(runTemporary, diagnostic).catch(() => {});
  } finally {
    clearInterval(leaseHeartbeat);
    progressReporter.stop();
  }
  if (status === "completed_batch" && counts.items === 0) {
    status = "failed";
    errorCode = "no_items_collected";
    if (counts.failures === 0) counts.failures = 1;
  }
  run.counts = counts;
  run.capture_summary = captureSummary;
  if (failureDiagnostic) run.failure_diagnostic = failureDiagnostic;
  const completed = await finishRun(root, run, status, errorCode);
  if (status !== "failed") {
    await fs.rm(path.join(root, "tmp", run.run_id), { recursive: true, force: true }).catch(() => {});
  }
  if (status === "failed") {
    throw Object.assign(new Error(errorCode), {
      extra: { run_id: run.run_id, run: completed,
        ...(failureDiagnostic ? { failure_diagnostic: failureDiagnostic } : {}) },
    });
  }
  return success("run_once", {
    state: status,
    run: completed,
    exports: {
      storage: "local_persistent_csv",
      delivery_requested: false,
      videos_csv: path.join(root, "exports", "videos.csv"),
      images_csv: path.join(root, "exports", "images.csv"),
    },
    side_effect_applied: counts.videos + counts.images > 0,
  });
}

function addCounts(target, source = {}) {
  for (const key of ["items", "videos", "images", "duplicates", "failures"]) {
    target[key] += Number(source[key]) || 0;
  }
}

async function runContinuous(request, runtime = {}) {
  const root = storageRoot(request);
  let worker;
  try {
    worker = await beginBackgroundWorker(root);
  } catch (error) {
    if (error?.message !== "background_worker_already_active") throw error;
    return success("run_enabled_once", { state: "already_running",
      background_worker: (await readStatusState(root)).background_worker, side_effect_applied: false });
  }
  const counts = { items: 0, videos: 0, images: 0, duplicates: 0, failures: 0 };
  const sleep = runtime.sleep || ((milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds)));
  const now = runtime.now || Date.now;
  const platformOutcomes = {};
  const inFlight = new Map();
  const fatalErrors = new Map();
  const limit = runtime.parallelLimit || parallelPlatformLimit();
  const progressRun = { run_id: worker.worker_id, platforms: worker.platforms };
  let shutdown = false;
  let coordinatorError = null;
  const reporter = createBackgroundProgressReporter({
    requestId: request?.request_id,
    run: progressRun,
    counts,
    writeFrame: runtime.writeProgress,
  });
  let completedBatches = 0;
  let attemptedBatches = 0;
  let lastErrorCode = null;
  const snapshot = () => ({
    completed_batches: completedBatches, counts: { ...counts }, last_error_code: lastErrorCode,
    platform_outcomes: structuredClone(platformOutcomes), parallel_limit: limit,
    current_platforms: [...inFlight.keys()],
  });
  const workerHeartbeat = setInterval(() => {
    heartbeatBackgroundWorker(root, worker.worker_id, snapshot()).catch(error => {
      coordinatorError = error;
      shutdown = true;
    });
  }, 30_000);
  workerHeartbeat.unref?.();
  const runPlatform = async (platform, platformState) => {
    const outcome = platformOutcomes[platform] || {
      completed_batches: 0, consecutive_failures: 0,
      counts: { items: 0, videos: 0, images: 0, duplicates: 0, failures: 0 },
    };
    platformOutcomes[platform] = outcome;
    outcome.lifecycle_state = "running";
    let errorCode = null;
    try {
      const result = await runPlatformBatch(request, { action: "run_once" }, {
        ...runtime, backgroundPlatform: platform, workerId: worker.worker_id,
        shouldShutdown: () => shutdown, writeProgress: undefined,
      });
      addCounts(counts, result.extra?.run?.counts);
      addCounts(outcome.counts, result.extra?.run?.counts);
      if (["completed_batch", "stopped_after_current_item"].includes(result.extra?.state)) {
        completedBatches += 1;
        outcome.completed_batches += 1;
        outcome.consecutive_failures = 0;
        fatalErrors.delete(platform);
      } else {
        errorCode = result.extra?.run?.error_code || result.extra?.state || "execution_failed";
        outcome.consecutive_failures += 1;
      }
    } catch (error) {
      errorCode = String(error?.message || "execution_failed");
      outcome.consecutive_failures += 1;
      const partial = error.extra?.run?.counts || { failures: 1 };
      addCounts(counts, partial);
      addCounts(outcome.counts, partial);
      if (!["collection_capacity_busy", "run_already_active"].includes(errorCode)
        && !CONTINUOUS_RETRYABLE_ERROR_CODES.has(errorCode)) {
        fatalErrors.set(platform, error);
        await setPlatformControl(root, [platform], "pause");
      }
    }
    lastErrorCode = errorCode;
    const delay = backgroundRetryDelayMs({ [platform]: platformState.config || {} },
      errorCode, outcome.consecutive_failures, runtime.random || Math.random);
    Object.assign(outcome, {
      lifecycle_state: fatalErrors.has(platform) ? "failed" : waitingLifecycleState(errorCode),
      last_error_code: errorCode,
      retry_not_before: new Date(now() + delay).toISOString(),
    });
    reporter.emitIfDue();
  };
  try {
    while (!shutdown) {
      const state = await readState(root);
      if (state.background_worker?.worker_id !== worker.worker_id) { shutdown = true; break; }
      const enabledEntries = Object.entries(state.platforms)
        .filter(([, platform]) => platform?.enabled);
      progressRun.platforms = enabledEntries.map(([platform]) => platform);
      if (enabledEntries.length === 0 && inFlight.size === 0) {
        if (fatalErrors.size) break;
        const completed = await finishBackgroundWorker(root, worker.worker_id, "stopped", snapshot(), { only_when_disabled: true });
        if (completed) return success("run_enabled_once", { state: "stopped",
          background_worker: completed, side_effect_applied: counts.items > 0 });
        continue;
      }
      const activeEntries = enabledEntries.filter(([, platform]) => !platform?.paused);
      if (activeEntries.length === 0 && inFlight.size === 0 && fatalErrors.size) break;
      const dueAt = ([platform]) => platformOutcomes[platform]
        ? Date.parse(platformOutcomes[platform].retry_not_before) : -Infinity;
      const occupied = new Set(activeRuns(state).flatMap(run => run.platforms));
      const readyEntries = activeEntries.filter(entry => !inFlight.has(entry[0])
        && !occupied.has(entry[0]) && dueAt(entry) <= now())
        .sort((left, right) => dueAt(left) - dueAt(right));
      for (const [platform, platformState] of readyEntries) {
        if (inFlight.size >= limit || occupied.size >= limit
          || (Number.isInteger(runtime.maxContinuousCycles) && attemptedBatches >= runtime.maxContinuousCycles)) break;
        attemptedBatches += 1;
        occupied.add(platform);
        const job = runPlatform(platform, platformState).catch(error => {
          coordinatorError = error; shutdown = true;
        }).finally(() => inFlight.delete(platform));
        inFlight.set(platform, job);
      }
      await heartbeatBackgroundWorker(root, worker.worker_id, {
        ...snapshot(), platforms: progressRun.platforms,
        lifecycle_state: inFlight.size ? "running" : activeEntries.length ? "resting" : "paused",
      });
      if (inFlight.size === 0 && Number.isInteger(runtime.maxContinuousCycles)
        && attemptedBatches >= runtime.maxContinuousCycles) break;
      await Promise.race([...inFlight.values(), sleep(1000)]);
    }
    await Promise.all(inFlight.values());
    if (coordinatorError) throw coordinatorError;
    if (fatalErrors.size) throw fatalErrors.values().next().value;
    const completed = await finishBackgroundWorker(root, worker.worker_id, "stopped", snapshot());
    return success("run_enabled_once", {
      state: "stopped",
      background_worker: completed,
      side_effect_applied: counts.items > 0,
    });
  } catch (error) {
    shutdown = true;
    await Promise.all(inFlight.values());
    await finishBackgroundWorker(root, worker.worker_id, "failed", snapshot()).catch(() => {});
    error.extra = { ...error.extra, ...snapshot() };
    throw error;
  } finally {
    clearInterval(workerHeartbeat);
    reporter.stop();
  }
}

async function status(request) {
  const root = storageRoot(request);
  const state = await readStatusState(root);
  const records = await readRecords(root);
  return success("status", {
    platforms: state.platforms,
    background_worker: state.background_worker,
    active_run: state.active_run,
    active_runs: state.active_runs,
    parallel_limit: parallelPlatformLimit(),
    expired_leases: state.expired_leases,
    latest_run: state.runs?.[0] || null,
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

async function clearResults(request, args) {
  if (args.confirm !== true) throw new Error("confirmation_required");
  const cleared = await clearCollectedData(storageRoot(request));
  return success("clear_results", {
    cleared,
    side_effect_applied: cleared.records > 0 || cleared.bytes > 0,
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
      return await runContinuous(request, runtime);
    }
    if (action === "status") return await status(request);
    if (action === "stop_current") {
      const platforms = requestedPlatforms(args, true);
      const state = await requestStop(storageRoot(request), platforms);
      const draining = activeRuns(state).filter(run => run.stop_requested_at
        && (platforms.length === 0 || run.platforms.some(platform => platforms.includes(platform))));
      return success(action, {
        lifecycle_state: draining.length ? "draining" : "idle",
        drain_run_id: draining[0]?.run_id || null,
        drain_run_ids: draining.map(run => run.run_id),
        stop_mode: draining.length ? "after_current_item" : null,
        side_effect_applied: draining.length > 0,
      });
    }
    if (action === "list_runs") return await listRuns(request, args);
    if (action === "export_results") return await exportResults(request);
    if (action === "clear_results") return await clearResults(request, args);
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
