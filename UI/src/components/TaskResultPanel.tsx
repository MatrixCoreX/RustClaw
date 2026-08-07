import {
  Archive,
  Bot,
  CheckCircle2,
  Circle,
  CircleSlash2,
  ListChecks,
  Loader2,
  MessageCircle,
  Pause,
  Play,
  RefreshCw,
  Save,
  Send,
  ShieldCheck,
  ShieldX,
  Square,
  Trash2,
  Users,
} from "lucide-react";
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
  buildSubagentPanelView,
  buildTaskApprovalRequest,
  buildTaskGoalView,
  buildTaskOutcome,
  buildTaskPermissionView,
  buildTaskPlanView,
  buildTaskTraceEventView,
  taskArtifactRefs,
  taskTraceEvents,
  type TaskOutcomeView,
  type TaskPermissionView,
  type SubagentNodeView,
  type SubagentPanelView,
} from "../lib/task-result";
import { buildTaskCostGovernance, formatUsdNanos } from "../lib/task-cost";
import type { TaskApprovalDecision, TaskLlmDebugResponse, TaskQueryResponse } from "../types/api";
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
  onDecideTaskApproval: (
    taskId: string,
    approvalRequestId: string,
    decision: TaskApprovalDecision,
  ) => unknown | Promise<unknown>;
  onControlTask: (control: "pause" | "resume", taskId: string) => unknown | Promise<unknown>;
  onControlSubagent: (
    control: "steer" | "pause" | "resume" | "stop" | "stop_all" | "close",
    parentTaskId: string,
    childTaskId?: string,
    userMessage?: string,
  ) => unknown | Promise<unknown>;
  onViewTask: (taskId: string) => unknown | Promise<unknown>;
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

function approvalActionLabel(actionRef: string, t: Translate): string {
  if (actionRef === "git.push") return t("即将推送这个提交", "Commit to push");
  if (actionRef === "forge.create_pr") return t("即将创建这个 Pull Request", "Pull request to create");
  return actionRef;
}

function approvalFieldLabel(name: string, t: Translate): string {
  const labels: Record<string, [string, string]> = {
    connection_id: ["连接", "Connection"],
    remote: ["本地远端名称", "Local remote"],
    local_branch: ["本地分支", "Local branch"],
    remote_branch: ["目标分支", "Target branch"],
    expected_local_sha: ["确认的完整提交 SHA", "Approved full commit SHA"],
    expected_remote_sha: ["远端当前 SHA", "Current remote SHA"],
    expected_remote_url_digest: ["远端地址指纹", "Remote URL fingerprint"],
    set_upstream: ["成功后设置 upstream", "Set upstream after success"],
    push_receipt_ref: ["已验证推送收据", "Verified push receipt"],
    expected_head_sha: ["PR 提交 SHA", "PR commit SHA"],
    head: ["来源分支", "Head branch"],
    base: ["目标分支", "Base branch"],
    title: ["标题", "Title"],
    body: ["说明", "Description"],
    draft: ["草稿 PR", "Draft PR"],
  };
  const label = labels[name];
  return label ? t(label[0], label[1]) : name;
}

function approvalFieldValue(value: string | number | boolean | null, t: Translate): string {
  if (value === null) return t("不存在（首次创建）", "Does not exist (new target)");
  if (value === true) return t("是", "Yes");
  if (value === false) return t("否", "No");
  return String(value);
}

