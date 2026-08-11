import { useEffect, useState } from "react";
import { Activity, History, Wrench } from "lucide-react";

import { ActiveTasksPanel, type ActiveTasksPanelProps } from "./ActiveTasksPanel";
import { ApprovalScopeGrantsPanel, type ApprovalScopeGrantsPanelProps } from "./ApprovalScopeGrantsPanel";
import { ManualTaskSubmitPanel, type ManualTaskSubmitPanelProps } from "./ManualTaskSubmitPanel";
import { TaskResultPanel, type TaskResultPanelProps } from "./TaskResultPanel";
import { TaskHistoryPanel, type TaskHistoryPanelProps } from "./TaskHistoryPanel";

export type TasksPageProps = ActiveTasksPanelProps &
  ApprovalScopeGrantsPanelProps &
  ManualTaskSubmitPanelProps &
  TaskHistoryPanelProps &
  TaskResultPanelProps & {
    taskHistoryLoaded: boolean;
  };

type TasksPageTab = "active" | "history" | "manual";

export function TasksPage(props: TasksPageProps) {
  const [activeTab, setActiveTab] = useState<TasksPageTab>("active");
  useEffect(() => {
    if (
      activeTab === "history" &&
      !props.taskHistoryLoaded &&
      !props.taskHistoryLoading &&
      !props.taskHistoryError
    ) {
      void props.onFetchTaskHistory(0);
    }
  }, [
    activeTab,
    props.taskHistoryLoaded,
    props.taskHistoryLoading,
    props.taskHistoryError,
    props.onFetchTaskHistory,
  ]);

  return (
    <>
      <header className="mb-5 border-b border-white/10 pb-5">
        <h2 className="text-xl font-semibold">{props.t("任务管理", "Task management")}</h2>
        <p className="mt-2 max-w-3xl text-sm leading-6 text-white/60">
          {props.t(
            "查看各个入口正在处理的任务；需要直接测试后端能力时，再使用高级手动任务。",
            "Review work currently running from every entry point. Use advanced manual tasks only when direct backend testing is needed.",
          )}
        </p>
      </header>

      <div
        role="tablist"
        aria-label={props.t("任务页面", "Task pages")}
        className="mb-5 grid gap-2 rounded-lg border border-white/10 bg-black/20 p-1 sm:inline-grid sm:grid-cols-3"
      >
        <button
          type="button"
          role="tab"
          aria-controls="active-tasks-panel"
          aria-selected={activeTab === "active"}
          onClick={() => setActiveTab("active")}
          className={`inline-flex min-h-10 items-center justify-center gap-2 rounded-md px-4 py-2 text-sm font-medium transition ${
            activeTab === "active"
              ? "theme-primary-btn"
              : "text-white/65 hover:bg-white/10 hover:text-white"
          }`}
        >
          <Activity className="h-4 w-4" />
          {props.t("正在处理", "Active")}
          <span className="rounded-md border border-current/20 px-1.5 py-0.5 text-[11px] leading-none">
            {props.activeTasks.length}
          </span>
        </button>
        <button
          type="button"
          role="tab"
          aria-controls="task-history-panel"
          aria-selected={activeTab === "history"}
          onClick={() => setActiveTab("history")}
          className={`inline-flex min-h-10 items-center justify-center gap-2 rounded-md px-4 py-2 text-sm font-medium transition ${
            activeTab === "history"
              ? "theme-primary-btn"
              : "text-white/65 hover:bg-white/10 hover:text-white"
          }`}
        >
          <History className="h-4 w-4" />
          {props.t("历史记录", "History")}
          {props.taskHistoryLoaded ? (
            <span className="rounded-md border border-current/20 px-1.5 py-0.5 text-[11px] leading-none">
              {props.taskHistoryTotal}
            </span>
          ) : null}
        </button>
        <button
          type="button"
          role="tab"
          aria-controls="manual-tasks-panel"
          aria-selected={activeTab === "manual"}
          onClick={() => setActiveTab("manual")}
          className={`inline-flex min-h-10 items-center justify-center gap-2 rounded-md px-4 py-2 text-sm font-medium transition ${
            activeTab === "manual"
              ? "theme-primary-btn"
              : "text-white/65 hover:bg-white/10 hover:text-white"
          }`}
        >
          <Wrench className="h-4 w-4" />
          {props.t("高级手动任务", "Advanced manual tasks")}
        </button>
      </div>

      {activeTab === "active" ? (
        <div id="active-tasks-panel" role="tabpanel" className="space-y-5">
          <ActiveTasksPanel {...props} />
          {props.taskLoading || props.taskResult || props.taskError ? <TaskResultPanel {...props} /> : null}
        </div>
      ) : activeTab === "history" ? (
        <div id="task-history-panel" role="tabpanel" className="space-y-5">
          <TaskHistoryPanel {...props} />
          {props.taskLoading || props.taskResult || props.taskError ? <TaskResultPanel {...props} /> : null}
        </div>
      ) : (
        <div id="manual-tasks-panel" role="tabpanel" className="space-y-5">
          <ManualTaskSubmitPanel {...props} />
          <TaskResultPanel {...props} />
          <ApprovalScopeGrantsPanel {...props} />
        </div>
      )}
    </>
  );
}
