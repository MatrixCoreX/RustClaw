import { ChevronLeft, ChevronRight, History, Loader2, MessageCircle, RefreshCw, UserRound } from "lucide-react";

import { channelLabel } from "../lib/channel-display";
import { formatDuration } from "../lib/display-format";
import { buildTaskKindLabel, buildTaskLifecycleView } from "../lib/task-lifecycle";
import type { TaskHistoryItem } from "../types/api";
import { TaskIdCopyButton } from "./TaskIdCopyButton";

type UiLanguage = "zh" | "en";
type Translate = (zh: string, en: string) => string;

export interface TaskHistoryPanelProps {
  lang: UiLanguage;
  t: Translate;
  taskHistory: TaskHistoryItem[];
  taskHistoryLoading: boolean;
  taskHistoryError: string | null;
  taskHistoryTotal: number;
  taskHistoryOffset: number;
  taskHistoryLimit: number;
  onFetchTaskHistory: (offset?: number) => unknown | Promise<unknown>;
  onViewTask: (taskId: string) => unknown | Promise<unknown>;
}

function statusClass(tone: ReturnType<typeof buildTaskLifecycleView>["tone"]): string {
  if (tone === "ok") return "border-emerald-300/25 bg-emerald-400/10 text-emerald-100";
  if (tone === "attention") return "border-amber-300/25 bg-amber-400/10 text-amber-100";
  if (tone === "failed") return "border-rose-300/25 bg-rose-400/10 text-rose-100";
  return "border-sky-300/25 bg-sky-400/10 text-sky-100";
}

function historyTime(timestampSeconds: number, lang: UiLanguage): string {
  if (!Number.isFinite(timestampSeconds) || timestampSeconds <= 0) return "--";
  return new Date(timestampSeconds * 1000).toLocaleString(lang === "zh" ? "zh-CN" : "en-US");
}

