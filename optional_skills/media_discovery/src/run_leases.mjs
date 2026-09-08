import os from "node:os";

export const ACTIVE_RUN_STALE_MS = 10 * 60 * 1000;

export function parallelPlatformLimit() {
  const constrained = process.constrainedMemory?.() || Infinity;
  const memory = Math.min(os.totalmem(), constrained);
  return memory < 4 * 1024 ** 3 ? 1 : memory < 8 * 1024 ** 3 ? 2 : 3;
}

export function activeRunIsFresh(run) {
  if (!run) return false;
  const heartbeat = Date.parse(run.heartbeat_at || run.started_at || "");
  return Number.isFinite(heartbeat) && Date.now() - heartbeat < ACTIVE_RUN_STALE_MS;
}

export function activeRuns(state) {
  return Object.values(state.active_runs || {}).filter(activeRunIsFresh);
}

// The host reads this bounded summary; execution always uses individual leases.
export function projectActiveRuns(state) {
  const runs = activeRuns(state);
  state.active_run = runs.length === 0 ? null : runs.length === 1 ? { ...runs[0] } : {
    run_id: runs[0].run_id,
    run_ids: runs.map(run => run.run_id),
    platforms: [...new Set(runs.flatMap(run => run.platforms))],
    lifecycle_state: runs.every(run => run.lifecycle_state === "draining") ? "draining" : "running",
    started_at: runs.map(run => run.started_at).sort()[0],
    heartbeat_at: runs.map(run => run.heartbeat_at).sort().at(-1),
    counts: Object.fromEntries(["items", "videos", "images", "duplicates", "failures"].map(key => [
      key, runs.reduce((sum, run) => sum + (Number(run.counts?.[key]) || 0), 0),
    ])),
  };
  state.stop_after_item_run_id = runs.find(run => run.stop_requested_at)?.run_id || null;
  return state;
}

export function drainRuns(state, platforms) {
  for (const run of activeRuns(state)) {
    if (platforms.length > 0 && !run.platforms.some(platform => platforms.includes(platform))) continue;
    run.lifecycle_state = "draining";
    run.stop_requested_at = new Date().toISOString();
  }
}

export async function mapBounded(values, limit, operation) {
  const results = new Array(values.length);
  let next = 0;
  await Promise.all(Array.from({ length: Math.min(limit, values.length) }, async () => {
    while (next < values.length) {
      const index = next++;
      try {
        results[index] = { value: await operation(values[index]) };
      } catch (error) {
        results[index] = { error };
      }
    }
  }));
  return results;
}
