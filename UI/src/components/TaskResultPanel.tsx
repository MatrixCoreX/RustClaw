import { Loader2, MessageCircle, Pause, Play, RefreshCw, Save, ShieldCheck, Trash2 } from "lucide-react";
import { useEffect, useState } from "react";

import {
  buildTaskLifecycleView,
  buildTaskPollingView,
  canPauseTaskControl,
  canResumeTaskControl,
  type TaskLifecycleLang,
} from "../lib/task-lifecycle";
import {
  buildReplaySummary,
  buildTaskApprovalRequest,
  buildTaskGoalView,
  buildTaskOutcome,
  buildTaskPermissionView,
  buildTaskTraceEventView,
  taskArtifactRefs,
  taskTraceEvents,
  type TaskOutcomeView,
  type TaskPermissionView,
} from "../lib/task-result";
import type { TaskLlmDebugResponse, TaskQueryResponse } from "../types/api";
import { TaskLlmTracePanel } from "./TaskLlmTracePanel";

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
  taskLlmDebug: TaskLlmDebugResponse | null;
  taskLlmDebugLoading: boolean;
  taskLlmDebugError: string | null;
  resumeDrafts: Record<string, string>;
  resumeSubmittingTaskId: string | null;
  taskControlSubmittingId: string | null;
  onTaskIdChange: (value: string) => void;
  onQueryTask: () => unknown | Promise<unknown>;
  onQueryTaskLlmDebug: (taskId?: string) => unknown | Promise<unknown>;
  onResumeDraftChange: (taskId: string, value: string) => void;
  onSubmitResume: (taskId: string) => unknown | Promise<unknown>;
  onApproveTask: (taskId: string, approvalRequestId: string) => unknown | Promise<unknown>;
  onControlTask: (control: "pause" | "resume", taskId: string) => unknown | Promise<unknown>;
  onControlTaskGoal: (
    operation: "edit" | "clear",
    taskId: string,
    goal?: Record<string, unknown>,
  ) => unknown | Promise<unknown>;
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
  taskLlmDebug,
  taskLlmDebugLoading,
  taskLlmDebugError,
  resumeDrafts,
  resumeSubmittingTaskId,
  taskControlSubmittingId,
  onTaskIdChange,
  onQueryTask,
  onQueryTaskLlmDebug,
  onResumeDraftChange,
  onSubmitResume,
  onApproveTask,
  onControlTask,
  onControlTaskGoal,
}: TaskResultPanelProps) {
  const taskOutcome = taskResult ? buildTaskOutcome(taskResult, lang) : null;
  const taskGoalView = taskResult ? buildTaskGoalView(taskResult, lang) : null;
  const taskLifecycleView = taskResult ? buildTaskLifecycleView(taskResult.lifecycle, taskResult.status, lang) : null;
  const taskPollingView = taskResult ? buildTaskPollingView(taskResult.lifecycle, lang) : null;
  const taskPermissionView = taskResult ? buildTaskPermissionView(taskResult, lang) : null;
  const taskEvents = taskResult ? taskTraceEvents(taskResult) : [];
  const artifactRefs = taskResult ? taskArtifactRefs(taskResult) : [];
  const replaySummary = taskResult ? buildReplaySummary(taskResult) : null;
  const approvalRequest = taskResult ? buildTaskApprovalRequest(taskResult) : null;
  const [goalObjectiveDraft, setGoalObjectiveDraft] = useState("");
  useEffect(() => {
    setGoalObjectiveDraft(taskGoalView?.objective ?? "");
  }, [taskGoalView?.objective, taskResult?.task_id]);
  const canPauseGoalTask = taskResult
    ? canPauseTaskControl(taskResult.lifecycle, taskResult.status)
    : false;
  const canResumeGoalTask = taskResult
    ? canResumeTaskControl(taskResult.lifecycle, taskResult.status)
    : false;
  const goalPauseSubmitting = taskResult
    ? taskControlSubmittingId === `pause:${taskResult.task_id}`
    : false;
  const goalResumeSubmitting = taskResult
    ? taskControlSubmittingId === `resume:${taskResult.task_id}`
    : false;
  const goalEditSubmitting = taskResult
    ? taskControlSubmittingId === `goal-edit:${taskResult.task_id}`
    : false;
  const goalClearSubmitting = taskResult
    ? taskControlSubmittingId === `goal-clear:${taskResult.task_id}`
    : false;
  const approvalSubmitting = taskResult
    ? taskControlSubmittingId === `approve:${taskResult.task_id}`
    : false;
  const approvalExpired = approvalRequest ? approvalRequest.expiresAt * 1000 <= Date.now() : false;
  const approvalPending = approvalRequest?.status === "pending" && !approvalExpired;

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
          {taskGoalView ? (
            <div className={`mt-4 rounded-xl border px-3 py-3 ${toneClassName(taskGoalView.tone)}`}>
              <div className="flex flex-wrap items-center justify-between gap-2">
                <p className="font-semibold">{taskGoalView.title}</p>
                <span className="theme-status-pill rounded-md px-2 py-1 font-mono text-xs">
                  {taskGoalView.status}
                </span>
              </div>
              {taskGoalView.objective ? (
                <p className="mt-2 text-sm opacity-85">{taskGoalView.objective}</p>
              ) : null}
              <div className="mt-3 flex flex-wrap gap-2 text-xs">
                {taskGoalView.meta.map((item) => (
                  <span key={item} className="rounded-md border border-white/10 bg-black/20 px-2 py-1 font-mono">
                    {item}
                  </span>
                ))}
              </div>
              <div className="mt-3 grid gap-2 md:grid-cols-[1fr_auto]">
                <input
                  className="theme-input text-xs"
                  value={goalObjectiveDraft}
                  onChange={(event) => setGoalObjectiveDraft(event.target.value)}
                  placeholder={t("目标说明", "Goal objective")}
                />
                <div className="flex flex-wrap gap-2">
                  <button
                    type="button"
                    onClick={() => void onControlTaskGoal("edit", taskResult.task_id, { objective: goalObjectiveDraft.trim() })}
                    disabled={goalEditSubmitting || !goalObjectiveDraft.trim()}
                    className="theme-secondary-btn px-3 py-2 text-xs disabled:cursor-not-allowed disabled:opacity-50"
                  >
                    {goalEditSubmitting ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Save className="h-3.5 w-3.5" />}
                    {t("保存目标", "Save goal")}
                  </button>
                  {canPauseGoalTask ? (
                    <button
                      type="button"
                      onClick={() => void onControlTask("pause", taskResult.task_id)}
                      disabled={goalPauseSubmitting}
                      className="theme-secondary-btn px-3 py-2 text-xs disabled:cursor-not-allowed disabled:opacity-50"
                    >
                      {goalPauseSubmitting ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Pause className="h-3.5 w-3.5" />}
                      {t("暂停", "Pause")}
                    </button>
                  ) : null}
                  {canResumeGoalTask ? (
                    <button
                      type="button"
                      onClick={() => void onControlTask("resume", taskResult.task_id)}
                      disabled={goalResumeSubmitting}
                      className="theme-secondary-btn px-3 py-2 text-xs disabled:cursor-not-allowed disabled:opacity-50"
                    >
                      {goalResumeSubmitting ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Play className="h-3.5 w-3.5" />}
                      {t("恢复", "Resume")}
                    </button>
                  ) : null}
                  <button
                    type="button"
                    onClick={() => void onControlTaskGoal("clear", taskResult.task_id)}
                    disabled={goalClearSubmitting}
                    className="theme-secondary-btn px-3 py-2 text-xs disabled:cursor-not-allowed disabled:opacity-50"
                  >
                    {goalClearSubmitting ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Trash2 className="h-3.5 w-3.5" />}
                    {t("清除目标", "Clear goal")}
                  </button>
                </div>
              </div>
              {[
                [t("完成条件", "Done conditions"), taskGoalView.doneConditions],
                [t("约束", "Constraints"), taskGoalView.constraints],
                [t("验证命令", "Verification commands"), taskGoalView.verificationCommands],
                [t("当前进度", "Current progress"), taskGoalView.currentProgress],
                [t("剩余工作", "Remaining work"), taskGoalView.remainingWork],
              ].some(([, items]) => Array.isArray(items) && items.length > 0) ? (
                <details className="mt-3 rounded-lg border border-white/10 bg-black/20 p-3">
                  <summary className="cursor-pointer text-xs font-medium opacity-75">
                    {t("目标字段", "Goal fields")}
                  </summary>
                  <div className="mt-3 space-y-2">
                    {[
                      [t("完成条件", "Done conditions"), taskGoalView.doneConditions],
                      [t("约束", "Constraints"), taskGoalView.constraints],
                      [t("验证命令", "Verification commands"), taskGoalView.verificationCommands],
                      [t("当前进度", "Current progress"), taskGoalView.currentProgress],
                      [t("剩余工作", "Remaining work"), taskGoalView.remainingWork],
                    ].map(([label, items]) => (
                      Array.isArray(items) && items.length > 0 ? (
                        <div key={String(label)}>
                          <p className="mb-1 text-[11px] font-medium opacity-60">{String(label)}</p>
                          <div className="flex flex-wrap gap-2">
                            {items.map((item) => (
                              <span key={item} className="rounded-md border border-white/10 bg-black/25 px-2 py-1 font-mono text-[11px] opacity-75">
                                {item}
                              </span>
                            ))}
                          </div>
                        </div>
                      ) : null
                    ))}
                  </div>
                </details>
              ) : null}
            </div>
          ) : null}
          {taskLifecycleView ? (
            <div className={`mt-4 rounded-xl border px-3 py-3 ${toneClassName(taskLifecycleView.tone)}`}>
              <div className="flex flex-wrap items-center justify-between gap-2">
                <p className="font-semibold">{t("执行状态", "Runtime lifecycle")}</p>
                <span className="theme-status-pill rounded-md px-2 py-1 text-xs font-medium">{taskLifecycleView.stateLabel}</span>
              </div>
              <p className="mt-1 text-sm opacity-80">{taskLifecycleView.detail}</p>
              <div className="mt-3 rounded-lg border border-white/10 bg-black/20 px-3 py-2 text-xs">
                <div className="font-medium">{t("下一步", "Next step")}</div>
                <p className="mt-1 font-medium">{taskLifecycleView.recommendedAction.label}</p>
                <p className="mt-1 opacity-75">{taskLifecycleView.recommendedAction.detail}</p>
              </div>
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
          {approvalRequest ? (
            <div className="mt-4 rounded-lg border border-amber-400/25 bg-amber-500/10 px-3 py-3 text-amber-50">
              <div className="flex flex-wrap items-start justify-between gap-3">
                <div>
                  <p className="font-semibold">
                    {approvalPending
                      ? t("需要你的授权", "Your approval is required")
                      : t("本次授权状态", "One-time approval status")}
                  </p>
                  <p className="mt-1 text-sm text-amber-50/80">
                    {approvalPending
                      ? t(
                          `RustClaw 准备执行 ${approvalRequest.actionCount} 项会修改数据或访问外部系统的操作。`,
                          `RustClaw is ready to run ${approvalRequest.actionCount} action(s) that may change data or access an external system.`,
                        )
                      : t(
                          "这条记录显示当前任务的一次性授权状态。",
                          "This record shows the task's current one-time approval state.",
                        )}
                  </p>
                </div>
                <span className="theme-status-pill rounded-md px-2 py-1 font-mono text-xs">
                  {approvalExpired ? t("已过期", "Expired") : approvalRequest.status}
                </span>
              </div>
              <div className="mt-3 flex flex-wrap gap-2 text-xs">
                {approvalRequest.targets.map((target) => (
                  <span key={target} className="rounded-md border border-white/10 bg-black/20 px-2 py-1 font-mono">
                    {target}
                  </span>
                ))}
              </div>
              <p className="mt-3 text-xs text-amber-50/70">
                {approvalRequest.reversible
                  ? t("这项操作支持恢复。", "This action can be reversed.")
                  : t("系统不能保证自动恢复这项操作。请确认目标无误。", "Automatic recovery is not guaranteed. Check the targets before approving.")}
              </p>
              <p className="mt-1 text-xs text-amber-50/60">
                {t("授权有效期至", "Approval expires at")}: {new Date(approvalRequest.expiresAt * 1000).toLocaleString(lang === "zh" ? "zh-CN" : "en-US")}
              </p>
              {approvalPending ? (
                <button
                  type="button"
                  onClick={() => void onApproveTask(taskResult.task_id, approvalRequest.requestId)}
                  disabled={approvalSubmitting || approvalExpired}
                  className="theme-accent-btn mt-3 text-xs disabled:cursor-not-allowed disabled:opacity-50"
                >
                  {approvalSubmitting ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <ShieldCheck className="h-3.5 w-3.5" />}
                  {t("仅授权这一次", "Approve once")}
                </button>
              ) : null}
              <details className="mt-3 rounded-lg border border-white/10 bg-black/20 p-3">
                <summary className="cursor-pointer text-xs font-medium opacity-75">
                  {t("技术详情", "Technical details")}
                </summary>
                <div className="mt-2 space-y-1 font-mono text-[11px] opacity-70">
                  <p>request_id={approvalRequest.requestId}</p>
                  <p>effect={approvalRequest.effect}</p>
                  <p>reason_code={approvalRequest.reasonCode}</p>
                </div>
              </details>
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
              {taskPermissionView.steps.length > 0 ? (
                <details className="mt-3 rounded-lg border border-white/10 bg-black/20 p-3">
                  <summary className="cursor-pointer text-xs font-medium opacity-75">
                    {t("权限步骤详情", "Permission step details")} · {taskPermissionView.steps.length}
                  </summary>
                  <div className="mt-3 space-y-3">
                    {taskPermissionView.steps.map((step, stepIndex) => (
                      <div key={`${step.title}-${stepIndex}`} className="rounded-lg border border-white/10 bg-black/20 px-3 py-2">
                        <p className="text-xs font-semibold opacity-90">{step.title}</p>
                        <div className="mt-2 flex flex-wrap gap-2">
                          {step.meta.map((item) => (
                            <span key={item} className="rounded-md border border-white/10 bg-white/5 px-2 py-1 font-mono text-[11px] opacity-75">
                              {item}
                            </span>
                          ))}
                        </div>
                        {[
                          [t("沙箱", "Sandbox"), step.sandbox],
                          [t("工作区", "Workspace"), step.workspace],
                          [t("Registry 策略", "Registry policy"), step.registryPolicy],
                        ].map(([label, items]) => (
                          Array.isArray(items) && items.length > 0 ? (
                            <div key={String(label)} className="mt-2">
                              <p className="mb-1 text-[11px] font-medium opacity-60">{String(label)}</p>
                              <div className="flex flex-wrap gap-2">
                                {items.map((item) => (
                                  <span key={item} className="rounded-md border border-white/10 bg-black/25 px-2 py-1 font-mono text-[11px] opacity-75">
                                    {item}
                                  </span>
                                ))}
                              </div>
                            </div>
                          ) : null
                        ))}
                      </div>
                    ))}
                  </div>
                </details>
              ) : null}
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
              {[
                [t("完成条件", "Done conditions"), taskOutcome.doneConditions],
                [t("约束", "Constraints"), taskOutcome.constraints],
                [t("验证", "Verification"), taskOutcome.verification],
                [t("当前进度", "Current progress"), taskOutcome.currentProgress],
                [t("剩余工作", "Remaining work"), taskOutcome.remainingWork],
              ].some(([, items]) => Array.isArray(items) && items.length > 0) ? (
                <details className="mt-3 rounded-lg border border-white/10 bg-black/20 p-3">
                  <summary className="cursor-pointer text-xs font-medium opacity-75">
                    {t("目标与完成度", "Goal and done state")}
                  </summary>
                  <div className="mt-3 space-y-2">
                    {[
                      [t("完成条件", "Done conditions"), taskOutcome.doneConditions],
                      [t("约束", "Constraints"), taskOutcome.constraints],
                      [t("验证", "Verification"), taskOutcome.verification],
                      [t("当前进度", "Current progress"), taskOutcome.currentProgress],
                      [t("剩余工作", "Remaining work"), taskOutcome.remainingWork],
                    ].map(([label, items]) => (
                      Array.isArray(items) && items.length > 0 ? (
                        <div key={String(label)}>
                          <p className="mb-1 text-[11px] font-medium opacity-60">{String(label)}</p>
                          <div className="flex flex-wrap gap-2">
                            {items.map((item) => (
                              <span key={item} className="rounded-md border border-white/10 bg-black/25 px-2 py-1 font-mono text-[11px] opacity-75">
                                {item}
                              </span>
                            ))}
                          </div>
                        </div>
                      ) : null
                    ))}
                  </div>
                </details>
              ) : null}
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
          <TaskLlmTracePanel
            t={t}
            tSlash={tSlash}
            taskResult={taskResult}
            taskLlmDebug={taskLlmDebug}
            taskLlmDebugLoading={taskLlmDebugLoading}
            taskLlmDebugError={taskLlmDebugError}
            onQueryTaskLlmDebug={onQueryTaskLlmDebug}
          />
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
