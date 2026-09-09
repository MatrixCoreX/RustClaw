import fs from "node:fs/promises";
import path from "node:path";
import { randomUUID } from "node:crypto";

import { exportRecordCsv, writeAtomic } from "./csv.mjs";
import { resolveBrowserMode } from "./platforms.mjs";
import { recordIdentity, recordPostIdentity } from "./media_identity.mjs";
import { activeRunIsFresh, activeRuns, drainRuns, parallelPlatformLimit, projectActiveRuns } from "./run_leases.mjs";

const STATE_SCHEMA_VERSION = 1;
const LOCK_RETRY_MS = 40;
const LOCK_TIMEOUT_MS = 10_000;
const STALE_LOCK_MS = 30 * 60 * 1000;
const BACKGROUND_WORKER_STALE_MS = 2 * 60 * 1000;
const RETIRED_CONFIG_FIELDS = new Set(["interval_minutes", "recognition_mode"]);

function initialState() {
  return {
    schema_version: STATE_SCHEMA_VERSION,
    platforms: {},
    background_worker: null,
    active_runs: {},
    active_run: null,
    stop_after_item_run_id: null,
    runs: [],
    updated_at: new Date().toISOString(),
  };
}

export function storageRoot(request) {
  const storage = request?.context?.skill_storage;
  if (storage?.storage_kind !== "directory" || typeof storage.directory_path !== "string") {
    throw new Error("skill_storage_required");
  }
  if (!path.isAbsolute(storage.directory_path)) throw new Error("skill_storage_invalid");
  return storage.directory_path;
}

async function ensureLayout(root) {
  await Promise.all([
    fs.mkdir(path.join(root, "records"), { recursive: true }),
    fs.mkdir(path.join(root, "exports"), { recursive: true }),
    fs.mkdir(path.join(root, "tmp"), { recursive: true }),
    fs.mkdir(path.join(root, "diagnostics"), { recursive: true }),
    fs.mkdir(path.join(root, "browser-profile"), { recursive: true }),
  ]);
}

async function readJson(filePath, fallback) {
  try {
    return JSON.parse(await fs.readFile(filePath, "utf8"));
  } catch (error) {
    if (error?.code === "ENOENT") return fallback;
    throw error;
  }
}

async function acquireLock(root) {
  const lockPath = path.join(root, ".state.lock");
  const deadline = Date.now() + LOCK_TIMEOUT_MS;
  while (Date.now() < deadline) {
    try {
      const handle = await fs.open(lockPath, "wx", 0o600);
      await handle.writeFile(JSON.stringify({ pid: process.pid, created_at: Date.now() }));
      return { handle, lockPath };
    } catch (error) {
      if (error?.code !== "EEXIST") throw error;
      try {
        const stat = await fs.stat(lockPath);
        if (Date.now() - stat.mtimeMs > STALE_LOCK_MS) {
          await fs.unlink(lockPath);
          continue;
        }
      } catch (statError) {
        if (statError?.code !== "ENOENT") throw statError;
      }
      await new Promise((resolve) => setTimeout(resolve, LOCK_RETRY_MS));
    }
  }
  throw new Error("storage_lock_timeout");
}

async function withLock(root, operation) {
  await ensureLayout(root);
  const lock = await acquireLock(root);
  try {
    return await operation();
  } finally {
    await lock.handle.close().catch(() => {});
    await fs.unlink(lock.lockPath).catch(() => {});
  }
}

async function readStateUnlocked(root) {
  const state = await readJson(path.join(root, "state.json"), initialState());
  if (state?.schema_version !== STATE_SCHEMA_VERSION) return initialState();
  if (!Object.hasOwn(state, "background_worker")) state.background_worker = null;
  if (!Object.hasOwn(state, "active_runs")) {
    // An old binary must finish before adopting the new lease ownership model.
    if (activeRunIsFresh(state.active_run) || backgroundWorkerIsFresh(state.background_worker)) {
      throw new Error("storage_upgrade_requires_idle");
    }
    state.active_runs = {};
    state.active_run = null;
    state.stop_after_item_run_id = null;
  }
  for (const platformState of Object.values(state.platforms || {})) {
    if (platformState?.config) platformState.config = currentConfig(platformState.config);
  }
  for (const run of Object.values(state.active_runs)) {
    if (run?.platform_configs) run.platform_configs = currentPlatformConfigs(run.platform_configs);
  }
  for (const run of state.runs || []) {
    if (run?.platform_configs) run.platform_configs = currentPlatformConfigs(run.platform_configs);
  }
  return state;
}

