import { ENGAGEMENT_METRICS } from "./engagement.mjs";

export function metricAvailability(platform, metrics, supported, contentType = "unknown", notProvided = {}) {
  return Object.fromEntries(ENGAGEMENT_METRICS.map(metric => {
    const status = metrics[metric] ? "available"
      : platform === "toutiao" && ((contentType === "article" && metric === "views")
        || (contentType === "video" && metric === "reads")) ? "not_applicable"
      : notProvided[metric] ? "not_provided"
      : supported.includes(metric) ? "not_observed" : "adapter_unsupported";
    return [metric, { status, retryable: status === "not_observed",
      ...(status === "not_provided" ? { source: notProvided[metric] } : {}),
      ...(status === "available" ? { precision: metrics[metric].value == null ? "approximate" : "exact" } : {}),
    }];
  }));
}

export function requestedMetricSupport(filters = {}, platform, supported) {
  const groups = [filters, filters.platforms?.[platform]].filter(Boolean);
  const requested = [...new Set(groups.flatMap(group => [...(group.all || []), ...(group.any || [])]).map(rule => rule.metric))];
  const unsupported = requested.filter(metric => !supported.includes(metric));
  const evaluable = groups.every(group => (group.all || []).every(rule => supported.includes(rule.metric))
    && (!group.any?.length || group.any.some(rule => supported.includes(rule.metric))));
  return { platform, requested_metrics: requested, supported_metrics: supported,
    unsupported_metrics: unsupported, evaluable,
    platform_public_availability: "not_inferred", source: "current_adapter_coverage" };
}

export function recordFilterDiagnostics(report, platform, decision) {
  const seen = new Set();
  for (const observation of decision.observations || decision.conditions || []) {
    if (!["metric_unavailable", "metric_precision_insufficient"].includes(observation.reason)) continue;
    const status = observation.availability?.status || (observation.reason === "metric_precision_insufficient"
      ? "precision_insufficient" : "not_observed");
    const key = `${platform}:${observation.metric}:${status}`;
    if (seen.has(key)) continue;
    seen.add(key);
    const entry = report.find(item => item.platform === platform && item.metric === observation.metric && item.status === status);
    if (entry) entry.posts += 1;
    else report.push({ platform, metric: observation.metric, status, posts: 1,
      retryable: status === "not_observed", blocks_match: !decision.matched });
    if (entry && !decision.matched) entry.blocks_match = true;
  }
}
