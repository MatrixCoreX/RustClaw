import { ActiveTasksPanel, type ActiveTasksPanelProps } from "./ActiveTasksPanel";
import { ManualTaskSubmitPanel, type ManualTaskSubmitPanelProps } from "./ManualTaskSubmitPanel";
import { TaskResultPanel, type TaskResultPanelProps } from "./TaskResultPanel";

export type TasksPageProps = ActiveTasksPanelProps & ManualTaskSubmitPanelProps & TaskResultPanelProps;

export function TasksPage(props: TasksPageProps) {
  return (
    <>
      <ActiveTasksPanel {...props} />
      <ManualTaskSubmitPanel {...props} />
      <TaskResultPanel {...props} />
    </>
  );
}
