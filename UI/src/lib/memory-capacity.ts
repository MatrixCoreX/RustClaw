import type { MemorySettingsResult } from "../types/api";

export const capacityDecisions = ["available", "storage_paused", "storage_state_missing", "storage_state_unknown",
  "principal_quota_missing", "principal_quota_invalid", "principal_rows_exhausted", "principal_bytes_exhausted",
  "principal_background_cost_exhausted"] as const;
export type CapacityDecision = typeof capacityDecisions[number];
export type ByteMeasurement = { status: "measured"; bytes: string }
  | { status: "unavailable"; reason: string } | { status: "admin_only" };
export interface MemoryCapacity {
  schema_version: 1;
  sampled_at_ts: number;
  settings: MemorySettingsResult;
  capacity_decision: CapacityDecision;
  quota: { scope: "source_transcript_content_rows_and_bytes"; decision: CapacityDecision;
    counters: null | Record<"used_rows" | "max_rows" | "used_bytes" | "max_bytes"
      | "spent_and_pending_logical_microunits" | "max_logical_microunits", string> };
  pressure: { state: string; measurement_scope: "main_sqlite_allocated_pages_not_memory_domain_or_host_free_space";
    observed_at_ts: number | null; revision: number | null;
    observed_bytes: ByteMeasurement; configured_soft_limit_bytes: ByteMeasurement };
  measurements: Record<"own_source_transcript_payload_bytes" | "main_sqlite_allocated_page_bytes"
    | "memory_domain_assigned_btree_bytes" | "sqlite_freelist_page_bytes"
    | "database_path_filesystem_available_bytes", ByteMeasurement>;
}

const record = (value: unknown): value is Record<string, unknown> => Boolean(value) && typeof value === "object" && !Array.isArray(value);
const integer = (value: unknown): value is number => typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
const decimal = (value: unknown): value is string => typeof value === "string" && /^(0|[1-9][0-9]*)$/.test(value);
const oneOf = (value: unknown, values: readonly string[]) => typeof value === "string" && values.includes(value);

function settingsValid(value: unknown): value is MemorySettingsResult {
  if (!record(value) || !record(value.requested)) return false;
  return value.schema_version === 1 && oneOf(value.scope, ["principal", "conversation"])
    && typeof value.target_principal_id === "string" && Boolean(value.target_principal_id.trim())
    && (value.conversation_id == null || typeof value.conversation_id === "string")
    && [value.requested.use_mode, value.requested.generate_mode].every(mode => oneOf(mode, ["inherit", "enabled", "disabled"]))
    && oneOf(value.requested.external_context_policy, ["inherit", "exclude", "evidence_only", "allow"])
    && oneOf(value.external_context_policy, ["inherit", "exclude", "evidence_only", "allow"])
    && [value.use_memory, value.generate_memory, value.restart_required].every(flag => typeof flag === "boolean")
    && [value.use_source, value.generate_source, value.external_context_source, value.policy_digest].every(text => typeof text === "string" && Boolean(text))
    && (value.managed_deny_reason == null || typeof value.managed_deny_reason === "string") && integer(value.revision);
}

export function capacityPolicyKey(value: MemorySettingsResult): string {
  return JSON.stringify([value.schema_version, value.scope, value.target_principal_id, value.conversation_id ?? null,
    value.requested.use_mode, value.requested.generate_mode, value.requested.external_context_policy,
    value.use_memory, value.generate_memory, value.external_context_policy,
    value.use_source, value.generate_source, value.external_context_source, value.managed_deny_reason ?? null,
    value.revision, value.policy_digest, value.restart_required]);
}

export function capacitySelection(settings: MemorySettingsResult | null, scope: "principal" | "conversation", conversation: string | null) {
  if (!settingsValid(settings) || settings.scope !== scope) return null;
  if (scope === "conversation" && (!conversation?.trim() || settings.conversation_id !== conversation)) return null;
  if (scope === "principal" && settings.conversation_id != null) return null;
  const query = new URLSearchParams({ scope });
  if (scope === "conversation") query.set("conversation_id", conversation!);
  return { key: capacityPolicyKey(settings), path: `/v1/memory/availability?${query}` };
}

function measurement(value: unknown): value is ByteMeasurement {
  return record(value) && (value.status === "measured" ? decimal(value.bytes)
    : value.status === "admin_only" || (value.status === "unavailable" && oneOf(value.reason,
      ["query_failed", "invalid_value", "unsupported_dbstat", "in_memory_database", "filesystem_unavailable", "not_sampled"])));
}

/** Validate the full observation and the exact owner/scope/policy represented by the page. */
export function parseMemoryCapacity(value: unknown, expectedPolicy: string): MemoryCapacity | null {
  if (!record(value) || value.schema_version !== 1 || !integer(value.sampled_at_ts)
    || !settingsValid(value.settings) || capacityPolicyKey(value.settings) !== expectedPolicy
    || !oneOf(value.capacity_decision, capacityDecisions)
    || !record(value.quota) || value.quota.scope !== "source_transcript_content_rows_and_bytes"
    || !oneOf(value.quota.decision, capacityDecisions) || !record(value.pressure) || !record(value.measurements)) return null;
  const counters = value.quota.counters;
  if (counters !== null && (!record(counters) || !["used_rows", "max_rows", "used_bytes", "max_bytes",
    "spent_and_pending_logical_microunits", "max_logical_microunits"].every(key => decimal(counters[key])))) return null;
  if (record(counters)) {
    const quotaDecision = BigInt(counters.used_rows as string) >= BigInt(counters.max_rows as string) ? "principal_rows_exhausted"
      : BigInt(counters.used_bytes as string) >= BigInt(counters.max_bytes as string) ? "principal_bytes_exhausted"
        : BigInt(counters.spent_and_pending_logical_microunits as string) >= BigInt(counters.max_logical_microunits as string)
          ? "principal_background_cost_exhausted" : "available";
    if (value.quota.decision !== quotaDecision) return null;
  }
  const pressure = value.pressure;
  const observed = value.measurements;
  if (!oneOf(pressure.state, ["normal", "derived_cleanup", "backfill_paused", "automatic_generation_paused", "explicit_write_blocked", "missing", "unknown"])
    || pressure.measurement_scope !== "main_sqlite_allocated_pages_not_memory_domain_or_host_free_space"
    || !(pressure.observed_at_ts === null || integer(pressure.observed_at_ts))
    || !(pressure.revision === null || integer(pressure.revision))
    || !measurement(pressure.observed_bytes) || !measurement(pressure.configured_soft_limit_bytes)
    || !["own_source_transcript_payload_bytes", "main_sqlite_allocated_page_bytes", "memory_domain_assigned_btree_bytes",
      "sqlite_freelist_page_bytes", "database_path_filesystem_available_bytes"].every(key => measurement(observed[key]))) return null;
  const decision = pressure.state === "missing" ? "storage_state_missing" : pressure.state === "unknown" ? "storage_state_unknown"
    : ["automatic_generation_paused", "explicit_write_blocked"].includes(pressure.state as string) ? "storage_paused" : value.quota.decision;
  if (value.capacity_decision !== decision) return null;
  if ((["principal_quota_missing", "principal_quota_invalid"].includes(value.quota.decision as string)) !== (counters === null)) return null;
  return value as unknown as MemoryCapacity;
}
