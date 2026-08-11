import { useEffect, useState } from "react";
import { Activity, ArrowLeft, FileText, History, Wrench } from "lucide-react";

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

export type TasksPageTab = "active" | "history" | "manual" | "report";
type TasksPageListTab = Exclude<TasksPageTab, "report">;

export function taskReportReturnTab(activeTab: TasksPageTab): TasksPageListTab {
  return activeTab === "report" ? "active" : activeTab;
}

export function TasksPage(props: TasksPageProps) {
  const [activeTab, setActiveTab] = useState<TasksPageTab>("active");
  const [reportReturnTab, setReportReturnTab] = useState<TasksPageListTab>("active");
  const openTaskReport = (taskId: string) => {
    if (activeTab !== "report") {
      setReportReturnTab(taskReportReturnTab(activeTab));
    }
    setActiveTab("report");
    return props.onViewTask(taskId);
  };
  const showReportTab = () => {
    if (activeTab !== "report") {
      setReportReturnTab(taskReportReturnTab(activeTab));
    }
    setActiveTab("report");
  };
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
        className="mb-5 grid gap-2 rounded-lg border border-white/10 bg-black/20 p-1 sm:inline-grid sm:grid-cols-4"
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
        <button
          type="button"
          role="tab"
          aria-controls="task-report-panel"
          aria-selected={activeTab === "report"}
          onClick={showReportTab}
          className={`inline-flex min-h-10 items-center justify-center gap-2 rounded-md px-4 py-2 text-sm font-medium transition ${
            activeTab === "report"
              ? "theme-primary-btn"
              : "text-white/65 hover:bg-white/10 hover:text-white"
          }`}
        >
          <FileText className="h-4 w-4" />
          {props.t("任务报告", "Task report")}
        </button>
      </div>

      {activeTab === "active" ? (
        <div id="active-tasks-panel" role="tabpanel" className="space-y-5">
          <ActiveTasksPanel {...props} onViewTask={openTaskReport} />
        </div>
      ) : activeTab === "history" ? (
        <div id="task-history-panel" role="tabpanel" className="space-y-5">
          <TaskHistoryPanel {...props} onViewTask={openTaskReport} />
        </div>
      ) : activeTab === "manual" ? (
        <div id="manual-tasks-panel" role="tabpanel" className="space-y-5">
          <ManualTaskSubmitPanel {...props} />
          <TaskResultPanel {...props} />
          <ApprovalScopeGrantsPanel {...props} />
        </div>
      ) : (
        <div id="task-report-panel" role="tabpanel" className="space-y-4">
          <button
            type="button"
            onClick={() => setActiveTab(reportReturnTab)}
            className="theme-secondary-btn px-3 py-2 text-xs"
          >
            <ArrowLeft className="h-3.5 w-3.5" />
            {props.t("返回任务列表", "Back to task list")}
          </button>
          <TaskResultPanel {...props} onViewTask={openTaskReport} />
        </div>
      )}
    </>
  );
}