function currentConfig(config) {
  return Object.fromEntries(
    Object.entries(config || {}).filter(([key]) => !RETIRED_CONFIG_FIELDS.has(key)),
  );
}

function currentPlatformConfigs(configs) {
  return Object.fromEntries(
    Object.entries(configs || {}).map(([platform, config]) => [platform, currentConfig(config)]),
  );
}

async function writeStateUnlocked(root, state) {
  projectActiveRuns(state);
  delete state.cancel_run_id;
  state.updated_at = new Date().toISOString();
  await writeAtomic(path.join(root, "state.json"), `${JSON.stringify(state, null, 2)}\n`);
}

export async function readState(root) {
  await ensureLayout(root);
  return readStateUnlocked(root);
}

export async function readStatusState(root) {
  const state = await readState(root);
  const expiredLeases = [];
  if (state.background_worker && !backgroundWorkerIsFresh(state.background_worker)) {
    expiredLeases.push({ kind: "background_worker", ...state.background_worker, lifecycle_state: "heartbeat_expired" });
    state.background_worker = null;
  }
  for (const [id, lease] of Object.entries(state.active_runs)) {
    if (activeRunIsFresh(lease)) continue;
    expiredLeases.push({ kind: "active_run", ...lease, lifecycle_state: "heartbeat_expired" });
    delete state.active_runs[id];
  }
  return { ...projectActiveRuns(state), expired_leases: expiredLeases };
}

export async function configurePlatforms(root, platforms, configs) {
  return withLock(root, async () => {
    const state = await readStateUnlocked(root);
    if (activeRuns(state).some(run => run.platforms.some(platform => platforms.includes(platform)))) {
      throw new Error("run_already_active");
    }
    if (platforms.some(platform => state.platforms[platform]?.enabled)) {
      throw new Error("collection_already_enabled");
    }
    const now = new Date().toISOString();
    for (const platform of platforms) {
      const config = configs[platform];
      if (!config) throw new Error("platform_not_configured");
      state.platforms[platform] = {
        ...(state.platforms[platform] || {}),
        enabled: true,
        paused: false,
        state: "enabled",
        config: { ...(state.platforms[platform]?.config || {}), ...config },
        updated_at: now,
      };
    }
    await writeStateUnlocked(root, state);
    return state;
  });
}

export function backgroundWorkerIsFresh(worker) {
  if (!worker) return false;
  const heartbeatAt = Date.parse(worker.heartbeat_at || worker.started_at || "");
  return Number.isFinite(heartbeatAt) && Date.now() - heartbeatAt < BACKGROUND_WORKER_STALE_MS;
}

export async function beginBackgroundWorker(root) {
  return withLock(root, async () => {
    const state = await readStateUnlocked(root);
    if (backgroundWorkerIsFresh(state.background_worker)) {
      throw new Error("background_worker_already_active");
    }
    const platforms = Object.keys(state.platforms)
      .filter((platform) => state.platforms[platform]?.enabled);
    if (platforms.length === 0) throw new Error("background_collection_not_enabled");
    const worker = {
      worker_id: `worker_${randomUUID()}`,
      platforms,
      started_at: new Date().toISOString(),
      heartbeat_at: new Date().toISOString(),
      lifecycle_state: "running",
      completed_batches: 0,
      counts: { items: 0, videos: 0, images: 0, duplicates: 0, failures: 0 },
      last_error_code: null,
      consecutive_failures: 0,
      retry_not_before: null,
    };
    state.background_worker = worker;
    await writeStateUnlocked(root, state);
    return worker;
  });
}

