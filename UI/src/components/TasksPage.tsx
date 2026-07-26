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
      <header className="mb-5 border-b border-white/10 pb-5">
        <h2 className="text-xl font-semibold">{props.t("手动任务", "Manual tasks")}</h2>
        <p className="mt-2 max-w-3xl text-sm leading-6 text-white/60">
          {props.t(
            "这个页面用于直接创建和跟踪一条后台任务，适合测试技能、复现问题和查看详细执行结果。普通聊天和日常操作请优先使用 Agent 页面。",
            "Use this page to create and track a backend task directly. It is intended for skill testing, issue reproduction, and detailed execution results. Use the Agent page for normal conversations and everyday work.",
          )}
        </p>
        <p className="mt-2 max-w-3xl text-xs leading-5 text-white/45">
          {props.t(
            "task_id 是系统为每次执行生成的唯一查询编号，通常由字母、数字和短横线组成；这种 UUID 外观是正常的，不是乱码，也不是访问密钥。",
            "A task_id is the unique lookup reference generated for each run. It normally contains letters, numbers, and hyphens; this UUID format is expected, not corrupted text or an access key.",
          )}
        </p>
      </header>
      <ActiveTasksPanel {...props} />
      <ApprovalScopeGrantsPanel {...props} />
      <ManualTaskSubmitPanel {...props} />
      <TaskResultPanel {...props} />
    </>
  );
}
