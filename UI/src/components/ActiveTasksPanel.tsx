import { Activity, CircleAlert, Clock3, Copy, Loader2, MessageCircle, Pause, Play, RefreshCw, X } from "lucide-react";

import { formatDuration } from "../lib/display-format";
import {
  buildTaskLifecycleView,
  buildTaskPollingView,
  buildTaskStatusSummary,
  type TaskStatusSummaryKind,
} from "../lib/task-lifecycle";
import type { ActiveTaskItem } from "../types/api";

type UiLanguage = "zh" | "en";
type Translate = (zh: string, en: string) => string;

export interface ActiveTasksPanelProps {
  lang: UiLanguage;
  t: Translate;
  activeTasks: ActiveTaskItem[];
  activeTasksLoading: boolean;
  activeTasksError: string | null;
  activeTasksLastUpdated: number | null;
  resumeTaskError: string | null;
  resumeTaskMessage: string | null;
  cancelTaskError: string | null;
  cancelTaskMessage: string | null;
  cancelingTaskIndex: number | null;
  taskControlSubmittingId: string | null;
  taskControlMessage: string | null;
  taskControlError: string | null;
  canUseInteractionContext: boolean;
  resumeDrafts: Record<string, string>;
  resumeSubmittingTaskId: string | null;
  toLocalTime: (value: number | null | undefined) => string;
  onFetchActiveTasks: () => unknown | Promise<unknown>;
  onViewTask: (taskId: string) => unknown | Promise<unknown>;
  onCancelTask: (task: ActiveTaskItem) => unknown | Promise<unknown>;
  onControlTask: (control: "pause" | "resume", taskId: string) => unknown | Promise<unknown>;
  onResumeDraftChange: (taskId: string, value: string) => void;
  onSubmitResume: (taskId: string) => unknown | Promise<unknown>;
}