export async function heartbeatBackgroundWorker(root, workerId, update = {}) {
  return withLock(root, async () => {
    const state = await readStateUnlocked(root);
    if (state.background_worker?.worker_id !== workerId) return null;
    state.background_worker = {
      ...state.background_worker,
      ...update,
      heartbeat_at: new Date().toISOString(),
    };
    await writeStateUnlocked(root, state);
    return state.background_worker;
  });
}

export async function finishBackgroundWorker(root, workerId, lifecycleState, update = {}, options = {}) {
  return withLock(root, async () => {
    const state = await readStateUnlocked(root);
    if (state.background_worker?.worker_id !== workerId) return null;
    if (options.only_when_disabled && Object.values(state.platforms).some(platform => platform?.enabled)) return null;
    const completed = {
      ...state.background_worker,
      ...update,
      lifecycle_state: lifecycleState,
      finished_at: new Date().toISOString(),
    };
    state.background_worker = null;
    await writeStateUnlocked(root, state);
    return completed;
  });
}

export async function setPlatformControl(root, platforms, action) {
  return withLock(root, async () => {
    const state = await readStateUnlocked(root);
    const targets = platforms.length > 0 ? platforms : Object.keys(state.platforms);
    if (
      action === "resume" &&
      targets.some((platform) => !state.platforms[platform]?.config || Object.keys(state.platforms[platform].config).length === 0)
    ) {
      throw new Error("platform_not_configured");
    }
    const now = new Date().toISOString();
    for (const platform of targets) {
      const current = state.platforms[platform];
      if (!current) continue;
      if (action === "disable") {
        state.platforms[platform] = { ...current, enabled: false, paused: false, state: "disabled", updated_at: now };
      } else if (action === "pause") {
        state.platforms[platform] = { ...current, paused: true, state: "paused", updated_at: now };
      } else if (action === "resume") {
        state.platforms[platform] = {
          ...current,
          enabled: true,
          paused: false,
          state: "enabled",
          updated_at: now,
        };
      }
    }
    if (["disable", "pause"].includes(action)) drainRuns(state, targets);
    await writeStateUnlocked(root, state);
    return state;
  });
}

export async function beginRun(root, requestedPlatforms, options = {}) {
  return withLock(root, async () => {
    const state = await readStateUnlocked(root);
    const directOneShot = options.mode === "one_shot" && requestedPlatforms.length > 0;
    const platforms = directOneShot
      ? requestedPlatforms
      : (requestedPlatforms.length > 0 ? requestedPlatforms : Object.keys(state.platforms))
        .filter((platform) => state.platforms[platform]?.enabled && !state.platforms[platform]?.paused);
    if (platforms.length === 0) return { state, run: null };
    if (options.worker_id && (state.background_worker?.worker_id !== options.worker_id
      || !backgroundWorkerIsFresh(state.background_worker))) throw new Error("worker_lease_lost");
    const current = activeRuns(state);
    if (current.some(run => run.platforms.some(platform => platforms.includes(platform)))) throw new Error("run_already_active");
    const limit = options.parallel_limit || parallelPlatformLimit();
    if (current.reduce((sum, run) => sum + run.platforms.length, 0) + platforms.length > limit) {
      throw new Error("collection_capacity_busy");
    }
    const platformConfigs = Object.fromEntries(platforms.map((platform) => {
      const saved = state.platforms[platform]?.config;
      const config = directOneShot && (options.config_explicit || !saved)
        ? options.platform_configs?.[platform]
        : saved;
      if (!config) throw new Error("platform_not_configured");
      return [platform, { ...structuredClone(config), browser_mode: resolveBrowserMode(platform, config.browser_mode) }];
    }));
    const run = {
      run_id: `run_${randomUUID()}`,
      platforms,
      worker_id: options.worker_id || null,
      run_mode: directOneShot ? "one_shot" : options.mode || "enabled_manual",
      platform_configs: platformConfigs,
      browser_modes: Object.fromEntries(
        platforms.map((platform) => [platform, platformConfigs[platform].browser_mode]),
      ),
      started_at: new Date().toISOString(),
      heartbeat_at: new Date().toISOString(),
      lifecycle_state: "running",
      counts: { items: 0, videos: 0, images: 0, duplicates: 0, failures: 0 },
    };
    state.active_runs = Object.fromEntries(current.map(run => [run.run_id, run]));
    state.active_runs[run.run_id] = run;
    await writeStateUnlocked(root, state);
    return { state, run };
  });
}

