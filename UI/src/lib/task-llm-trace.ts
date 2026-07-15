import type {
  TaskLlmDebugFlowStageSummary,
  TaskLlmDebugResponse,
  TaskLlmDebugFlowSummary,
} from "../types/api";

export interface AgentFlowTimelineRow {
  stageOrder: number;
  phaseToken: string;
  flowStage: string;
  callIndexes: number[];
  callCount: number;
  promptLabels: string[];
  flowNodes: string[];
  codeModules: string[];
  codeEntrypoints: string[];
  triggerCounts: Record<string, number>;
  statusCounts: Record<string, number>;
  providerErrorCount: number;
}

function compactValue(value: string | number | boolean | null | undefined): string | null {
  if (value == null) return null;
  const text = String(value).trim();
  return text ? text : null;
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

function traceToken(record: Record<string, unknown>, key: string): string | null {
  const value = compactValue(record[key] as string | number | boolean | null | undefined);
  return value ? `${key}=${value}` : null;
}

function nestedTraceToken(record: Record<string, unknown>, parentKey: string, childKey: string): string | null {
  const parent = asRecord(record[parentKey]);
  if (!parent) return null;
  const value = compactValue(parent[childKey] as string | number | boolean | null | undefined);
  return value ? `${parentKey}.${childKey}=${value}` : null;
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

export function modelCatalogTraceMachineTokens(trace: unknown): string[] {
  const record = asRecord(trace);
  if (!record) return [];
  return [
    traceToken(record, "trace_kind"),
    traceToken(record, "status"),
    traceToken(record, "selected_provider"),
    traceToken(record, "selected_model"),
    traceToken(record, "observed_provider_count"),
    traceToken(record, "entry_count"),
    nestedTraceToken(record, "catalog_guard_status", "status"),
    nestedTraceToken(record, "catalog_guard_status", "finding_count"),
  ].filter((item): item is string => Boolean(item));
}

export function resumeTraceMachineTokens(trace: unknown): string[] {
  const record = asRecord(trace);
  if (!record) return [];
  return [
    traceToken(record, "trace_kind"),
    traceToken(record, "state"),
    traceToken(record, "execution_state"),
    traceToken(record, "reason_code"),
    traceToken(record, "checkpoint_id"),
    traceToken(record, "resume_entrypoint"),
    traceToken(record, "resume_due"),
    traceToken(record, "resume_wait_seconds"),
    traceToken(record, "recommended_user_action_kind"),
    traceToken(record, "completed_side_effect_count"),
    traceToken(record, "requires_idempotency_guard"),
    traceToken(record, "provider_blocker_status_code"),
    traceToken(record, "provider_blocker_retry_after_seconds"),
    traceToken(record, "open_issue_count"),
  ].filter((item): item is string => Boolean(item));
}

export function agentFlowPhaseToken(flowStage: string | null | undefined): string {
  const stage = compactValue(flowStage) ?? "";
  if (stage.startsWith("boundary.")) return "boundary";
  if (stage.startsWith("memory.")) return "memory";
  if (stage === "agent_loop.planner") return "planner";
  if (stage === "agent_loop.repair") return "repair";
  if (stage.startsWith("tool.")) return "tool";
  if (stage === "agent_loop.observed_synthesis") return "observed_synthesis";
  if (stage === "agent_loop.answer_verifier") return "answer_verifier";
  if (stage.startsWith("finalizer.")) return "finalizer";
  if (stage.startsWith("scheduler.")) return "scheduler";
  if (stage.startsWith("policy.")) return "policy";
  return "provider";
}

function agentFlowStageOrder(flowStage: string): number {
  const phase = agentFlowPhaseToken(flowStage);
  const baseOrder: Record<string, number> = {
    boundary: 10,
    memory: 20,
    planner: 30,
    repair: 35,
    tool: 40,
    observed_synthesis: 50,
    answer_verifier: 60,
    finalizer: 70,
    scheduler: 80,
    policy: 90,
    provider: 100,
  };
  return baseOrder[phase] ?? 100;
}

function stageCallIndexes(debug: TaskLlmDebugResponse | null | undefined): Map<string, number[]> {
  const calls = debug?.calls && debug.calls.length > 0 ? debug.calls : debug?.entries ?? [];
  const indexes = new Map<string, number[]>();
  calls.forEach((call, index) => {
    const flowStage = compactValue(call.flow?.flow_stage);
    if (!flowStage) return;
    const callIndex = call.call_index ?? index + 1;
    if (!Number.isFinite(callIndex)) return;
    const existing = indexes.get(flowStage) ?? [];
    existing.push(callIndex);
    indexes.set(flowStage, existing);
  });
  return indexes;
}

export function agentFlowTimelineRows(
  debug: TaskLlmDebugResponse | null | undefined,
): AgentFlowTimelineRow[] {
  const stages = debug?.flow_summary?.stages ?? [];
  const callIndexes = stageCallIndexes(debug);
  return stages
    .filter((stage) => compactValue(stage.flow_stage))
    .map((stage) => ({
      stageOrder: agentFlowStageOrder(stage.flow_stage),
      phaseToken: agentFlowPhaseToken(stage.flow_stage),
      flowStage: stage.flow_stage,
      callIndexes: callIndexes.get(stage.flow_stage) ?? [],
      callCount: stage.call_count,
      promptLabels: stage.prompt_labels,
      flowNodes: stage.flow_nodes,
      codeModules: stage.code_modules,
      codeEntrypoints: stage.code_entrypoints,
      triggerCounts: stage.trigger_counts,
      statusCounts: stage.status_counts,
      providerErrorCount: stage.provider_error_count,
    }))
    .sort(
      (left, right) =>
        left.stageOrder - right.stageOrder ||
        left.flowStage.localeCompare(right.flowStage),
    );
}
