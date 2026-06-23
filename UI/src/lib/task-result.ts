import type { TaskQueryResponse } from "../types/api";
import type { TaskLifecycleLang } from "./task-lifecycle";

export interface TaskOutcomeView {
  title: string;
  tone: "ok" | "running" | "attention" | "failed";
  nextStep: string;
  finalShape?: string;
  missingEvidence: string[];
  failureLabel?: string;
}

export interface TaskPermissionView {
  tone: "ok" | "attention" | "failed";
  title: string;
  meta: string[];
}

export function extractTaskText(result: TaskQueryResponse): string {
  if (result.result_json && typeof result.result_json === "object") {
    const maybeText = (result.result_json as { text?: unknown }).text;
    if (typeof maybeText === "string" && maybeText.trim()) {
      return maybeText;
    }
  }
  if (result.error_text) {
    return result.error_text;
  }
  return JSON.stringify(result.result_json ?? null, null, 2);
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

function getPathValue(root: unknown, path: string[]): unknown {
  let current: unknown = root;
  for (const key of path) {
    const record = asRecord(current);
    if (!record) return undefined;
    current = record[key];
  }
  return current;
}

function stringAt(root: unknown, path: string[]): string | undefined {
  const value = getPathValue(root, path);
  return typeof value === "string" && value.trim() ? value.trim() : undefined;
}

function stringArrayAt(root: unknown, path: string[]): string[] {
  const value = getPathValue(root, path);
  if (!Array.isArray(value)) return [];
  return value.filter((item): item is string => typeof item === "string" && item.trim().length > 0);
}

function boolAt(root: unknown, path: string[]): boolean | undefined {
  const value = getPathValue(root, path);
  return typeof value === "boolean" ? value : undefined;
}

function taskTraceRoot(result: TaskQueryResponse): unknown {
  return getPathValue(result.result_json, ["task_journal", "trace"]);
}

function taskSummaryRoot(result: TaskQueryResponse): unknown {
  return getPathValue(result.result_json, ["task_journal", "summary"]);
}

export function taskTraceEvents(result: TaskQueryResponse): Record<string, unknown>[] {
  const value = getPathValue(taskTraceRoot(result), ["event_stream"]);
  if (!Array.isArray(value)) return [];
  return value.filter(
    (item): item is Record<string, unknown> => Boolean(item) && typeof item === "object" && !Array.isArray(item),
  );
}

function traceEventPayload(event: Record<string, unknown>): Record<string, unknown> | null {
  const payload = event.payload;
  return payload && typeof payload === "object" && !Array.isArray(payload)
    ? (payload as Record<string, unknown>)
    : null;
}

export function traceEventMeta(event: Record<string, unknown>): string[] {
  const payload = traceEventPayload(event);
  const meta: string[] = [];
  const seq = typeof event.seq === "number" || typeof event.seq === "string" ? String(event.seq) : "";
  if (seq) meta.push(`seq=${seq}`);
  const eventType = typeof event.event_type === "string" ? event.event_type.trim() : "";
  if (eventType) meta.push(`type=${eventType}`);
  if (!payload) return meta;
  for (const key of [
    "status",
    "state",
    "error_kind",
    "failure_attribution",
    "owner_layer",
    "stage",
    "decision",
    "reason_code",
    "role",
    "execution_mode",
    "write_enabled",
    "external_publish_enabled",
    "failure_isolated",
    "child_run_id",
    "objective_present",
    "objective_char_count",
    "context_ref_count",
    "allowed_capability_count",
    "skill",
    "tool_or_skill",
    "step_id",
    "action_kind",
    "action_ref",
    "requested_capability",
    "requested_action_ref",
    "resolved_tool_or_skill",
    "resolved_capability",
    "resolution_source",
    "output_evidence_count",
    "artifact_ref_count",
    "prompt_label",
    "llm_call_count",
    "elapsed_ms",
    "provider_attempt_count",
    "provider_retry_count",
    "provider_retryable_error_count",
    "provider_final_error_count",
    "prompt_truncation_count",
    "prompt_bytes_before_max",
    "prompt_bytes_budget_min",
    "prompt_bytes_after_max",
    "prompt_truncated_bytes_total",
    "checkpoint_id",
    "poll_ref",
    "final_status",
    "final_stop_signal",
  ]) {
    const value = payload[key];
    if ((typeof value === "string" && value.trim()) || typeof value === "number" || typeof value === "boolean") {
      meta.push(`${key}=${String(value)}`);
    }
  }
  const childTraceMergeStatus = stringAt(payload, ["child_run_summary", "trace_merge_status"]);
  if (childTraceMergeStatus) meta.push(`child_trace_merge_status=${childTraceMergeStatus}`);
  const childResultStatus = stringAt(payload, ["child_run_summary", "result_status"]);
  if (childResultStatus) meta.push(`child_result_status=${childResultStatus}`);
  const childRequestState = stringAt(payload, ["child_request", "state"]);
  if (childRequestState) meta.push(`child_request_state=${childRequestState}`);
  const schedulerStatus = stringAt(payload, ["scheduler", "status"]);
  if (schedulerStatus) meta.push(`scheduler_status=${schedulerStatus}`);
  const schedulerReasonCode = stringAt(payload, ["scheduler", "reason_code"]);
  if (schedulerReasonCode) meta.push(`scheduler_reason_code=${schedulerReasonCode}`);
  const mergeStrategy = stringAt(payload, ["merge_contract", "strategy"]);
  if (mergeStrategy) meta.push(`merge_strategy=${mergeStrategy}`);
  const mergeStatus = stringAt(payload, ["merge_contract", "child_trace_merge_status"]);
  if (mergeStatus) meta.push(`merge_status=${mergeStatus}`);
  return meta;
}

function taskPermissionRoot(result: TaskQueryResponse): unknown {
  return (
    getPathValue(taskSummaryRoot(result), ["verify_result", "permission_decision"]) ??
    getPathValue(taskTraceRoot(result), ["verify_result", "permission_decision"]) ??
    getPathValue(result.result_json, ["permission_decision"])
  );
}

function humanFailureLabel(kind: string | undefined, lang: TaskLifecycleLang): string | undefined {
  if (!kind) return undefined;
  const zh: Record<string, string> = {
    budget_exhausted: "任务用完了本轮尝试次数",
    contract_gap: "任务规则拦截了不合适的动作",
    delivery_error: "结果发送或文件交付没有完成",
    permission_denied: "需要权限或确认后才能继续",
    provider_error: "模型服务暂时不可用",
    schema_error: "模型返回格式不符合要求",
    tool_gap: "当前没有合适的工具完成这一步",
  };
  const en: Record<string, string> = {
    budget_exhausted: "The task used its retry budget",
    contract_gap: "The task rules blocked an unsuitable action",
    delivery_error: "Result delivery did not finish",
    permission_denied: "Permission or confirmation is required",
    provider_error: "The model provider is unavailable",
    schema_error: "The model response did not match the expected format",
    tool_gap: "No suitable tool is available for this step",
  };
  return (lang === "zh" ? zh : en)[kind] ?? kind;
}

export function buildTaskOutcome(result: TaskQueryResponse, lang: TaskLifecycleLang): TaskOutcomeView {
  const summary = taskSummaryRoot(result);
  const trace = taskTraceRoot(result);
  const outcome = getPathValue(summary, ["task_outcome"]);
  const outcomeState = stringAt(outcome, ["state"]);
  const outcomeMessage =
    stringAt(outcome, [lang === "zh" ? "message_zh" : "message_en"]) ??
    stringAt(outcome, ["message_zh"]) ??
    stringAt(outcome, ["message_en"]);
  const outcomeNextStep =
    stringAt(outcome, [lang === "zh" ? "next_step_zh" : "next_step_en"]) ??
    stringAt(outcome, ["next_step_zh"]) ??
    stringAt(outcome, ["next_step_en"]);
  const finalStatus = stringAt(summary, ["final_status"]) ?? result.status;
  const failureKind =
    stringAt(summary, ["final_failure_attribution"]) ?? stringAt(trace, ["final_failure_attribution"]);
  const missingEvidence = stringArrayAt(trace, ["evidence_coverage", "missing_evidence"]);
  const finalShape =
    stringAt(trace, ["contract_matrix", "final_answer_shape"]) ??
    stringAt(summary, ["finalizer_summary", "final_answer_shape"]);
  const tLocal = (zh: string, en: string) => (lang === "zh" ? zh : en);

  if (result.status === "queued" || result.status === "running") {
    return {
      title: tLocal("正在处理", "In progress"),
      tone: "running",
      nextStep: tLocal(
        "稍后会自动刷新；如果等待较久，可以重新查询这个任务 ID。",
        "This will refresh automatically; query the task ID again if it takes a while.",
      ),
      finalShape,
      missingEvidence,
    };
  }

  if (result.status === "succeeded" || finalStatus === "success") {
    return {
      title: outcomeMessage ?? tLocal("已完成", "Completed"),
      tone: outcomeState === "needs_attention" || missingEvidence.length > 0 ? "attention" : "ok",
      nextStep:
        outcomeNextStep ??
        (missingEvidence.length > 0
          ? tLocal(
              "任务已返回结果，但还有证据项没有完全匹配；请查看详情确认。",
              "The task returned a result, but some evidence fields did not fully match; check details.",
            )
          : tLocal("任务已经完成，可以直接查看结果。", "The task completed. You can review the result.")),
      finalShape,
      missingEvidence,
    };
  }

  return {
    title: outcomeMessage ?? tLocal("没有完成", "Not completed"),
    tone: "failed",
    nextStep:
      outcomeNextStep ??
      (missingEvidence.length > 0
        ? tLocal(
            `还缺少证据：${missingEvidence.join(", ")}。请补充目标或稍后重试。`,
            `Missing evidence: ${missingEvidence.join(", ")}. Add the target or retry later.`,
          )
        : tLocal(
            "请根据错误提示处理后重试；技术详情已放在下方。",
            "Use the error message to decide the next step, then retry. Technical details are below.",
          )),
    finalShape,
    missingEvidence,
    failureLabel: humanFailureLabel(failureKind, lang),
  };
}

export function buildTaskPermissionView(
  result: TaskQueryResponse,
  lang: TaskLifecycleLang,
): TaskPermissionView | null {
  const permission = taskPermissionRoot(result);
  if (!asRecord(permission)) return null;
  const tLocal = (zh: string, en: string) => (lang === "zh" ? zh : en);
  const allowed = boolAt(permission, ["allowed"]);
  const needsConfirmation = boolAt(permission, ["needs_confirmation"]);
  const deniedByPolicy = boolAt(permission, ["denied_by_policy"]);
  const dryRunRequired = boolAt(permission, ["dry_run_required"]);
  const externalProviderBlocked = boolAt(permission, ["external_provider_blocked"]);
  const riskLevel = stringAt(permission, ["risk_level"]);
  const actionEffect = stringAt(permission, ["action_effect"]);
  const ownerLayer = stringAt(permission, ["owner_layer"]);
  const statusCode = stringAt(permission, ["status_code"]);
  const messageKey = stringAt(permission, ["message_key"]);
  const meta = [
    `allowed=${allowed ?? "--"}`,
    `needs_confirmation=${needsConfirmation ?? "--"}`,
    `denied_by_policy=${deniedByPolicy ?? "--"}`,
  ];
  if (dryRunRequired !== undefined) meta.push(`dry_run_required=${dryRunRequired}`);
  if (externalProviderBlocked !== undefined) meta.push(`external_provider_blocked=${externalProviderBlocked}`);
  if (riskLevel) meta.push(`risk=${riskLevel}`);
  if (actionEffect) meta.push(`effect=${actionEffect}`);
  if (ownerLayer) meta.push(`owner=${ownerLayer}`);
  if (statusCode) meta.push(`status=${statusCode}`);
  if (messageKey) meta.push(`message_key=${messageKey}`);
  const tone = deniedByPolicy || externalProviderBlocked ? "failed" : needsConfirmation || dryRunRequired ? "attention" : "ok";
  return {
    tone,
    title: tLocal("权限决策", "Permission decision"),
    meta,
  };
}
