import type { TaskQueryResponse } from "../types/api";

export interface TaskCostGovernanceView {
  costStatus: string;
  budgetStatus?: string;
  enforcement?: string;
  estimatedCostUsdNanos: number;
  taskKnownCostUsdNanos?: number;
  softTaskLimitUsdNanos?: number;
  hardTaskLimitUsdNanos?: number;
  unknownRecordCount: number;
  hardExceeded: boolean;
  signals: string[];
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

function machineString(record: Record<string, unknown> | null, key: string): string | undefined {
  const value = record?.[key];
  return typeof value === "string" && value.trim() ? value.trim() : undefined;
}

function numberValue(record: Record<string, unknown> | null, key: string): number | undefined {
  const value = record?.[key];
  return typeof value === "number" && Number.isFinite(value) && value >= 0
    ? value
    : undefined;
}

export function buildTaskCostGovernance(
  result: TaskQueryResponse,
): TaskCostGovernanceView | null {
  const resultJson = asRecord(result.result_json);
  const journal = asRecord(resultJson?.task_journal);
  const summary = asRecord(journal?.summary);
  const metrics = asRecord(summary?.task_metrics);
  const cost = asRecord(metrics?.llm_cost);
  const budget = asRecord(metrics?.llm_cost_budget);
  if (!cost && !budget) return null;

  const signals = Array.isArray(budget?.signals)
    ? budget.signals.filter(
        (value): value is string => typeof value === "string" && value.trim().length > 0,
      )
    : [];
  return {
    costStatus: machineString(cost, "status") ?? "unknown",
    budgetStatus: machineString(budget, "status"),
    enforcement: machineString(budget, "enforcement"),
    estimatedCostUsdNanos: numberValue(cost, "estimated_cost_usd_nanos") ?? 0,
    taskKnownCostUsdNanos: numberValue(budget, "task_known_cost_usd_nanos"),
    softTaskLimitUsdNanos: numberValue(budget, "soft_task_limit_usd_nanos"),
    hardTaskLimitUsdNanos: numberValue(budget, "hard_task_limit_usd_nanos"),
    unknownRecordCount:
      numberValue(cost, "unknown_record_count")
      ?? numberValue(budget, "task_unknown_record_count")
      ?? 0,
    hardExceeded: budget?.hard_exceeded === true,
    signals,
  };
}

export function formatUsdNanos(value: number | undefined): string | null {
  if (value == null || !Number.isFinite(value) || value < 0) return null;
  const usd = value / 1_000_000_000;
  const digits = usd >= 1 ? 4 : 6;
  return `$${usd.toFixed(digits)}`;
}
