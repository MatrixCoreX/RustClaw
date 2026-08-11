const BACKGROUND_REPORT_INTERVAL_MS = 15 * 60 * 1000;
const BACKGROUND_REPORT_DETAIL_KEY = "media_discovery.background.status";

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
    emitIfDue,
    stop: () => {
      stopped = true;
      clearInterval(timer);
    },
  };
}

export const backgroundReportIntervalMs = BACKGROUND_REPORT_INTERVAL_MS;