export function ActiveTasksPanel({
  lang,
  t,
  activeTasks,
  activeTasksLoading,
  activeTasksError,
  activeTasksLastUpdated,
  resumeTaskError,
  resumeTaskMessage,
  cancelTaskError,
  cancelTaskMessage,
  cancelingTaskIndex,
  taskControlSubmittingId,
  taskControlMessage,
  taskControlError,
  canUseInteractionContext,
  resumeDrafts,
  resumeSubmittingTaskId,
  toLocalTime,
  onFetchActiveTasks,
  onViewTask,
  onCancelTask,
  onControlTask,
  onResumeDraftChange,
  onSubmitResume,
}: ActiveTasksPanelProps) {
  const summaryItems = buildTaskStatusSummary(activeTasks, lang);
  return (
    <section className="rounded-2xl border border-white/10 bg-white/5 p-5">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <p className="theme-kicker text-[10px] uppercase tracking-[0.3em]">{t("任务 inbox", "Task inbox")}</p>
          <h3 className="mt-2 text-lg font-semibold">{t("正在处理的任务", "Active tasks")}</h3>
          <p className="mt-1 text-sm text-white/55">
            {activeTasks.length > 0
              ? t(`当前有 ${activeTasks.length} 个任务还在排队或执行。`, `${activeTasks.length} task(s) are queued or running.`)
              : t("当前没有排队或执行中的任务。", "No queued or running tasks right now.")}
          </p>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          {activeTasksLastUpdated ? (
            <span className="text-xs text-white/45">{t("更新时间", "Updated")}: {toLocalTime(activeTasksLastUpdated)}</span>
          ) : null}
          <button
            type="button"
            onClick={() => void onFetchActiveTasks()}
            disabled={activeTasksLoading || !canUseInteractionContext}
            className="theme-topbar-btn px-3 py-2 text-xs font-medium disabled:cursor-not-allowed disabled:opacity-50"
          >
            {activeTasksLoading ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <RefreshCw className="h-3.5 w-3.5" />}
            {t("刷新任务", "Refresh tasks")}
          </button>
        </div>
      </div>
      {activeTasksError ? (
        <p className="mt-3 rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">
          {t("任务列表读取失败", "Task list failed")}: {activeTasksError}
        </p>
      ) : null}
      {resumeTaskError ? (
        <p className="mt-3 rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">
          {t("继续执行失败", "Resume failed")}: {resumeTaskError}
        </p>
      ) : null}
      {resumeTaskMessage ? (
        <p className="mt-3 rounded-lg border border-emerald-400/30 bg-emerald-500/10 px-3 py-2 text-sm text-emerald-100">
          {resumeTaskMessage}
        </p>
      ) : null}
      {cancelTaskError ? (
        <p className="mt-3 rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">
          {t("取消失败", "Cancel failed")}: {cancelTaskError}
        </p>
      ) : null}
      {cancelTaskMessage ? (
        <p className="mt-3 rounded-lg border border-emerald-400/30 bg-emerald-500/10 px-3 py-2 text-sm text-emerald-100">
          {cancelTaskMessage}
        </p>
      ) : null}
      {taskControlError ? (
        <p className="mt-3 rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">
          {t("任务控制失败", "Task control failed")}: {taskControlError}
        </p>
      ) : null}
      {taskControlMessage ? (
        <p className="mt-3 rounded-lg border border-emerald-400/30 bg-emerald-500/10 px-3 py-2 text-sm text-emerald-100">
          {taskControlMessage}
        </p>
      ) : null}
      <div className="mt-4 grid gap-2 sm:grid-cols-2 xl:grid-cols-4">
        {summaryItems.map((item) => {
          const Icon = taskSummaryIcon(item.kind);
          return (
            <div key={item.kind} className="rounded-xl border border-white/10 bg-black/20 px-3 py-3">
              <div className="flex items-center justify-between gap-3">
                <span className="text-xs font-medium text-white/60">{item.label}</span>
                <span className={taskSummaryIconClass(item.kind)}>
                  <Icon className="h-3.5 w-3.5" />
                </span>
              </div>
              <p className="mt-2 text-2xl font-semibold leading-none text-white">{item.count}</p>
            </div>
          );
        })}
      </div>
      <div className="mt-4 space-y-3">
        {activeTasks.length === 0 ? (
          <div className="rounded-xl border border-white/10 bg-black/20 px-4 py-4 text-sm text-white/55">
            {t("提交任务后，这里会显示排队、执行、等待恢复和后台轮询状态。", "After submitting tasks, queued, running, resumable, and background polling states appear here.")}
          </div>
        ) : (
          activeTasks.map((item) => {
            const lifecycleView = buildTaskLifecycleView(item.lifecycle, item.status, lang);
            const pollingView = buildTaskPollingView(item.lifecycle, lang);
            const childView = buildChildTaskView(item);
            const canPause = canPauseTask(item);
            const canResume = canResumeTask(item);
            const pauseSubmitting = taskControlSubmittingId === `pause:${item.task_id}`;
            const resumeSubmitting = taskControlSubmittingId === `resume:${item.task_id}`;
            return (
              <div
                key={item.task_id}
                className={`rounded-xl border bg-black/20 px-4 py-3 ${
                  childView ? "border-sky-300/25 border-l-4 border-l-sky-300/60" : "border-white/10"
                }`}
              >
                <div className="flex flex-wrap items-start justify-between gap-3">
                  <div className="min-w-0">
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="rounded-md border border-white/10 bg-white/5 px-2 py-1 text-xs text-white/60">#{item.index}</span>
                      <span className="theme-status-pill rounded-md px-2 py-1 text-xs font-medium">{lifecycleView.stateLabel}</span>
                      <span className="rounded-md border border-white/10 bg-white/5 px-2 py-1 text-xs text-white/60">{item.kind}</span>
                      {childView ? (
                        <span className="rounded-md border border-sky-300/20 bg-sky-400/10 px-2 py-1 text-xs font-medium text-sky-100">
                          {t("子任务", "Child task")}
                        </span>
                      ) : null}
                      <span className="text-xs text-white/45">{formatDuration(item.age_seconds)}</span>
                    </div>
                    <p className="mt-2 break-words text-sm text-white/85">{item.summary || item.task_id}</p>
                    {childView ? (
                      <div className="mt-2 flex flex-wrap gap-1.5 text-[11px] text-sky-50/75">
                        {childView.meta.map((meta) => (
                          <span key={`${item.task_id}-${meta}`} className="rounded-md border border-sky-300/20 bg-sky-400/10 px-2 py-1">
                            {meta}
                          </span>
                        ))}
                      </div>
                    ) : null}
                    <p className="mt-1 text-xs text-white/55">{lifecycleView.detail}</p>
                    <div className="mt-2 rounded-lg border border-white/10 bg-white/5 px-3 py-2 text-xs">
                      <div className="font-medium text-white/80">{t("下一步", "Next step")}</div>
                      <p className="mt-1 font-medium text-white/85">{lifecycleView.recommendedAction.label}</p>
                      <p className="mt-1 text-white/55">{lifecycleView.recommendedAction.detail}</p>
                    </div>
                    <p className="mt-1 break-all font-mono text-[11px] text-white/40">{item.task_id}</p>
                    <div className="mt-3 flex flex-wrap gap-1.5 text-[11px] text-white/55">
                      {lifecycleView.meta.slice(0, 4).map((meta) => (
                        <span key={`${item.task_id}-${meta}`} className="rounded-md border border-white/10 bg-white/5 px-2 py-1">
                          {meta}
                        </span>
                      ))}
                    </div>
                  </div>
                  <div className="flex flex-wrap gap-2">
                    <button
                      type="button"
                      onClick={() => void onViewTask(item.task_id)}
                      className="theme-secondary-btn px-3 py-2 text-xs"
                    >
                      {t("打开报告", "Open report")}
                    </button>
                    <button
                      type="button"
                      onClick={() => void copyTaskId(item.task_id)}
                      className="theme-secondary-btn px-3 py-2 text-xs"
                      title={t("复制任务 ID", "Copy task ID")}
                    >
                      <Copy className="h-3.5 w-3.5" />
                      {t("复制 ID", "Copy ID")}
                    </button>
                    {canPause ? (
                      <button
                        type="button"
                        onClick={() => void onControlTask("pause", item.task_id)}
                        disabled={pauseSubmitting || !canUseInteractionContext}
                        className="theme-secondary-btn px-3 py-2 text-xs disabled:cursor-not-allowed disabled:opacity-50"
                      >
                        {pauseSubmitting ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Pause className="h-3.5 w-3.5" />}
                        {t("暂停", "Pause")}
                      </button>
                    ) : null}
                    {canResume ? (
                      <button
                        type="button"
                        onClick={() => void onControlTask("resume", item.task_id)}
                        disabled={resumeSubmitting || !canUseInteractionContext}
                        className="theme-secondary-btn px-3 py-2 text-xs disabled:cursor-not-allowed disabled:opacity-50"
                      >
                        {resumeSubmitting ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Play className="h-3.5 w-3.5" />}
                        {t("恢复", "Resume")}
                      </button>
                    ) : null}
                  </div>
                  <div className="mt-3 border-t border-rose-300/20 pt-3">
                    <div className="flex flex-wrap items-center justify-between gap-2">
                      <div className="min-w-0">
                        <p className="flex items-center gap-1.5 text-xs font-medium text-rose-50">
                          <CircleAlert className="h-3.5 w-3.5" />
                          {t("停止这个任务", "Stop this task")}
                        </p>
                        <p className="mt-1 text-[11px] text-white/50">
                          {t("只在确定不需要继续执行时使用。", "Use this only when the task should not continue.")}
                        </p>
                      </div>
                    <button
                      type="button"
                      onClick={() => void onCancelTask(item)}
                      disabled={cancelingTaskIndex === item.index || !canUseInteractionContext || item.lifecycle?.can_cancel === false}
                      className="inline-flex items-center gap-1.5 rounded-md border border-rose-300/35 bg-rose-500/15 px-3 py-2 text-xs font-medium text-rose-50 transition hover:bg-rose-500/25 disabled:cursor-not-allowed disabled:opacity-50"
                    >
                      {cancelingTaskIndex === item.index ? (
                        <Loader2 className="h-3.5 w-3.5 animate-spin" />
                      ) : (
                        <X className="h-3.5 w-3.5" />
                      )}
                      {t("取消", "Cancel")}
                    </button>
                    </div>
                  </div>
                </div>
                {pollingView ? (
                  <div className="mt-3 rounded-lg border border-sky-400/20 bg-sky-500/10 px-3 py-2">
                    <p className="text-xs font-medium text-sky-50">{t("后台轮询", "Background polling")}</p>
                    <p className="mt-1 text-xs text-sky-50/75">{pollingView.detail}</p>
                    <div className="mt-2 flex flex-wrap gap-1.5 text-[11px] text-sky-50/75">
                      {pollingView.meta.map((meta) => (
                        <span key={`${item.task_id}-${meta}`} className="rounded-md border border-white/10 bg-black/20 px-2 py-1">
                          {meta}
                        </span>
                      ))}
                    </div>
                  </div>
                ) : null}
                {item.lifecycle?.state === "needs_user" ? (
                  <div className="mt-3 rounded-lg border border-amber-400/25 bg-amber-500/10 p-3">
                    <label className="block space-y-2">
                      <span className="text-xs font-medium text-amber-50">
                        {t("补充确认内容", "Follow-up input")}
                      </span>
                      <textarea
                        className="theme-input min-h-20"
                        value={resumeDrafts[item.task_id] ?? ""}
                        onChange={(event) => onResumeDraftChange(item.task_id, event.target.value)}
                        placeholder={t("输入确认或补充说明后继续执行", "Enter confirmation or follow-up text to continue")}
                      />
                    </label>
                    <button
                      type="button"
                      onClick={() => void onSubmitResume(item.task_id)}
                      disabled={resumeSubmittingTaskId === item.task_id || !(resumeDrafts[item.task_id] ?? "").trim()}
                      className="theme-accent-btn mt-3 text-xs disabled:cursor-not-allowed disabled:opacity-50"
                    >
                      {resumeSubmittingTaskId === item.task_id ? (
                        <Loader2 className="h-3.5 w-3.5 animate-spin" />
                      ) : (
                        <MessageCircle className="h-3.5 w-3.5" />
                      )}
                      {t("继续执行", "Resume")}
                    </button>
                  </div>
                ) : null}
                <details className="mt-3 rounded-lg border border-white/10 bg-[#12151f] px-3 py-2">
                  <summary className="cursor-pointer text-xs text-white/55">{t("生命周期机器字段", "Lifecycle machine fields")}</summary>
                  <pre className="mt-2 max-h-44 overflow-auto text-[11px] text-white/70">
                    {JSON.stringify(item.lifecycle ?? null, null, 2)}
                  </pre>
                </details>
              </div>
            );
          })
        )}
      </div>
    </section>
  );
}