export function TaskHistoryPanel({
  lang,
  t,
  taskHistory,
  taskHistoryLoading,
  taskHistoryError,
  taskHistoryTotal,
  taskHistoryOffset,
  taskHistoryLimit,
  onFetchTaskHistory,
  onViewTask,
}: TaskHistoryPanelProps) {
  const currentPage = Math.floor(taskHistoryOffset / Math.max(taskHistoryLimit, 1)) + 1;
  const totalPages = Math.max(1, Math.ceil(taskHistoryTotal / Math.max(taskHistoryLimit, 1)));
  const canGoBack = taskHistoryOffset > 0 && !taskHistoryLoading;
  const canGoForward = taskHistoryOffset + taskHistory.length < taskHistoryTotal && !taskHistoryLoading;

  return (
    <section className="rounded-2xl border border-white/10 bg-white/5 p-5">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <p className="theme-kicker text-[10px] uppercase tracking-[0.3em]">{t("任务记录", "Task records")}</p>
          <h3 className="mt-2 text-lg font-semibold">{t("历史记录", "History")}</h3>
          <p className="mt-1 text-sm text-white/55">
            {t(`共 ${taskHistoryTotal} 条已结束任务。`, `${taskHistoryTotal} completed task record(s).`)}
          </p>
        </div>
        <button
          type="button"
          onClick={() => void onFetchTaskHistory(taskHistoryOffset)}
          disabled={taskHistoryLoading}
          className="theme-topbar-btn px-3 py-2 text-xs disabled:cursor-not-allowed disabled:opacity-50"
        >
          {taskHistoryLoading ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <RefreshCw className="h-3.5 w-3.5" />}
          {t("刷新记录", "Refresh history")}
        </button>
      </div>

      {taskHistoryError ? (
        <p className="mt-3 rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">
          {t("历史记录读取失败", "Task history failed")}: {taskHistoryError}
        </p>
      ) : null}

      <div className="mt-4 space-y-3">
        {!taskHistoryLoading && taskHistory.length === 0 ? (
          <div className="rounded-lg border border-white/10 bg-black/20 px-4 py-6 text-center text-sm text-white/55">
            <History className="mx-auto mb-2 h-5 w-5" />
            {t("还没有已结束的任务记录。", "No completed task records yet.")}
          </div>
        ) : null}
        {taskHistory.map((item) => {
          const lifecycle = buildTaskLifecycleView(null, item.status, lang);
          const sourceUser = item.external_user_id?.trim() || item.source_user_id;
          return (
            <article key={item.task_id} className="rounded-lg border border-white/10 bg-black/20 px-4 py-3">
              <div className="flex flex-wrap items-start justify-between gap-3">
                <div className="min-w-0 flex-1">
                  <div className="flex flex-wrap items-center gap-2">
                    <span className={`rounded-md border px-2 py-1 text-xs font-medium ${statusClass(lifecycle.tone)}`}>
                      {lifecycle.stateLabel}
                    </span>
                    <span className="rounded-md border border-white/10 bg-white/5 px-2 py-1 text-xs text-white/60">
                      {buildTaskKindLabel(item.kind, lang)}
                    </span>
                    <span className="text-xs text-white/45">{historyTime(item.created_at_ts, lang)}</span>
                  </div>
                  <p className="mt-2 break-words text-sm text-white/85">{item.summary || item.task_id}</p>
                  <div className="mt-2 flex flex-wrap gap-2 text-xs text-white/60">
                    <span className="inline-flex items-center gap-1.5 rounded-md border border-white/10 bg-white/5 px-2 py-1">
                      <MessageCircle className="h-3.5 w-3.5" />
                      {t("来源", "Source")}: {channelLabel(item.channel, lang)}
                    </span>
                    <span className="inline-flex min-w-0 items-center gap-1.5 rounded-md border border-white/10 bg-white/5 px-2 py-1">
                      <UserRound className="h-3.5 w-3.5 shrink-0" />
                      <span className="shrink-0">{t("用户", "User")}:</span>
                      <span className="max-w-72 truncate font-mono" title={sourceUser}>{sourceUser}</span>
                    </span>
                    <span className="rounded-md border border-white/10 bg-white/5 px-2 py-1">
                      {t("耗时", "Duration")}: {formatDuration(item.duration_seconds)}
                    </span>
                  </div>
                  <p className="mt-2 break-all font-mono text-[11px] text-white/40">{item.task_id}</p>
                </div>
                <div className="flex flex-wrap gap-2">
                  <button type="button" onClick={() => void onViewTask(item.task_id)} className="theme-secondary-btn px-3 py-2 text-xs">
                    {t("打开报告", "Open report")}
                  </button>
                  <TaskIdCopyButton taskId={item.task_id} t={t} />
                </div>
              </div>
            </article>
          );
        })}
      </div>

      <div className="mt-4 flex flex-wrap items-center justify-between gap-3 border-t border-white/10 pt-4">
        <span className="text-xs text-white/50">
          {t(`第 ${currentPage} / ${totalPages} 页`, `Page ${currentPage} of ${totalPages}`)}
        </span>
        <div className="flex items-center gap-2">
          <button
            type="button"
            disabled={!canGoBack}
            onClick={() => void onFetchTaskHistory(Math.max(0, taskHistoryOffset - taskHistoryLimit))}
            className="theme-secondary-btn px-3 py-2 text-xs disabled:cursor-not-allowed disabled:opacity-50"
          >
            <ChevronLeft className="h-3.5 w-3.5" />
            {t("上一页", "Previous")}
          </button>
          <button
            type="button"
            disabled={!canGoForward}
            onClick={() => void onFetchTaskHistory(taskHistoryOffset + taskHistoryLimit)}
            className="theme-secondary-btn px-3 py-2 text-xs disabled:cursor-not-allowed disabled:opacity-50"
          >
            {t("下一页", "Next")}
            <ChevronRight className="h-3.5 w-3.5" />
          </button>
        </div>
      </div>
    </section>
  );
}
