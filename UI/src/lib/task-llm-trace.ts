import type { TaskLlmDebugFlowStageSummary, TaskLlmDebugFlowSummary } from "../types/api";

function compactValue(value: string | number | boolean | null | undefined): string | null {
  if (value == null) return null;
  const text = String(value).trim();
  return text ? text : null;
}

function sortedCounterEntries(counts: Record<string, number> | null | undefined, limit: number): string[] {
  if (!counts) return [];
  return Object.entries(counts)
    .filter(([key, value]) => key.trim() && Number.isFinite(value))
    .sort(([leftKey, leftValue], [rightKey, rightValue]) => rightValue - leftValue || leftKey.localeCompare(rightKey))
    .slice(0, limit)
    .map(([key, value]) => `${key}:${value}`);
}

function limitedArrayTokens(prefix: string, values: string[] | null | undefined, limit: number): string[] {
  return (values ?? [])
    .map((value) => compactValue(value))
    .filter((value): value is string => Boolean(value))
    .slice(0, limit)
    .map((value) => `${prefix}=${value}`);
}

export function flowSummaryMachineTokens(summary: TaskLlmDebugFlowSummary | null | undefined): string[] {
  if (!summary) return [];
  return [
    `call_count=${summary.call_count}`,
    `stage_count=${summary.stage_count}`,
    `retry_count=${summary.retry_count}`,
    `verifier_call_count=${summary.verifier_call_count}`,
    `finalizer_call_count=${summary.finalizer_call_count}`,
    `provider_error_count=${summary.provider_error_count}`,
    `module_count=${summary.modules?.length ?? 0}`,
    ...sortedCounterEntries(summary.status_counts, 4).map((item) => `status=${item}`),
    ...sortedCounterEntries(summary.trigger_counts, 4).map((item) => `trigger=${item}`),
  ];
}

export function flowStageMachineTokens(stage: TaskLlmDebugFlowStageSummary): string[] {
  return [
    `flow_stage=${stage.flow_stage}`,
    `call_count=${stage.call_count}`,
    `provider_error_count=${stage.provider_error_count}`,
    ...limitedArrayTokens("prompt_label", stage.prompt_labels, 3),
    ...limitedArrayTokens("flow_node", stage.flow_nodes, 3),
    ...sortedCounterEntries(stage.status_counts, 3).map((item) => `status=${item}`),
    ...sortedCounterEntries(stage.trigger_counts, 3).map((item) => `trigger=${item}`),
  ];
}
