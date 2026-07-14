import { Database, Loader2, RefreshCw, Workflow } from "lucide-react";

import {
  taskLlmDebugCallEntry,
  taskLlmDebugCallMetaTokens,
  taskLlmDebugRawFields,
  taskLlmDebugRequestData,
  taskLlmDebugResponseData,
} from "../lib/task-llm-debug-display";
import {
  agentFlowTimelineRows,
  flowStageMachineTokens,
  flowSummaryMachineTokens,
  type AgentFlowTimelineRow,
} from "../lib/task-llm-trace";
import type { TaskLlmDebugCall, TaskLlmDebugResponse, TaskQueryResponse } from "../types/api";

type Translate = (zh: string, en: string) => string;
type TranslateSlash = (text: string) => string;

export interface TaskLlmTracePanelProps {
  t: Translate;
  tSlash: TranslateSlash;
  taskResult: TaskQueryResponse;
  taskLlmDebug: TaskLlmDebugResponse | null;
  taskLlmDebugLoading: boolean;
  taskLlmDebugError: string | null;
  onQueryTaskLlmDebug: (taskId?: string) => unknown | Promise<unknown>;
}

function compactMetaValue(value: string | number | null | undefined): string | null {
  if (value == null) return null;
  const text = String(value).trim();
  return text ? text : null;
}

function formatJsonish(value: unknown): string {
  if (value == null) return "";
  if (typeof value === "string") {
    const trimmed = value.trim();
    if (!trimmed) return "";
    try {
      return JSON.stringify(JSON.parse(trimmed), null, 2);
    } catch {
      return value;
    }
  }
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}

function flowMeta(call: TaskLlmDebugCall): string[] {
  const flow = call.flow;
  if (!flow) return [];
  return [
    compactMetaValue(flow.prompt_label) ? `prompt_label=${flow.prompt_label}` : null,
    compactMetaValue(flow.flow_stage) ? `flow_stage=${flow.flow_stage}` : null,
    compactMetaValue(flow.flow_node) ? `flow_node=${flow.flow_node}` : null,
    compactMetaValue(flow.trigger_kind) ? `trigger=${flow.trigger_kind}` : null,
  ].filter((item): item is string => Boolean(item));
}

function phaseLabel(phaseToken: string, t: Translate): string {
  const labels: Record<string, string> = {
    boundary: t("边界解析", "Boundary"),
    memory: t("记忆上下文", "Memory"),
    planner: t("规划决策", "Planner"),
    repair: t("循环修复", "Recovery"),
    tool: t("工具意图", "Tool intent"),
    observed_synthesis: t("观测合成", "Observed synthesis"),
    answer_verifier: t("答案验证", "Answer verifier"),
    finalizer: t("最终表达", "Finalizer"),
    scheduler: t("定时任务", "Scheduler"),
    policy: t("策略判断", "Policy"),
    provider: t("模型调用", "Provider"),
  };
  return labels[phaseToken] ?? labels.provider;
}

function timelineMachineTokens(row: AgentFlowTimelineRow): string[] {
  const counterTokens = (prefix: string, counters: Record<string, number>) =>
    Object.entries(counters)
      .sort(([leftKey], [rightKey]) => leftKey.localeCompare(rightKey))
      .slice(0, 2)
      .map(([key, value]) => `${prefix}=${key}:${value}`);
  return [
    `stage_order=${row.stageOrder}`,
    `flow_stage=${row.flowStage}`,
    `call_count=${row.callCount}`,
    row.callIndexes.length > 0 ? `llm_calls=${row.callIndexes.join(",")}` : null,
    row.providerErrorCount > 0 ? `provider_error_count=${row.providerErrorCount}` : null,
    ...counterTokens("status", row.statusCounts),
    ...counterTokens("trigger", row.triggerCounts),
  ].filter((item): item is string => Boolean(item));
}

