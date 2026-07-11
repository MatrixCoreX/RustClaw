import { Database, Loader2, RefreshCw } from "lucide-react";

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

function usageMeta(call: TaskLlmDebugCall): string[] {
  const usage = call.usage;
  if (!usage) return [];
  const promptTokens = usage.prompt_tokens ?? usage.input_tokens;
  const completionTokens = usage.completion_tokens ?? usage.output_tokens;
  return [
    promptTokens != null ? `prompt=${promptTokens}` : null,
    completionTokens != null ? `completion=${completionTokens}` : null,
    usage.reasoning_tokens != null ? `reasoning=${usage.reasoning_tokens}` : null,
    usage.cached_tokens != null ? `cached=${usage.cached_tokens}` : null,
    usage.total_tokens != null ? `total=${usage.total_tokens}` : null,
  ].filter((item): item is string => Boolean(item));
}

function callMeta(call: TaskLlmDebugCall): string[] {
  return [
    compactMetaValue(call.status) ? `status=${call.status}` : null,
    compactMetaValue(call.model) ? `model=${call.model}` : null,
    compactMetaValue(call.provider) ? `provider=${call.provider}` : null,
    compactMetaValue(call.vendor) ? `vendor=${call.vendor}` : null,
    compactMetaValue(call.prompt_source ?? call.prompt_file) ? `stage=${call.prompt_source ?? call.prompt_file}` : null,
    compactMetaValue(call.call_id) ? `call_id=${call.call_id}` : null,
    compactMetaValue(call.ts) ? `ts=${call.ts}` : null,
    ...usageMeta(call),
  ].filter((item): item is string => Boolean(item));
}

function rawFields(call: TaskLlmDebugCall): string {
  return Object.keys(call)
    .filter((key) => call[key as keyof TaskLlmDebugCall] != null)
    .sort()
    .join(", ");
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

          {calls.length > 0 ? (
            <div className="space-y-3">
              {calls.map((call, index) => {
                const callIndex = call.call_index ?? index + 1;
                return (
                  <details
                    key={`${call.call_id ?? call.ts ?? "llm"}-${callIndex}`}
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
                          {call.status ?? "--"}
                        </span>
                      </div>
                      <div className="mt-2 flex flex-wrap gap-2">
                        {callMeta(call).slice(0, 8).map((item) => (
                          <span key={item} className="rounded-md border border-white/10 bg-white/5 px-2 py-1 font-mono text-[11px] text-white/60">
                            {item}
                          </span>
                        ))}
                      </div>
                    </summary>

                    <div className="mt-3 grid gap-3 xl:grid-cols-2">
                      <div>
                        <p className="mb-2 text-xs font-medium text-white/60">
                          {t("发送给大模型的数据", "Request sent to model")}
                        </p>
                        <pre className="max-h-80 overflow-auto rounded-md bg-black/40 p-3 text-[11px] leading-relaxed text-white/75">
                          {formatJsonish(call.request_payload ?? call.prompt)}
                        </pre>
                      </div>
                      <div>
                        <p className="mb-2 text-xs font-medium text-white/60">
                          {t("大模型返回的数据", "Model response data")}
                        </p>
                        <pre className="max-h-80 overflow-auto rounded-md bg-black/40 p-3 text-[11px] leading-relaxed text-white/75">
                          {formatJsonish(call.raw_response ?? call.clean_response ?? call.response)}
                        </pre>
                      </div>
                    </div>

                    <details className="mt-3">
                      <summary className="cursor-pointer text-[11px] text-white/45">
                        {t("原始字段", "Raw fields")} · {rawFields(call)}
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