async function copyTaskId(taskId: string): Promise<void> {
  const value = taskId.trim();
  if (!value || !navigator.clipboard?.writeText) return;
  await navigator.clipboard.writeText(value);
}

function canPauseTask(item: ActiveTaskItem): boolean {
  const state = (item.lifecycle?.state || item.status || "").toLowerCase();
  return !["succeeded", "failed", "cancelled", "canceled", "timeout", "needs_user"].includes(state);
}

function canResumeTask(item: ActiveTaskItem): boolean {
  const state = (item.lifecycle?.state || item.status || "").toLowerCase();
  return state === "waiting" || state === "background";
}

function buildChildTaskView(item: ActiveTaskItem): { meta: string[] } | null {
  const parentTaskId = item.lifecycle?.parent_task_id?.trim();
  if (!parentTaskId) return null;
  const meta = [`parent=${parentTaskId}`];
  const childTaskId = item.lifecycle?.child_task_id?.trim();
  if (childTaskId) meta.push(`child=${childTaskId}`);
  const role = item.lifecycle?.role?.trim();
  if (role) meta.push(`role=${role}`);
  const permissionProfile = item.lifecycle?.permission_profile?.trim();
  if (permissionProfile) meta.push(`profile=${permissionProfile}`);
  if (typeof item.lifecycle?.required === "boolean") {
    meta.push(`required=${item.lifecycle.required}`);
  }
  return { meta };
}

function taskSummaryIcon(kind: TaskStatusSummaryKind) {
  if (kind === "active") return Activity;
  if (kind === "failed") return CircleAlert;
  return Clock3;
}

function taskSummaryIconClass(kind: TaskStatusSummaryKind): string {
  if (kind === "active") return "rounded-md border border-cyan-300/20 bg-cyan-400/10 p-1.5 text-cyan-100";
  if (kind === "failed") return "rounded-md border border-red-300/20 bg-red-400/10 p-1.5 text-red-100";
  return "rounded-md border border-amber-300/20 bg-amber-400/10 p-1.5 text-amber-100";
}
