import { SUPPORTED_PLATFORMS } from "./platforms.mjs";

export const ENGAGEMENT_METRICS = Object.freeze([
  "views", "reads", "likes", "comments", "favorites", "shares", "reposts", "coins", "danmaku",
]);
const OPERATORS = new Set(["gt", "gte", "lt", "lte", "eq"]);
const numericDisplay = display => display.replace(/\s*(?:次观看|次播放|次阅读|播放|阅读|观看|views?|likes?|comments?|shares?|reposts?)$/iu, "").trim();

export function metricFromDisplay(raw, source) {
  const display = String(raw ?? "").replaceAll(/\s+/gu, " ").trim();
  if (!display || display.length > 32 || /\p{Cc}/u.test(display)) return null;
  const numeric = numericDisplay(display);
  if (/^(?:0|[1-9]\d*|[1-9]\d{0,2}(?:,\d{3})+)$/u.test(numeric)) {
    const value = Number(numeric.replaceAll(",", ""));
    return Number.isSafeInteger(value) ? { display, value, ...(source ? { source } : {}) } : null;
  }
  if (!/^\d+(?:\.\d+)?\s*(?:千|万|萬|亿|億|[kmwb])\+?$/iu.test(numeric)
    && !/^\d+\+$/u.test(numeric)) return null;
  return { display, ...(source ? { source } : {}) };
}

export function exactMetric(value, source, rendered) {
  if (value === null || value === undefined || value === "") return null;
  const parsed = typeof value === "number" ? value : /^\d+$/u.test(String(value)) ? Number(value) : NaN;
  if (!Number.isSafeInteger(parsed) || parsed < 0) return null;
  return { display: rendered?.display || String(parsed), value: parsed, ...(source ? { source } : {}) };
}

function metricBounds(metric) {
  if (Number.isSafeInteger(metric?.value) && metric.value >= 0) return [metric.value, metric.value];
  const parsed = metricFromDisplay(metric?.display);
  if (!parsed) return null;
  if (parsed.value != null) return [parsed.value, parsed.value];
  const numeric = numericDisplay(parsed.display);
  const match = /^(\d+(?:\.\d+)?)\s*(千|万|萬|亿|億|[kmwb])?(\+)?$/iu.exec(numeric);
  if (!match) return null;
  const scale = { 千: 1000, 万: 10_000, 萬: 10_000, 亿: 100_000_000, 億: 100_000_000, k: 1000, w: 10_000, m: 1_000_000, b: 1_000_000_000 }[match[2]?.toLowerCase()] || 1;
  const center = Number(match[1]) * scale;
  const quantum = scale / (10 ** (match[1].split(".")[1]?.length || 0));
  if (!Number.isSafeInteger(center) || !Number.isFinite(quantum)) return null;
  // Abbreviations are not exact: conservatively cover rounding, ceiling and
  // truncation. A boundary that overlaps this interval cannot qualify a post.
  return match[3] ? [center, Number.MAX_SAFE_INTEGER]
    : [Math.max(0, Math.floor(center - quantum)), Math.min(Number.MAX_SAFE_INTEGER, Math.ceil(center + quantum) - 1)];
}

function normalizeGroup(value) {
  if (!value || typeof value !== "object" || Array.isArray(value)
    || Object.keys(value).some(key => !["all", "any"].includes(key))) throw new Error("invalid_args");
  const result = {};
  for (const key of ["all", "any"]) {
    if (value[key] === undefined) continue;
    if (!Array.isArray(value[key])) throw new Error("invalid_args");
    result[key] = value[key].map(condition => {
      if (!condition || typeof condition !== "object" || Array.isArray(condition)
        || Object.keys(condition).some(field => !["metric", "operator", "value"].includes(field))
        || !ENGAGEMENT_METRICS.includes(condition.metric) || !OPERATORS.has(condition.operator)
        || !Number.isSafeInteger(condition.value) || condition.value < 0) throw new Error("invalid_args");
      return { metric: condition.metric, operator: condition.operator, value: condition.value };
    });
  }
  return result;
}

export function normalizeEngagementFilters(value) {
  if (value === undefined) return {};
  if (!value || typeof value !== "object" || Array.isArray(value)
    || Object.keys(value).some(key => !["all", "any", "platforms"].includes(key))) throw new Error("invalid_args");
  const { platforms, ...global } = value;
  const result = normalizeGroup(global);
  if (platforms !== undefined) {
    if (!platforms || typeof platforms !== "object" || Array.isArray(platforms)
      || Object.keys(platforms).some(key => !SUPPORTED_PLATFORMS.includes(key))) throw new Error("invalid_args");
    result.platforms = Object.fromEntries(Object.entries(platforms).map(([platform, group]) => [platform, normalizeGroup(group)]));
  }
  return result;
}

function evaluateCondition(metrics, condition, availability) {
  const bounds = metricBounds(metrics?.[condition.metric]);
  if (!bounds) return { ...condition, matched: false, reason: "metric_unavailable",
    ...(availability?.[condition.metric] ? { availability: availability[condition.metric] } : {}) };
  const [low, high] = bounds;
  const value = condition.value;
  const [matched, rejected] = {
    gt: [low > value, high <= value], gte: [low >= value, high < value],
    lt: [high < value, low >= value], lte: [high <= value, low > value],
    eq: [low === high && low === value, value < low || value > high],
  }[condition.operator];
  return { ...condition, matched, reason: matched ? "matched" : rejected ? "threshold_not_met" : "metric_precision_insufficient" };
}

export function evaluateEngagementFilters(engagement, filters = {}, platform = engagement?.platform) {
  const groups = [filters, filters.platforms?.[platform]].filter(Boolean);
  const failures = [];
  const observations = [];
  for (const group of groups) {
    for (const condition of group.all || []) {
      const result = evaluateCondition(engagement?.metrics, condition, engagement?.availability);
      observations.push(result);
      if (!result.matched) failures.push(result);
    }
    if (group.any?.length) {
      const results = group.any.map(condition => evaluateCondition(engagement?.metrics, condition, engagement?.availability));
      observations.push(...results);
      if (!results.some(result => result.matched)) failures.push(...results);
    }
  }
  const reason = failures.find(result => result.reason === "threshold_not_met")?.reason
    || failures[0]?.reason || "matched";
  return { matched: failures.length === 0, reason, conditions: failures, observations };
}

export function hasEngagementFilters(config, platform) {
  const filters = config.engagement_filters || {};
  return [filters, filters.platforms?.[platform]].some(group => group?.all?.length || group?.any?.length);
}

export function rejectedEngagementResult(engagement, config, platform) {
  const decision = evaluateEngagementFilters(engagement, config.engagement_filters, platform);
  return decision.matched ? null : { skipped: true, filter_decision: decision, records: [], temporaryPaths: [] };
}
