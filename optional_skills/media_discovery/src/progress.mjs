const BACKGROUND_REPORT_INTERVAL_MS = 15 * 60 * 1000;
const BACKGROUND_REPORT_DETAIL_KEY = "media_discovery.background.status";

export function createCollectionStartReporter({ requestId, writeFrame, continuous = false,
  nextSequence = () => 1, platforms }) {
  let started = false;
  return run => {
    if (started || !requestId || typeof writeFrame !== "function") return false;
    started = true;
    const config = run.platform_configs?.[run.platforms[0]] || {};
    writeFrame({
      schema_version: 1, record_type: "skill_progress", request_id: requestId,
      sequence: nextSequence(), kind: "progress", detail_key: "media_discovery.collection.started",
      params: {
        notification_delivery: "runtime", notification_renderer: "model", notification_event: "started",
        continuous, platforms: platforms || [...run.platforms],
        requested_items: continuous ? 0 : config.max_items_per_run || 0,
        max_run_minutes: continuous ? 0 : config.max_run_minutes || 0,
        stop_capability: continuous ? "media_discovery.disable" : "media_discovery.stop_current",
        stop_after_current_item: true,
      },
    });
    return true;
  };
}

function nonNegativeInteger(value) {
  const parsed = Number(value);
  return Number.isFinite(parsed) && parsed >= 0 ? Math.floor(parsed) : 0;
}

export function backgroundProgressFrame({ requestId, sequence, run, counts, elapsedMs }) {
  return {
    schema_version: 1,
    record_type: "skill_progress",
    request_id: requestId,
    sequence,
    kind: "heartbeat",
    detail_key: BACKGROUND_REPORT_DETAIL_KEY,
    params: {
      notification_delivery: "runtime",
      notification_interval_seconds: BACKGROUND_REPORT_INTERVAL_MS / 1000,
      message_key: "channel.notice.media_discovery_background_progress",
      run_id: run.run_id,
      platforms: [...run.platforms],
      elapsed_minutes: Math.max(15, Math.floor(nonNegativeInteger(elapsedMs) / 60_000)),
      items: nonNegativeInteger(counts.items),
      videos: nonNegativeInteger(counts.videos),
      images: nonNegativeInteger(counts.images),
      duplicates: nonNegativeInteger(counts.duplicates),
      failures: nonNegativeInteger(counts.failures),
    },
  };
}

export function createBackgroundProgressReporter({
  requestId,
  run,
  counts,
  writeFrame,
  now = () => Date.now(),
  intervalMs = BACKGROUND_REPORT_INTERVAL_MS,
}) {
  if (typeof writeFrame !== "function" || typeof requestId !== "string" || !requestId.trim()) {
    return { emitIfDue: () => false, stop: () => {} };
  }

  const startedAt = now();
  let nextReportAt = startedAt + intervalMs;
  let sequence = 0;
  let stopped = false;
  const start = createCollectionStartReporter({ requestId, writeFrame, continuous: true,
    nextSequence: () => ++sequence });

  const emitIfDue = () => {
    const observedAt = now();
    if (stopped || observedAt < nextReportAt) return false;
    sequence += 1;
    writeFrame(backgroundProgressFrame({
      requestId,
      sequence,
      run,
      counts,
      elapsedMs: observedAt - startedAt,
    }));
    while (nextReportAt <= observedAt) nextReportAt += intervalMs;
    return true;
  };

  const timer = setInterval(emitIfDue, intervalMs);
  timer.unref?.();
  return {
    start: () => start(run),
    emitIfDue,
    stop: () => {
      stopped = true;
      clearInterval(timer);
    },
  };
}

export const backgroundReportIntervalMs = BACKGROUND_REPORT_INTERVAL_MS;