function memoryTraceMeta(memoryTrace: unknown): string[] {
  if (!memoryTrace || typeof memoryTrace !== "object") return [];
  const record = memoryTrace as Record<string, unknown>;
  return [
    compactMetaValue(record.trace_kind as string | number | null | undefined)
      ? `trace_kind=${record.trace_kind}`
      : null,
    compactMetaValue(record.stage as string | number | null | undefined)
      ? `stage=${record.stage}`
      : null,
    compactMetaValue(record.use_policy as string | number | null | undefined)
      ? `use_policy=${record.use_policy}`
      : null,
    compactMetaValue(record.stage_count as string | number | null | undefined)
      ? `stage_count=${record.stage_count}`
      : null,
  ].filter((item): item is string => Boolean(item));
}

function FlowField({
  label,
  value,
  wide = false,
}: {
  label: string;
  value?: string | null;
  wide?: boolean;
}) {
  return (
    <div className={wide ? "min-w-0 md:col-span-2" : "min-w-0"}>
      <div className="mb-1 text-white/45">{label}</div>
      <div className="truncate rounded-md border border-white/10 bg-black/25 px-2 py-1 font-mono text-white/75" title={value ?? "--"}>
        {value ?? "--"}
      </div>
    </div>
  );
}

export function TaskLlmTracePanel({
  t,
  tSlash,
  taskResult,
  taskLlmDebug,
  taskLlmDebugLoading,
  taskLlmDebugError,
  onQueryTaskLlmDebug,
}: TaskLlmTracePanelProps) {
  const calls =
    taskLlmDebug?.calls && taskLlmDebug.calls.length > 0
      ? taskLlmDebug.calls
      : taskLlmDebug?.entries ?? [];
  const callCount = taskLlmDebug?.call_count ?? calls.length;
  const timelineRows = agentFlowTimelineRows(taskLlmDebug);

  return (
    <div className="mt-4 rounded-xl border border-white/10 bg-[#12151f] p-3">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <p className="text-sm font-semibold">{t("模型调用明细", "LLM call trace")}</p>
          <p className="mt-1 text-xs text-white/50">
            {t("按当前 task_id 查询发送给模型的数据和模型返回的数据。", "Query model request and response data for this task_id.")}
          </p>
        </div>
        <button
          type="button"
          onClick={() => void onQueryTaskLlmDebug(taskResult.task_id)}
          disabled={taskLlmDebugLoading}
          className="inline-flex items-center justify-center gap-2 rounded-xl bg-white/10 px-3 py-2 text-xs font-medium transition hover:bg-white/20 disabled:cursor-not-allowed disabled:opacity-50"
        >
          {taskLlmDebugLoading ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <RefreshCw className="h-3.5 w-3.5" />}
          {t("查询调用明细", "Load trace")}
        </button>
      </div>

      {taskLlmDebugError ? (
        <p className="mt-3 rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-xs text-red-200">
          {tSlash("模型调用明细查询失败 / LLM trace query failed")}: {taskLlmDebugError}
        </p>
      ) : null}

      {taskLlmDebug ? (
        <div className="mt-3">
          <div className="mb-3 flex flex-wrap gap-2 text-xs">
            <span className="rounded-md border border-white/10 bg-black/20 px-2 py-1 font-mono text-white/70">
              task_id={taskLlmDebug.task_id}
            </span>
            <span className="rounded-md border border-white/10 bg-black/20 px-2 py-1 font-mono text-white/70">
              llm_calls={callCount}
            </span>
          </div>

          {taskLlmDebug.flow_summary ? (
            <details className="mb-3 rounded-lg border border-sky-300/15 bg-sky-400/5 p-3" open>
              <summary className="cursor-pointer text-xs font-medium text-sky-100">
                {t("Agent 流程摘要", "Agent flow summary")}
              </summary>
              <div className="mt-2 flex flex-wrap gap-2">
                {flowSummaryMachineTokens(taskLlmDebug.flow_summary).map((item) => (
                  <span key={item} className="rounded-md border border-white/10 bg-black/20 px-2 py-1 font-mono text-[11px] text-sky-50/75">
                    {item}
                  </span>
                ))}
              </div>
              {timelineRows.length > 0 ? (
                <div className="mt-3 border-t border-white/10 pt-3">
                  <div className="mb-2 flex items-center gap-2 text-xs font-medium text-sky-100">
                    <Workflow className="h-3.5 w-3.5" />
                    {t("Agent 过程时间线", "Agent process timeline")}
                  </div>
                  <div className="overflow-hidden rounded-md border border-white/10">
                    {timelineRows.map((row, index) => (
                      <div
                        key={row.flowStage}
                        className={
                          index === 0
                            ? "grid gap-2 bg-black/15 px-3 py-2 text-[11px] md:grid-cols-[7rem_minmax(0,1fr)_minmax(0,1.2fr)]"
                            : "grid gap-2 border-t border-white/10 bg-black/15 px-3 py-2 text-[11px] md:grid-cols-[7rem_minmax(0,1fr)_minmax(0,1.2fr)]"
                        }
                      >
                        <div className="font-medium text-sky-100">
                          {phaseLabel(row.phaseToken, t)}
                        </div>
                        <div className="min-w-0">
                          <div className="flex flex-wrap gap-1.5">
                            {timelineMachineTokens(row).map((item) => (
                              <span key={item} className="rounded border border-white/10 bg-white/5 px-1.5 py-0.5 font-mono text-white/60">
                                {item}
                              </span>
                            ))}
                          </div>
                        </div>
                        <div className="min-w-0 font-mono text-white/55">
                          <div className="truncate" title={row.codeModules[0] ?? "--"}>
                            module={row.codeModules[0] ?? "--"}
                          </div>
                          <div className="truncate" title={row.codeEntrypoints[0] ?? "--"}>
                            entrypoint={row.codeEntrypoints[0] ?? "--"}
                          </div>
                        </div>
                      </div>
                    ))}
                  </div>
                </div>
              ) : null}
              {taskLlmDebug.flow_summary.stages.length > 0 ? (
                <div className="mt-3 grid gap-2 lg:grid-cols-2">
                  {taskLlmDebug.flow_summary.stages.map((stage) => (
                    <details key={stage.flow_stage} className="rounded-md border border-white/10 bg-black/20 p-2">
                      <summary className="cursor-pointer font-mono text-[11px] text-white/70">
                        {stage.flow_stage} · calls={stage.call_count}
                      </summary>
                      <div className="mt-2 flex flex-wrap gap-2">
                        {flowStageMachineTokens(stage).map((item) => (
                          <span key={item} className="rounded-md border border-white/10 bg-white/5 px-2 py-1 font-mono text-[11px] text-white/60">
                            {item}
                          </span>
                        ))}
                      </div>
                      <div className="mt-2 grid gap-2 text-[11px] md:grid-cols-2">
                        <FlowField label={t("代码模块", "Code module")} value={stage.code_modules[0]} />
                        <FlowField label={t("入口函数", "Entrypoint")} value={stage.code_entrypoints[0]} />
                      </div>
                    </details>
                  ))}
                </div>
              ) : null}
            </details>
          ) : null}

          {taskLlmDebug.memory_trace ? (
            <details className="mb-3 rounded-lg border border-emerald-300/15 bg-emerald-400/5 p-3">
              <summary className="cursor-pointer text-xs font-medium text-emerald-100">
                {t("记忆/知识库上下文策略", "Memory and KB context policy")}
              </summary>
              <div className="mt-2 flex flex-wrap gap-2">
                {memoryTraceMeta(taskLlmDebug.memory_trace).map((item) => (
                  <span key={item} className="rounded-md border border-white/10 bg-black/20 px-2 py-1 font-mono text-[11px] text-emerald-50/75">
                    {item}
                  </span>
                ))}
              </div>
              <pre className="mt-2 max-h-72 overflow-auto rounded-md bg-black/40 p-3 text-[11px] leading-relaxed text-white/70">
                {formatJsonish(taskLlmDebug.memory_trace)}
              </pre>
            </details>
          ) : null}

          {calls.length > 0 ? (
            <div className="space-y-3">
              {calls.map((call, index) => {
                const entry = taskLlmDebugCallEntry(call);
                const callIndex = call.call_index ?? index + 1;
                return (
                  <details
                    key={`${entry.call_id ?? entry.ts ?? "llm"}-${callIndex}`}
                    className="rounded-lg border border-white/10 bg-black/20 p-3"
                    open={index === calls.length - 1}
                  >
                    <summary className="cursor-pointer list-none">
                      <div className="flex flex-wrap items-center justify-between gap-2">
                        <div className="inline-flex items-center gap-2">
                          <Database className="h-4 w-4 text-sky-200" />
                          <span className="font-mono text-sm font-semibold text-white">
                            LLM #{callIndex}
                          </span>
                        </div>
                        <span className="rounded-md border border-white/10 bg-white/5 px-2 py-1 text-[11px] text-white/60">
                          {entry.status ?? "--"}
                        </span>
                      </div>
                      <div className="mt-2 flex flex-wrap gap-2">
                        {flowMeta(call).slice(0, 4).map((item) => (
                          <span key={item} className="rounded-md border border-sky-300/20 bg-sky-400/10 px-2 py-1 font-mono text-[11px] text-sky-100">
                            {item}
                          </span>
                        ))}
                        {taskLlmDebugCallMetaTokens(call).slice(0, 8).map((item) => (
                          <span key={item} className="rounded-md border border-white/10 bg-white/5 px-2 py-1 font-mono text-[11px] text-white/60">
                            {item}
                          </span>
                        ))}
                      </div>
                    </summary>

                    {call.flow ? (
                      <div className="mt-3 rounded-lg border border-sky-300/15 bg-sky-400/5 p-3">
                        <p className="mb-2 text-xs font-medium text-sky-100">
                          {t("RustClaw 流程", "RustClaw flow")}
                        </p>
                        <div className="grid gap-2 text-[11px] md:grid-cols-2">
                          <FlowField label={t("流程阶段", "Flow stage")} value={call.flow.flow_stage} />
                          <FlowField label={t("流程节点", "Flow node")} value={call.flow.flow_node} />
                          <FlowField label={t("触发类型", "Trigger")} value={call.flow.trigger_kind} />
                          <FlowField label={t("Prompt 标签", "Prompt label")} value={call.flow.prompt_label} />
                          <FlowField label={t("代码模块", "Code module")} value={call.flow.code_module} wide />
                          <FlowField label={t("入口函数", "Entrypoint")} value={call.flow.code_entrypoint} wide />
                        </div>
                      </div>
                    ) : null}

                    <div className="mt-3 grid gap-3 xl:grid-cols-2">
                      <div>
                        <p className="mb-2 text-xs font-medium text-white/60">
                          {t("发送给大模型的数据", "Request sent to model")}
                        </p>
                        <pre className="max-h-80 overflow-auto rounded-md bg-black/40 p-3 text-[11px] leading-relaxed text-white/75">
                          {formatJsonish(taskLlmDebugRequestData(call))}
                        </pre>
                      </div>
                      <div>
                        <p className="mb-2 text-xs font-medium text-white/60">
                          {t("大模型返回的数据", "Model response data")}
                        </p>
                        <pre className="max-h-80 overflow-auto rounded-md bg-black/40 p-3 text-[11px] leading-relaxed text-white/75">
                          {formatJsonish(taskLlmDebugResponseData(call))}
                        </pre>
                      </div>
                    </div>

                    <details className="mt-3">
                      <summary className="cursor-pointer text-[11px] text-white/45">
                        {t("原始字段", "Raw fields")} · {taskLlmDebugRawFields(call)}
                      </summary>
                      <pre className="mt-2 max-h-72 overflow-auto rounded-md bg-black/40 p-3 text-[11px] leading-relaxed text-white/70">
                        {formatJsonish(call)}
                      </pre>
                    </details>
                  </details>
                );
              })}
            </div>
          ) : (
            <p className="rounded-lg border border-white/10 bg-black/20 px-3 py-2 text-xs text-white/55">
              {t("这个任务还没有可显示的模型调用记录。", "No model call records are available for this task yet.")}
            </p>
          )}
        </div>
      ) : null}
    </div>
  );
}