export async function heartbeat(root, runId, counts, lifecycleState = null, evidence = {}) {
  return withLock(root, async () => {
    const state = await readStateUnlocked(root);
    const run = state.active_runs[runId];
    if (!run || !activeRunIsFresh(run)) return true;
    if (run.worker_id && state.background_worker?.worker_id !== run.worker_id) return true;
    run.heartbeat_at = new Date().toISOString();
    run.counts = { ...run.counts, ...counts };
    if (evidence.manual_verification) run.manual_verification = { ...evidence.manual_verification };
    if (lifecycleState && !run.stop_requested_at) run.lifecycle_state = lifecycleState;
    await writeStateUnlocked(root, state);
    return Boolean(run.stop_requested_at);
  });
}

export async function requestStop(root, platforms = []) {
  return withLock(root, async () => {
    const state = await readStateUnlocked(root);
    drainRuns(state, platforms);
    await writeStateUnlocked(root, state);
    return state;
  });
}

export async function finishRun(root, run, status, errorCode = null) {
  return withLock(root, async () => {
    const state = await readStateUnlocked(root);
    const completed = {
      ...run,
      lifecycle_state: status,
      status,
      error_code: errorCode,
      finished_at: new Date().toISOString(),
    };
    state.runs = [completed, ...(state.runs || []).filter((item) => item.run_id !== run.run_id)].slice(0, 100);
    delete state.active_runs[run.run_id];
    await writeStateUnlocked(root, state);
    return completed;
  });
}

export async function readRecords(root) {
  await ensureLayout(root);
  const directory = path.join(root, "records");
  const names = (await fs.readdir(directory)).filter((name) => /^\d{12}\.json$/u.test(name)).sort();
  const records = [];
  for (const name of names) records.push(await readJson(path.join(directory, name), null));
  return records.filter(Boolean);
}

async function directoryBytes(directory) {
  let total = 0;
  const entries = await fs.readdir(directory, { withFileTypes: true }).catch((error) => {
    if (error?.code === "ENOENT") return [];
    throw error;
  });
  for (const entry of entries) {
    const target = path.join(directory, entry.name);
    if (entry.isDirectory()) total += await directoryBytes(target);
    else if (entry.isFile()) total += (await fs.stat(target)).size;
  }
  return total;
}

export async function clearCollectedData(root) {
  return withLock(root, async () => {
    const state = await readStateUnlocked(root);
    if (backgroundWorkerIsFresh(state.background_worker)) {
      throw new Error("background_worker_already_active");
    }
    if (activeRuns(state).length > 0) throw new Error("run_already_active");
    const records = await readRecords(root);
    const removable = ["records", "exports", "tmp", "diagnostics"];
    const removedBytes = (await Promise.all(
      removable.map((name) => directoryBytes(path.join(root, name))),
    )).reduce((total, value) => total + value, 0);
    for (const name of removable) {
      await fs.rm(path.join(root, name), { recursive: true, force: true });
    }
    await ensureLayout(root);
    await exportRecordCsv(root, []);
    state.active_runs = {};
    state.runs = [];
    await writeStateUnlocked(root, state);
    return {
      records: records.length,
      videos: records.filter((record) => record.kind === "video").length,
      images: records.filter((record) => record.kind === "image").length,
      bytes: removedBytes,
    };
  });
}

function recordMaxima(records) {
  return records.reduce(
    (maxima, record) => ({
      global: Math.max(maxima.global, Number(record.global_sequence) || 0),
      video: Math.max(maxima.video, record.kind === "video" ? Number(record.sequence) || 0 : 0),
      image: Math.max(maxima.image, record.kind === "image" ? Number(record.sequence) || 0 : 0),
      post: Math.max(maxima.post, Number(record.post_sequence) || 0),
    }),
    { global: 0, video: 0, image: 0, post: 0 },
  );
}

