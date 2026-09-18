import { browserStageError } from "./browser_diagnostics.mjs";
import { validatePlatformUrl } from "./platforms.mjs";

const contextBarriers = new WeakMap();
const BATCH_BARRIERS = new Set([
  "login_required", "challenge_required", "network_access_restricted", "rate_limited",
  "collection_stopped", "interactive_verification_cancelled", "interactive_verification_timeout",
  "search_results_restore_failed",
]);

export function stopsCollection(error) {
  return BATCH_BARRIERS.has(error?.message);
}

export function pacingDelayMs(config = {}, random = Math.random, multiplier = 1) {
  const numberOr = (value, fallback) => Number.isFinite(Number(value)) && Number(value) > 0
    ? Number(value) : fallback;
  const minimum = Math.min(5000, Math.max(200, numberOr(config.pacing_min_delay_ms, 1000)));
  const maximum = Math.min(8000, Math.max(minimum, numberOr(config.pacing_max_delay_ms, 2800)));
  const sample = Math.min(1, Math.max(0, Number(random()) || 0));
  const delay = (minimum + (maximum - minimum) * sample) * numberOr(multiplier, 1);
  // Capture/scroll multipliers must not undercut the user's minimum interval.
  return Math.round(Math.min(maximum, Math.max(minimum, delay)));
}

export function retryAfterTimestamp(value, now = Date.now()) {
  if (typeof value !== "string" || !value.trim()) return null;
  const field = value.trim();
  if (!/^\d+$/u.test(field) && !/^[A-Za-z]{3,9}(?:,|\s)/u.test(field)) return null;
  const timestamp = /^\d+$/u.test(field) ? now + Number(field) * 1000 : Date.parse(field);
  if (!Number.isSafeInteger(timestamp) || Math.abs(timestamp) > 8.64e15) return null;
  return new Date(Math.max(now, timestamp)).toISOString();
}

export function navigationAccessError(response, stage, now = Date.now()) {
  const status = response?.status?.();
  if (![401, 403, 429].includes(status)) return null;
  const error = browserStageError(status === 429 ? "rate_limited" : "challenge_required", stage);
  error.status_code = status;
  if (status === 429) {
    const headers = response.headers?.() || {};
    const value = Object.entries(headers).find(([key]) => key.toLowerCase() === "retry-after")?.[1];
    const retryAt = retryAfterTimestamp(value, now);
    if (retryAt) error.retry_after_at = retryAt;
  }
  return error;
}

// Watch the platform's own document/API responses, not unrelated ads or telemetry.
// A barrier ends this batch; waiting happens in the per-platform coordinator.
export function observePlatformBackpressure(context, platform, now = Date.now) {
  const state = { error: null };
  contextBarriers.set(context, state);
  const listener = response => {
    if (response.status() !== 429
      || !["document", "xhr", "fetch"].includes(response.request().resourceType())) return;
    try { validatePlatformUrl(platform, response.url()); } catch { return; }
    const error = navigationAccessError(response, "platform_response", now());
    const previous = Date.parse(state.error?.retry_after_at) || 0;
    const next = Date.parse(error.retry_after_at) || 0;
    if (!state.error || next > previous) state.error = error;
  };
  context.on("response", listener);
  return () => {
    context.off("response", listener);
    if (contextBarriers.get(context) === state) contextBarriers.delete(context);
  };
}

export function assertBrowserFlow(page, stage = "interaction_pacing") {
  const error = contextBarriers.get(page.context?.())?.error;
  if (error) throw Object.assign(browserStageError(error.message, stage), {
    status_code: error.status_code,
    ...(error.retry_after_at ? { retry_after_at: error.retry_after_at } : {}),
  });
}
