import { Loader2, MessageCircle, RefreshCw } from "lucide-react";

import { buildTaskLifecycleView, buildTaskPollingView, type TaskLifecycleLang } from "../lib/task-lifecycle";
import {
  buildReplaySummary,
  buildTaskOutcome,
  buildTaskPermissionView,
  buildTaskTraceEventView,
  taskArtifactRefs,
  taskTraceEvents,
  type TaskOutcomeView,
  type TaskPermissionView,
} from "../lib/task-result";
import type { TaskQueryResponse } from "../types/api";

type Translate = (zh: string, en: string) => string;
type TranslateSlash = (text: string) => string;
type Tone = TaskOutcomeView["tone"] | TaskPermissionView["tone"];

export interface TaskResultPanelProps {
  lang: TaskLifecycleLang;
  t: Translate;
  tSlash: TranslateSlash;
  taskId: string;
  taskLoading: boolean;
  taskError: string | null;
  taskResult: TaskQueryResponse | null;
  resumeDrafts: Record<string, string>;
  resumeSubmittingTaskId: string | null;
  onTaskIdChange: (value: string) => void;
  onQueryTask: () => unknown | Promise<unknown>;
  onResumeDraftChange: (taskId: string, value: string) => void;
  onSubmitResume: (taskId: string) => unknown | Promise<unknown>;
}

function toneClassName(tone: Tone): string {
  if (tone === "ok") return "border-emerald-400/25 bg-emerald-500/10 text-emerald-50";
  if (tone === "running") return "border-sky-400/25 bg-sky-500/10 text-sky-50";
  if (tone === "attention") return "border-amber-400/25 bg-amber-500/10 text-amber-50";
  return "border-red-400/25 bg-red-500/10 text-red-50";
}