function SubagentPanel({
  view,
  t,
  submittingId,
  onControl,
  onViewTask,
}: {
  view: SubagentPanelView;
  t: Translate;
  submittingId: string | null;
  onControl: TaskResultPanelProps["onControlSubagent"];
  onViewTask: TaskResultPanelProps["onViewTask"];
}) {
  const [steerDrafts, setSteerDrafts] = useState<Record<string, string>>({});
  const groups: Array<{ key: string; title: string; nodes: SubagentNodeView[] }> = [
    { key: "active", title: t("正在处理", "Active"), nodes: view.active },
    { key: "done", title: t("已完成", "Done"), nodes: view.done },
  ];
  const confirmStop = (label: string) => window.confirm(
    t(`确定停止${label}吗？已完成的工作会保留。`, `Stop ${label}? Completed work will be kept.`),
  );
  return (
    <div className="mt-4 rounded-xl border border-violet-300/20 bg-violet-500/8 px-3 py-3 text-white">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="flex min-w-0 items-start gap-2.5">
          <Users className="mt-0.5 h-5 w-5 shrink-0 text-violet-200" />
          <div>
            <p className="font-semibold">{t("并行任务", "Subagents")}</p>
            <p className="mt-1 text-xs text-white/65">
              {t(
                `正在处理 ${view.active.length} 项，已完成 ${view.done.length} 项。`,
                `${view.active.length} active, ${view.done.length} done.`,
              )}
            </p>
          </div>
        </div>
        {view.active.length > 0 ? (
          <button
            type="button"
            onClick={() => {
              if (confirmStop(t("全部并行任务", "all active parallel items"))) {
                void onControl("stop_all", view.parentTaskId);
              }
            }}
            disabled={submittingId === `subagent-stop_all:${view.parentTaskId}`}
            className="theme-secondary-btn px-3 py-2 text-xs text-red-100 disabled:opacity-50"
          >
            <Square className="h-3.5 w-3.5" />
            {t("停止全部", "Stop all")}
          </button>
        ) : null}
      </div>
      <div className="mt-3 grid gap-3 lg:grid-cols-2">
        {groups.map((group) => (
          <section key={group.key} className="rounded-lg border border-white/10 bg-black/15 p-3">
            <div className="flex items-center justify-between gap-2">
              <p className="text-xs font-semibold text-white/80">{group.title}</p>
              <span className="rounded-full bg-white/8 px-2 py-0.5 text-[11px] text-white/60">
                {group.nodes.length}
              </span>
            </div>
            {group.nodes.length > 0 ? (
              <div className="mt-2 space-y-2">
                {group.nodes.map((node) => {
                  const targetKey = `${node.childTaskId}`;
                  const isPaused = node.executionState === "paused";
                  const canResume = isPaused || node.executionState.startsWith("waiting");
                  const isClosed = node.threadState === "closed";
                  return (
                    <article key={node.childTaskId} className="rounded-lg border border-white/8 bg-black/20 p-2.5">
                      <div className="flex flex-wrap items-start justify-between gap-2">
                        <div className="flex min-w-0 items-start gap-2">
                          <Bot className="mt-0.5 h-4 w-4 shrink-0 text-violet-200" />
                          <div className="min-w-0">
                            <p className="truncate text-xs font-medium text-white">
                              {node.role} · {node.childTaskId}
                            </p>
                            <p className="mt-1 text-[11px] text-white/55">
                              {node.executionState} · {node.required ? t("必需", "Required") : t("可选", "Optional")}
                            </p>
                          </div>
                        </div>
                        <button
                          type="button"
                          onClick={() => void onViewTask(node.childTaskId)}
                          className="theme-secondary-btn px-2 py-1 text-[11px]"
                        >
                          {t("打开", "Open")}
                        </button>
                      </div>
                      {group.key === "active" ? (
                        <>
                          <div className="mt-2 flex flex-wrap gap-1.5">
                            {canResume ? (
                              <button
                                type="button"
                                onClick={() => void onControl("resume", view.parentTaskId, node.childTaskId)}
                                disabled={submittingId === `subagent-resume:${targetKey}`}
                                className="theme-secondary-btn px-2 py-1 text-[11px] disabled:opacity-50"
                              >
                                <Play className="h-3 w-3" /> {t("恢复", "Resume")}
                              </button>
                            ) : (
                              <button
                                type="button"
                                onClick={() => void onControl("pause", view.parentTaskId, node.childTaskId)}
                                disabled={submittingId === `subagent-pause:${targetKey}`}
                                className="theme-secondary-btn px-2 py-1 text-[11px] disabled:opacity-50"
                              >
                                <Pause className="h-3 w-3" /> {t("暂停", "Pause")}
                              </button>
                            )}
                            <button
                              type="button"
                              onClick={() => {
                                if (confirmStop(node.role)) void onControl("stop", view.parentTaskId, node.childTaskId);
                              }}
                              disabled={submittingId === `subagent-stop:${targetKey}`}
                              className="theme-secondary-btn px-2 py-1 text-[11px] text-red-100 disabled:opacity-50"
                            >
                              <Square className="h-3 w-3" /> {t("停止", "Stop")}
                            </button>
                          </div>
                          <div className="mt-2 grid grid-cols-[1fr_auto] gap-1.5">
                            <input
                              className="theme-input min-w-0 px-2 py-1 text-[11px]"
                              value={steerDrafts[node.childTaskId] ?? ""}
                              onChange={(event) => setSteerDrafts((current) => ({
                                ...current,
                                [node.childTaskId]: event.target.value,
                              }))}
                              placeholder={t("补充要求", "Add guidance")}
                            />
                            <button
                              type="button"
                              onClick={() => {
                                const message = steerDrafts[node.childTaskId]?.trim();
                                if (!message) return;
                                void onControl("steer", view.parentTaskId, node.childTaskId, message);
                                setSteerDrafts((current) => ({ ...current, [node.childTaskId]: "" }));
                              }}
                              disabled={!steerDrafts[node.childTaskId]?.trim() || submittingId === `subagent-steer:${targetKey}`}
                              className="theme-secondary-btn px-2 py-1 text-[11px] disabled:opacity-50"
                            >
                              <Send className="h-3 w-3" /> {t("发送", "Send")}
                            </button>
                          </div>
                        </>
                      ) : !isClosed ? (
                        <button
                          type="button"
                          onClick={() => void onControl("close", view.parentTaskId, node.childTaskId)}
                          disabled={submittingId === `subagent-close:${targetKey}`}
                          className="theme-secondary-btn mt-2 px-2 py-1 text-[11px] disabled:opacity-50"
                        >
                          <Archive className="h-3 w-3" /> {t("关闭", "Close")}
                        </button>
                      ) : null}
                      <details className="mt-2">
                        <summary className="cursor-pointer text-[11px] text-white/45">
                          {t("运行详情", "Runtime details")}
                        </summary>
                        <pre className="mt-2 max-h-40 overflow-auto rounded bg-black/25 p-2 text-[10px] text-white/60">
                          {JSON.stringify(node.raw, null, 2)}
                        </pre>
                      </details>
                    </article>
                  );
                })}
              </div>
            ) : (
              <p className="mt-2 text-xs text-white/40">{t("暂无项目", "No items")}</p>
            )}
          </section>
        ))}
      </div>
      <details className="mt-3 rounded-lg border border-white/8 bg-black/15 p-2.5">
        <summary className="cursor-pointer text-[11px] text-white/45">
          {t("容量与调度详情", "Capacity and scheduling details")}
        </summary>
        <div className="mt-2 flex flex-wrap gap-2 text-[11px] text-white/60">
          <span>{t("会话容量", "Session capacity")}: {view.sessionOpenCapacity ?? "--"}</span>
          <span>{t("已打开", "Open")}: {view.sessionOpenCount ?? "--"}</span>
          <span>{t("主任务不计入并行容量", "Main task is not counted")}</span>
        </div>
      </details>
    </div>
  );
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
  onDecideTaskApproval,
  onControlTask,
  onControlSubagent,
  onViewTask,
  onControlTaskGoal,
}: TaskResultPanelProps) {
  const taskOutcome = taskResult ? buildTaskOutcome(taskResult, lang) : null;
  const taskGoalView = taskResult ? buildTaskGoalView(taskResult, lang) : null;
  const taskLifecycleView = taskResult ? buildTaskLifecycleView(taskResult.lifecycle, taskResult.status, lang) : null;
  const taskPollingView = taskResult ? buildTaskPollingView(taskResult.lifecycle, lang) : null;
  const taskPermissionView = taskResult ? buildTaskPermissionView(taskResult, lang) : null;
  const taskPlanView = taskResult ? buildTaskPlanView(taskResult) : null;
  const subagentPanelView = taskResult ? buildSubagentPanelView(taskResult) : null;
  const taskCostView = taskResult ? buildTaskCostGovernance(taskResult) : null;
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
    ? taskControlSubmittingId === `approve_once:${taskResult.task_id}`
    : false;
  const approvalDenySubmitting = taskResult
    ? taskControlSubmittingId === `deny:${taskResult.task_id}`
    : false;
  const approvalScopeSubmitting = taskResult
    ? taskControlSubmittingId === `always_for_scope:${taskResult.task_id}`
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
          {taskPlanView ? (
            <div className="mt-4 rounded-xl border border-sky-400/30 bg-sky-500/10 px-3 py-3 text-white">
              <div className="flex flex-wrap items-start justify-between gap-3">
                <div className="flex min-w-0 items-start gap-2.5">
                  <ListChecks className="mt-0.5 h-5 w-5 shrink-0 text-sky-200" />
                  <div>
                    <p className="font-semibold">{t("当前执行计划", "Current plan")}</p>
                    <p className="mt-1 text-xs text-white/65">
                      {t(
                        `已完成 ${taskPlanView.completedCount} / ${taskPlanView.steps.length} 步`,
                        `${taskPlanView.completedCount} of ${taskPlanView.steps.length} steps completed`,
                      )}
                    </p>
                  </div>
                </div>
                <span className="rounded-md border border-sky-300/15 bg-black/15 px-2 py-1 font-mono text-[11px] text-white/70">
                  v{taskPlanView.planRevision}
                </span>
              </div>
              <div className="mt-3 h-1.5 overflow-hidden rounded-full bg-black/20">
                <div
                  className="h-full rounded-full bg-sky-300/70 transition-[width]"
                  style={{
                    width: `${Math.round(
                      (taskPlanView.completedCount / taskPlanView.steps.length) * 100,
                    )}%`,
                  }}
                />
              </div>
              <ol className="mt-3 space-y-2">
                {taskPlanView.steps.map((step, index) => {
                  const statusLabel =
                    step.status === "completed"
                      ? t("已完成", "Completed")
                      : step.status === "in_progress"
                        ? t("进行中", "In progress")
                        : step.status === "cancelled"
                          ? t("已取消", "Cancelled")
                          : t("待处理", "Pending");
                  const StepIcon =
                    step.status === "completed"
                      ? CheckCircle2
                      : step.status === "in_progress"
                        ? Loader2
                        : step.status === "cancelled"
                          ? CircleSlash2
                          : Circle;
                  return (
                    <li
                      key={step.stepId}
                      className="flex items-start gap-2.5 rounded-lg border border-white/8 bg-black/15 px-3 py-2.5"
                    >
                      <StepIcon
                        className={`mt-0.5 h-4 w-4 shrink-0 ${
                          step.status === "in_progress" ? "animate-spin text-sky-200" : "text-white/60"
                        }`}
                      />
                      <div className="min-w-0 flex-1">
                        <p className="break-words text-sm text-white">
                          <span className="mr-2 text-xs text-white/45">{index + 1}.</span>
                          {step.title}
                        </p>
                        <p className="mt-1 text-[11px] text-white/55">{statusLabel}</p>
                      </div>
                    </li>
                  );
                })}
              </ol>
              <details className="mt-3 rounded-lg border border-white/8 bg-black/15 p-3">
                <summary className="cursor-pointer text-xs text-white/60">
                  {t("技术详情（原始 JSON）", "Technical details (raw JSON)")}
                </summary>
                <pre className="mt-2 max-h-52 overflow-auto rounded-md bg-black/25 p-2 text-[11px] text-white/65">
                  {JSON.stringify(taskPlanView.raw, null, 2)}
                </pre>
              </details>
            </div>
          ) : null}
          {subagentPanelView ? (
            <SubagentPanel
              view={subagentPanelView}
              t={t}
              submittingId={taskControlSubmittingId}
              onControl={onControlSubagent}
              onViewTask={onViewTask}
            />
          ) : null}
          {taskCostView ? (
            <div className="mt-4 border-t border-white/10 pt-3">
              <div className="flex flex-wrap items-center justify-between gap-2">
                <p className="font-medium text-white/80">{t("模型用量与成本", "Model usage and cost")}</p>
                <span className="theme-status-pill rounded-md px-2 py-1 font-mono text-xs">
                  {taskCostView.budgetStatus ?? taskCostView.costStatus}
                </span>
              </div>
              <div className="mt-2 grid gap-2 text-xs sm:grid-cols-2 lg:grid-cols-4">
                <p>
                  <span className="text-white/50">{t("已估算", "Estimated")}: </span>
                  <span className="font-mono text-white/80">
                    {formatUsdNanos(taskCostView.taskKnownCostUsdNanos ?? taskCostView.estimatedCostUsdNanos)}
                  </span>
                </p>
                <p>
                  <span className="text-white/50">{t("软上限", "Soft limit")}: </span>
                  <span className="font-mono text-white/80">
                    {formatUsdNanos(taskCostView.softTaskLimitUsdNanos) ?? "--"}
                  </span>
                </p>
                <p>
                  <span className="text-white/50">{t("硬上限", "Hard limit")}: </span>
                  <span className="font-mono text-white/80">
                    {formatUsdNanos(taskCostView.hardTaskLimitUsdNanos) ?? "--"}
                  </span>
                </p>
                <p>
                  <span className="text-white/50">{t("未知计价记录", "Unknown price records")}: </span>
                  <span className="font-mono text-white/80">{taskCostView.unknownRecordCount}</span>
                </p>
              </div>
              {taskCostView.signals.length > 0 ? (
                <div className="mt-2 flex flex-wrap gap-2">
                  {taskCostView.signals.map((signal) => (
                    <span key={signal} className="rounded-md border border-white/10 bg-black/20 px-2 py-1 font-mono text-[11px] text-white/65">
                      {signal}
                    </span>
                  ))}
                </div>
              ) : null}
            </div>
          ) : null}
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
              {approvalRequest.previews.length > 0 ? (
                <div className="mt-3 space-y-3">
                  {approvalRequest.previews.map((preview, previewIndex) => (
                    <div key={`${preview.actionRef}:${previewIndex}`} className="rounded-lg border border-white/10 bg-black/20 p-3">
                      <p className="text-xs font-semibold">{approvalActionLabel(preview.actionRef, t)}</p>
                      <dl className="mt-2 grid gap-2 sm:grid-cols-2">
                        {preview.fields.map((field) => (
                          <div key={field.name} className={field.name === "body" ? "sm:col-span-2" : ""}>
                            <dt className="text-[11px] text-amber-50/60">{approvalFieldLabel(field.name, t)}</dt>
                            <dd className={`mt-0.5 break-all text-xs text-amber-50/90 ${field.name === "body" ? "max-h-40 overflow-auto whitespace-pre-wrap rounded bg-black/20 p-2" : "font-mono"}`}>
                              {approvalFieldValue(field.value, t)}
                            </dd>
                            {field.digest ? <p className="mt-0.5 break-all font-mono text-[10px] text-amber-50/45">digest={field.digest}</p> : null}
                          </div>
                        ))}
                      </dl>
                    </div>
                  ))}
                </div>
              ) : null}
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
              {taskResult?.lifecycle?.operation_progress ? (
                <div className="mt-3 rounded-lg border border-white/10 bg-black/15 px-3 py-2 text-xs">
                  <div className="flex flex-wrap items-center justify-between gap-2">
                    <span className="font-medium">{t("当前阶段", "Current phase")}</span>
                    <span className="theme-status-pill rounded-md px-2 py-1">
                      {taskResult.lifecycle.operation_progress.phase_key || t("处理中", "Working")}
                    </span>
                  </div>
                  <p className="mt-1 opacity-75">
                    {taskResult.lifecycle.operation_progress.total_units != null
                      ? t(
                          `已完成 ${taskResult.lifecycle.operation_progress.completed_units ?? 0} / ${taskResult.lifecycle.operation_progress.total_units}`,
                          `${taskResult.lifecycle.operation_progress.completed_units ?? 0} / ${taskResult.lifecycle.operation_progress.total_units} completed`,
                        )
                      : t("任务仍在运行，当前阶段无法可靠估算百分比。", "The task is alive; this phase has no reliable percentage estimate.")}
                  </p>
                </div>
              ) : null}
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
                          `{product_name} 准备执行 ${approvalRequest.actionCount} 项会修改数据或访问外部系统的操作。`,
                          `{product_name} is ready to run ${approvalRequest.actionCount} action(s) that may change data or access an external system.`,
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
              {approvalRequest.scopeGrant.available ? (
                <div className="mt-3 rounded-md border border-white/10 bg-black/20 px-3 py-2 text-xs">
                  <p className="font-medium">
                    {t("可限定授权范围", "Bounded approval is available")}
                  </p>
                  <p className="mt-1 text-amber-50/70">
                    {t(
                      "仅适用于当前会话中相同操作和相同资源，最长一小时，可随时撤销。",
                      "Only the same operation and resources in this session are covered, for up to one hour, and it can be revoked at any time.",
                    )}
                  </p>
                  <div className="mt-2 flex flex-wrap gap-2">
                    {approvalRequest.scopeGrant.entries.flatMap((entry) =>
                      entry.resources.map((resource) => (
                        <span key={`${entry.capability}:${resource}`} className="rounded-md border border-white/10 px-2 py-1 font-mono text-[11px]">
                          {entry.capability}: {resource}
                        </span>
                      )),
                    )}
                  </div>
                </div>
              ) : null}
              {approvalPending ? (
                <div className="mt-3 flex flex-wrap gap-2">
                  <button
                    type="button"
                    onClick={() => void onDecideTaskApproval(taskResult.task_id, approvalRequest.requestId, "approve_once")}
                    disabled={approvalSubmitting || approvalScopeSubmitting || approvalDenySubmitting || approvalExpired}
                    className="theme-accent-btn text-xs disabled:cursor-not-allowed disabled:opacity-50"
                  >
                    {approvalSubmitting ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <ShieldCheck className="h-3.5 w-3.5" />}
                    {t("仅授权这一次", "Approve once")}
                  </button>
                  {approvalRequest.scopeGrant.available ? (
                    <button
                      type="button"
                      onClick={() => void onDecideTaskApproval(taskResult.task_id, approvalRequest.requestId, "always_for_scope")}
                      disabled={approvalSubmitting || approvalScopeSubmitting || approvalDenySubmitting || approvalExpired}
                      className="inline-flex items-center justify-center gap-2 rounded-md border border-amber-200/25 bg-amber-100/10 px-3 py-2 text-xs font-medium transition hover:bg-amber-100/15 disabled:cursor-not-allowed disabled:opacity-50"
                    >
                      {approvalScopeSubmitting ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <ShieldCheck className="h-3.5 w-3.5" />}
                      {t("本会话相同范围", "Same scope in session")}
                    </button>
                  ) : null}
                  <button
                    type="button"
                    onClick={() => void onDecideTaskApproval(taskResult.task_id, approvalRequest.requestId, "deny")}
                    disabled={approvalSubmitting || approvalScopeSubmitting || approvalDenySubmitting || approvalExpired}
                    className="inline-flex items-center justify-center gap-2 rounded-md border border-white/15 bg-white/5 px-3 py-2 text-xs font-medium transition hover:bg-white/10 disabled:cursor-not-allowed disabled:opacity-50"
                  >
                    {approvalDenySubmitting ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <ShieldX className="h-3.5 w-3.5" />}
                    {t("拒绝这一次", "Deny")}
                  </button>
                </div>
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
