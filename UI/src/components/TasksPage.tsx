import { ActiveTasksPanel, type ActiveTasksPanelProps } from "./ActiveTasksPanel";
import { ApprovalScopeGrantsPanel, type ApprovalScopeGrantsPanelProps } from "./ApprovalScopeGrantsPanel";
import { ManualTaskSubmitPanel, type ManualTaskSubmitPanelProps } from "./ManualTaskSubmitPanel";
import { TaskResultPanel, type TaskResultPanelProps } from "./TaskResultPanel";

export type TasksPageProps = ActiveTasksPanelProps &
  ApprovalScopeGrantsPanelProps &
  ManualTaskSubmitPanelProps &
  TaskResultPanelProps;

export function TasksPage(props: TasksPageProps) {
  return (
    <>
      <ActiveTasksPanel {...props} />
      <ApprovalScopeGrantsPanel {...props} />
      <ManualTaskSubmitPanel {...props} />
      <TaskResultPanel {...props} />
    </>
  );
}