export function TaskResultPanel({
  lang,
  t,
  tSlash,
  taskId,
  taskLoading,
  taskError,
  taskResult,
  resumeDrafts,
  resumeSubmittingTaskId,
  onTaskIdChange,
  onQueryTask,
  onResumeDraftChange,
  onSubmitResume,
}: TaskResultPanelProps) {
  const taskOutcome = taskResult ? buildTaskOutcome(taskResult, lang) : null;
  const taskLifecycleView = taskResult ? buildTaskLifecycleView(taskResult.lifecycle, taskResult.status, lang) : null;
  const taskPollingView = taskResult ? buildTaskPollingView(taskResult.lifecycle, lang) : null;
  const taskPermissionView = taskResult ? buildTaskPermissionView(taskResult, lang) : null;
  const taskEvents = taskResult ? taskTraceEvents(taskResult) : [];
  const artifactRefs = taskResult ? taskArtifactRefs(taskResult) : [];
  const replaySummary = taskResult ? buildReplaySummary(taskResult) : null;

  return (
    <section className="rounded-2xl border border-white/10 bg-white/5 p-5">
      <h3 className="mb-4 text-lg font-semibold">{t("按 task_id 查询结果", "Query a result by task_id")}</h3>
      <div className="grid gap-4 md:grid-cols-[1fr_auto]">
        <input
          className="theme-input"
          placeholder="输入 task_id（UUID）/ Enter task_id"
          value={taskId}
          onChange={(event) => onTaskIdChange(event.target.value)}
        />
        <button
          type="button"
          onClick={() => void onQueryTask()}
          disabled={taskLoading || !taskId.trim()}
          className="inline-flex items-center justify-center gap-2 rounded-xl bg-white/10 px-4 py-2 text-sm font-medium transition hover:bg-white/20 disabled:cursor-not-allowed disabled:opacity-50"
        >
          {taskLoading ? <Loader2 className="h-4 w-4 animate-spin" /> : <RefreshCw className="h-4 w-4" />}
          {tSlash("查询任务 / Query")}
        </button>
      </div>

      {taskError ? (
        <p className="mt-3 rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">
          {tSlash("查询失败 / Query failed")}: {taskError}
        </p>
      ) : null}

      {taskResult ? (
        <div className="mt-4 rounded-xl border border-white/10 bg-black/30 p-4 text-sm">
          <p className="mb-1 text-white/60">{tSlash("任务 ID / Task ID")}</p>
          <p className="font-mono text-white">{taskResult.task_id}</p>
          <div className="mt-3 grid gap-3 md:grid-cols-2">
            <div>
              <p className="mb-1 text-white/60">{tSlash("状态 / Status")}</p>
              <p className="theme-status-pill inline-block rounded-md px-2 py-1 font-mono">{taskResult.status}</p>
            </div>
            <div>
              <p className="mb-1 text-white/60">{tSlash("错误信息 / Error")}</p>
              <p className="text-red-200">{taskResult.error_text || "--"}</p>
            </div>
          </div>
          {taskLifecycleView ? (
            <div className={`mt-4 rounded-xl border px-3 py-3 ${toneClassName(taskLifecycleView.tone)}`}>
              <div className="flex flex-wrap items-center justify-between gap-2">
                <p className="font-semibold">{t("执行状态", "Runtime lifecycle")}</p>
                <span className="theme-status-pill rounded-md px-2 py-1 text-xs font-medium">{taskLifecycleView.stateLabel}</span>
              </div>
              <p className="mt-1 text-sm opacity-80">{taskLifecycleView.detail}</p>
              <div className="mt-3 flex flex-wrap gap-2 text-xs">
                {taskLifecycleView.meta.map((item) => (
                  <span key={item} className="rounded-md border border-white/10 bg-black/20 px-2 py-1">
                    {item}
                  </span>
                ))}
              </div>
            </div>
          ) : null}
          {taskPollingView ? (
            <div className="mt-4 rounded-xl border border-sky-400/25 bg-sky-500/10 px-3 py-3 text-sky-50">
              <p className="font-semibold">{t("后台轮询", "Background polling")}</p>
              <p className="mt-1 text-sm text-sky-50/75">{taskPollingView.detail}</p>
              <div className="mt-3 flex flex-wrap gap-2 text-xs">
                {taskPollingView.meta.map((item) => (
                  <span key={item} className="rounded-md border border-white/10 bg-black/20 px-2 py-1">
                    {item}
                  </span>
                ))}
              </div>
            </div>
          ) : null}
          {taskResult.lifecycle?.state === "needs_user" ? (
            <div className="mt-4 rounded-xl border border-amber-400/25 bg-amber-500/10 px-3 py-3">
              <label className="block space-y-2">
                <span className="text-xs font-medium text-amber-50">
                  {t("补充确认内容", "Follow-up input")}
                </span>
                <textarea
                  className="theme-input min-h-20"
                  value={resumeDrafts[taskResult.task_id] ?? ""}
                  onChange={(event) => onResumeDraftChange(taskResult.task_id, event.target.value)}
                  placeholder={t("输入确认或补充说明后继续执行", "Enter confirmation or follow-up text to continue")}
                />
              </label>
              <button
                type="button"
                onClick={() => void onSubmitResume(taskResult.task_id)}
                disabled={resumeSubmittingTaskId === taskResult.task_id || !(resumeDrafts[taskResult.task_id] ?? "").trim()}
                className="theme-accent-btn mt-3 text-xs disabled:cursor-not-allowed disabled:opacity-50"
              >
                {resumeSubmittingTaskId === taskResult.task_id ? (
                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                ) : (
                  <MessageCircle className="h-3.5 w-3.5" />
                )}
                {t("继续执行", "Resume")}
              </button>
            </div>
          ) : null}
          {taskPermissionView ? (
            <div className={`mt-4 rounded-xl border px-3 py-3 ${toneClassName(taskPermissionView.tone)}`}>
              <p className="font-semibold">{taskPermissionView.title}</p>
              <div className="mt-3 flex flex-wrap gap-2 text-xs">
                {taskPermissionView.meta.map((item) => (
                  <span key={item} className="rounded-md border border-white/10 bg-black/20 px-2 py-1">
                    {item}
                  </span>
                ))}
              </div>
            </div>
          ) : null}
          {taskOutcome ? (
            <div className={`mt-4 rounded-xl border px-3 py-3 ${toneClassName(taskOutcome.tone)}`}>
              <p className="font-semibold">{taskOutcome.title}</p>
              <p className="mt-1 text-sm opacity-80">{taskOutcome.nextStep}</p>
              <div className="mt-3 flex flex-wrap gap-2 text-xs">
                {taskOutcome.finalShape ? (
                  <span className="rounded-md border border-white/10 bg-black/20 px-2 py-1">
                    {t("输出形状", "Answer shape")}: {taskOutcome.finalShape}
                  </span>
                ) : null}
                {taskOutcome.failureLabel ? (
                  <span className="rounded-md border border-white/10 bg-black/20 px-2 py-1">
                    {taskOutcome.failureLabel}
                  </span>
                ) : null}
                {taskOutcome.missingEvidence.length > 0 ? (
                  <span className="rounded-md border border-white/10 bg-black/20 px-2 py-1">
                    {t("缺少证据", "Missing evidence")}: {taskOutcome.missingEvidence.join(", ")}
                  </span>
                ) : null}
              </div>
            </div>
          ) : null}
          {taskEvents.length > 0 ? (
            <details className="mt-4 rounded-lg border border-white/10 bg-[#12151f] p-3">
              <summary className="cursor-pointer text-xs font-medium text-white/65">
                {t("工具事件", "Tool events")} · {taskEvents.length}
              </summary>
              <div className="mt-3 space-y-2">
                {taskEvents.slice(0, 12).map((event, index) => {
                  const eventView = buildTaskTraceEventView(event, lang);
                  const meta = eventView.meta;
                  const eventType = typeof event.event_type === "string" ? event.event_type : `event_${index + 1}`;
                  return (
                    <div key={`${eventType}-${index}`} className={`rounded-lg border px-3 py-2 ${toneClassName(eventView.tone)}`}>
                      <div className="flex flex-wrap items-start justify-between gap-2">
                        <div>
                          <p className="text-sm font-semibold">{eventView.title}</p>
                          <p className="mt-1 text-xs opacity-80">{eventView.detail}</p>
                        </div>
                        <span className="rounded-md border border-white/10 bg-black/20 px-2 py-1 font-mono text-[11px] opacity-75">
                          {eventView.eventType}
                        </span>
                      </div>
                      <div className="flex flex-wrap items-center gap-2">
                        {meta.length > 0 ? (
                          meta.map((item) => (
                            <span key={item} className="rounded-md border border-white/10 bg-white/5 px-2 py-1 font-mono text-[11px] text-white/70">
                              {item}
                            </span>
                          ))
                        ) : (
                          <span className="rounded-md border border-white/10 bg-white/5 px-2 py-1 font-mono text-[11px] text-white/70">
                            {eventType}
                          </span>
                        )}
                      </div>
                      <details className="mt-2">
                        <summary className="cursor-pointer text-[11px] text-white/45">{t("原始事件", "Raw event")}</summary>
                        <pre className="mt-2 max-h-48 overflow-auto rounded-md bg-black/30 p-2 text-[11px] text-white/70">
                          {JSON.stringify(event, null, 2)}
                        </pre>
                      </details>
                    </div>
                  );
                })}
                {taskEvents.length > 12 ? (
                  <p className="text-[11px] text-white/40">
                    {t(`还有 ${taskEvents.length - 12} 条事件在技术 JSON 中。`, `${taskEvents.length - 12} more event(s) are in Technical JSON.`)}
                  </p>
                ) : null}
              </div>
            </details>
          ) : null}
          {artifactRefs.length > 0 ? (
            <details className="mt-4 rounded-lg border border-white/10 bg-[#12151f] p-3">
              <summary className="cursor-pointer text-xs font-medium text-white/65">
                {t("产物引用", "Artifact refs")} · {artifactRefs.length}
              </summary>
              <div className="mt-3 space-y-2">
                {artifactRefs.slice(0, 12).map((artifact) => (
                  <div key={artifact.key} className="rounded-lg border border-white/10 bg-black/20 px-3 py-2">
                    <p className="break-words font-mono text-[11px] text-white/75">{artifact.summary}</p>
                    <details className="mt-2">
                      <summary className="cursor-pointer text-[11px] text-white/45">{t("原始产物字段", "Raw artifact field")}</summary>
                      <pre className="mt-2 max-h-48 overflow-auto rounded-md bg-black/30 p-2 text-[11px] text-white/70">
                        {JSON.stringify(artifact.raw, null, 2)}
                      </pre>
                    </details>
                  </div>
                ))}
                {artifactRefs.length > 12 ? (
                  <p className="text-[11px] text-white/40">
                    {t(`还有 ${artifactRefs.length - 12} 个产物引用在技术 JSON 中。`, `${artifactRefs.length - 12} more artifact ref(s) are in Technical JSON.`)}
                  </p>
                ) : null}
              </div>
            </details>
          ) : null}
          {replaySummary ? (
            <details className="mt-4 rounded-lg border border-white/10 bg-[#12151f] p-3">
              <summary className="cursor-pointer text-xs font-medium text-white/65">
                {t("回放摘要", "Replay summary")}
              </summary>
              <div className="mt-3 flex flex-wrap gap-2 text-xs">
                {replaySummary.meta.map((item) => (
                  <span key={item} className="rounded-md border border-white/10 bg-black/20 px-2 py-1 font-mono text-white/70">
                    {item}
                  </span>
                ))}
                {replaySummary.coverage.map((item) => (
                  <span key={item} className="rounded-md border border-white/10 bg-black/20 px-2 py-1 font-mono text-white/70">
                    {item}
                  </span>
                ))}
              </div>
            </details>
          ) : null}
          <details className="mt-4 rounded-lg border border-white/10 bg-[#12151f] p-3">
            <summary className="cursor-pointer text-xs font-medium text-white/65">
              {tSlash("技术详情 JSON / Technical JSON")}
            </summary>
            <pre className="mt-3 max-h-72 overflow-auto text-xs text-white/80">
              {JSON.stringify(taskResult.result_json ?? null, null, 2)}
            </pre>
          </details>
        </div>
      ) : null}
    </section>
  );
}