export async function commitPageRecords(root, proposedRecords, runId = null) {
  return withLock(root, async () => {
    if (runId) {
      const state = await readStateUnlocked(root);
      const run = state.active_runs[runId];
      if (!activeRunIsFresh(run) || (run.worker_id && state.background_worker?.worker_id !== run.worker_id)) {
        throw new Error("run_lease_lost");
      }
    }
    const existing = await readRecords(root);
    const dedup = new Set(existing.map(recordIdentity));
    const postSequences = new Map(existing.filter(record => record.kind === "image")
      .map(record => [recordPostIdentity(record), record.post_sequence])
      .filter(([post, sequence]) => post && Number.isSafeInteger(sequence) && sequence > 0));
    const maxima = recordMaxima(existing);
    const committed = [];
    let duplicateCount = 0;
    let anonymousPostSequence = null;
    for (const proposal of proposedRecords) {
      const identity = recordIdentity(proposal);
      if (dedup.has(identity)) {
        duplicateCount += 1;
        continue;
      }
      maxima.global += 1;
      let postSequence = null;
      if (proposal.kind === "video") maxima.video += 1;
      else {
        maxima.image += 1;
        const post = recordPostIdentity(proposal);
        postSequence = post ? postSequences.get(post) : anonymousPostSequence;
        if (postSequence == null) {
          maxima.post += 1;
          postSequence = maxima.post;
          if (post) postSequences.set(post, postSequence);
          else anonymousPostSequence = postSequence;
        }
      }
      const record = {
        ...proposal,
        schema_version: 1,
        global_sequence: maxima.global,
        sequence: proposal.kind === "video" ? maxima.video : maxima.image,
        post_sequence: proposal.kind === "image" ? postSequence : null,
      };
      const fileName = `${String(record.global_sequence).padStart(12, "0")}.json`;
      await writeAtomic(path.join(root, "records", fileName), `${JSON.stringify(record, null, 2)}\n`);
      dedup.add(identity);
      existing.push(record);
      committed.push(record);
    }
    const exported = await exportRecordCsv(root, existing);
    return { committed, duplicateCount, exported };
  });
}

export async function rebuildExports(root) {
  return withLock(root, async () => exportRecordCsv(root, await readRecords(root)));
}

export async function copyExportsTo(root, outputDirectory) {
  const exported = await rebuildExports(root);
  const sourceCoverDirectory = path.join(root, "exports", "video_covers");
  const coverNames = await fs.readdir(sourceCoverDirectory).catch((error) => {
    if (error?.code === "ENOENT") return [];
    throw error;
  });
  const sourceCoverPaths = coverNames
    .filter((name) => name.endsWith(".png"))
    .sort()
    .map((name) => path.join(sourceCoverDirectory, name));
  if (!outputDirectory || !path.isAbsolute(outputDirectory)) {
    return { ...exported, coverPaths: sourceCoverPaths };
  }
  await fs.mkdir(outputDirectory, { recursive: true });
  const videoPath = path.join(outputDirectory, "videos.csv");
  const imagePath = path.join(outputDirectory, "images.csv");
  await fs.copyFile(exported.videoPath, videoPath);
  await fs.copyFile(exported.imagePath, imagePath);
  const coverDirectory = path.join(outputDirectory, "video_covers");
  await fs.mkdir(coverDirectory, { recursive: true });
  const coverPaths = [];
  for (const sourcePath of sourceCoverPaths) {
    const targetPath = path.join(coverDirectory, path.basename(sourcePath));
    await fs.copyFile(sourcePath, targetPath);
    coverPaths.push(targetPath);
  }
  return { ...exported, videoPath, imagePath, coverPaths };
}

export async function cleanupExpiredDiagnostics(root, retentionHours) {
  const directory = path.join(root, "diagnostics");
  await fs.mkdir(directory, { recursive: true });
  const cutoff = Date.now() - Math.max(1, retentionHours) * 60 * 60 * 1000;
  for (const name of await fs.readdir(directory)) {
    const target = path.join(directory, name);
    const stat = await fs.stat(target).catch(() => null);
    if (stat && stat.mtimeMs < cutoff) await fs.rm(target, { recursive: true, force: true });
  }
}
