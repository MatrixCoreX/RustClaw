import { Loader2, MessageCircle, RefreshCw, X } from "lucide-react";

import { formatDuration } from "../lib/display-format";
import { buildTaskLifecycleView } from "../lib/task-lifecycle";
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
  canUseInteractionContext: boolean;
  resumeDrafts: Record<string, string>;
  resumeSubmittingTaskId: string | null;
  toLocalTime: (value: number | null | undefined) => string;
  onFetchActiveTasks: () => unknown | Promise<unknown>;
  onViewTask: (taskId: string) => unknown | Promise<unknown>;
  onCancelTask: (task: ActiveTaskItem) => unknown | Promise<unknown>;
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
  canUseInteractionContext,
  resumeDrafts,
  resumeSubmittingTaskId,
  toLocalTime,
  onFetchActiveTasks,
  onViewTask,
  onCancelTask,
  onResumeDraftChange,
  onSubmitResume,
}: ActiveTasksPanelProps) {
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
      <div className="mt-4 space-y-3">
        {activeTasks.length === 0 ? (
          <div className="rounded-xl border border-white/10 bg-black/20 px-4 py-4 text-sm text-white/55">
            {t("提交任务后，这里会显示排队、执行、等待恢复和后台轮询状态。", "After submitting tasks, queued, running, resumable, and background polling states appear here.")}
          </div>
        ) : (
          activeTasks.map((item) => {
            const lifecycleView = buildTaskLifecycleView(item.lifecycle, item.status, lang);
            return (
              <div key={item.task_id} className="rounded-xl border border-white/10 bg-black/20 px-4 py-3">
                <div className="flex flex-wrap items-start justify-between gap-3">
                  <div className="min-w-0">
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="rounded-md border border-white/10 bg-white/5 px-2 py-1 text-xs text-white/60">#{item.index}</span>
                      <span className="theme-status-pill rounded-md px-2 py-1 text-xs font-medium">{lifecycleView.stateLabel}</span>
                      <span className="rounded-md border border-white/10 bg-white/5 px-2 py-1 text-xs text-white/60">{item.kind}</span>
                      <span className="text-xs text-white/45">{formatDuration(item.age_seconds)}</span>
                    </div>
                    <p className="mt-2 break-words text-sm text-white/85">{item.summary || item.task_id}</p>
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
                      {t("查看详情", "View details")}
                    </button>
                    <button
                      type="button"
                      onClick={() => void onCancelTask(item)}
                      disabled={cancelingTaskIndex === item.index || !canUseInteractionContext || item.lifecycle?.can_cancel === false}
                      className="theme-secondary-btn px-3 py-2 text-xs disabled:cursor-not-allowed disabled:opacity-50"
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
