import type { HostCapacitySummary, HostSystemSummary } from "../types/api";

export const HOST_SUMMARY_STALE_SECONDS = 5 * 60;

export function hostSystemTitle(summary: HostSystemSummary | null): string {
  if (!summary) return "--";
  const name = summary.os.name?.trim() || summary.os.family;
  const version = summary.os.version?.trim();
  return version && !name.includes(version) ? `${name} ${version}` : name;
}

export function hostCapacityUsedPercent(capacity: HostCapacitySummary): number | null {
  if (
    capacity.total_bytes == null ||
    capacity.available_bytes == null ||
    capacity.total_bytes <= 0
  ) {
    return null;
  }
  const used = 100 - (capacity.available_bytes / capacity.total_bytes) * 100;
  return Math.min(100, Math.max(0, Math.round(used)));
}

export function hostSummaryIsStale(
  summary: HostSystemSummary | null,
  nowSeconds = Math.floor(Date.now() / 1000),
): boolean {
  if (!summary || summary.collected_at_ts <= 0) return false;
  return nowSeconds - summary.collected_at_ts > HOST_SUMMARY_STALE_SECONDS;
}

export function hostSummaryIsPartial(summary: HostSystemSummary | null): boolean {
  return Boolean(summary?.unavailable_fields.length);
}
