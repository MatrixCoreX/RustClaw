export type TaskLifecycleLang = "zh" | "en";

export interface TaskLifecycleProjection {
  schema_version?: number;
  state?: string;
  db_status?: string;
  source?: string;
  can_poll?: boolean;
  can_cancel?: boolean;
  last_heartbeat_ts?: number;
  next_check_after?: number;
  resume_due?: boolean;
  resume_wait_seconds?: number;
  resume_reason?: string;
  waiting_reason_code?: string;
  checkpoint_id?: string;
  pending_job_ref?: string;
  poll_ref?: string;
  cancel_ref?: string;
  next_action_kind?: string;
  next_action_ref?: string | number | boolean;
  next_poll_after?: number;
  resume_owner?: string;
  resume_entrypoint?: string;
  last_safe_step_id?: string;
  state_source?: string;
  terminal_reason?: string;
  parent_task_id?: string;
  child_task_id?: string;
  role?: string;
  required?: boolean;
  permission_profile?: string;
}

export interface TaskLifecycleView {
  stateLabel: string;
  detail: string;
  tone: "ok" | "running" | "attention" | "failed";
  meta: string[];
}

export interface TaskPollingView {
  detail: string;
  meta: string[];
}

export type TaskStatusSummaryKind = "active" | "waiting" | "needs_user" | "failed";

export interface TaskStatusSummaryInput {
  status: string;
  lifecycle?: TaskLifecycleProjection | null;
}

export interface TaskStatusSummaryItem {
  kind: TaskStatusSummaryKind;
  label: string;
  count: number;
  tone: TaskLifecycleView["tone"];
}

const STATE_LABELS: Record<string, { zh: string; en: string; tone: TaskLifecycleView["tone"] }> = {
  queued: { zh: "排队中", en: "Queued", tone: "running" },
  running: { zh: "执行中", en: "Running", tone: "running" },
  waiting: { zh: "等待中", en: "Waiting", tone: "attention" },
  background: { zh: "后台运行", en: "Background", tone: "running" },
  needs_user: { zh: "等待确认", en: "Needs input", tone: "attention" },
  succeeded: { zh: "已完成", en: "Completed", tone: "ok" },
  failed: { zh: "失败", en: "Failed", tone: "failed" },
  cancelled: { zh: "已取消", en: "Cancelled", tone: "failed" },
  canceled: { zh: "已取消", en: "Cancelled", tone: "failed" },
};

function t(lang: TaskLifecycleLang, zh: string, en: string): string {
  return lang === "zh" ? zh : en;
}

function stateToken(lifecycle: TaskLifecycleProjection | null | undefined, dbStatus: string): string {
  return (lifecycle?.state || dbStatus || "running").trim().toLowerCase();
}

function boolLabel(lang: TaskLifecycleLang, value: boolean | undefined): string {
  if (value === true) return t(lang, "是", "Yes");
  if (value === false) return t(lang, "否", "No");
  return "--";
}

function timestampLabel(lang: TaskLifecycleLang, ts: number | undefined): string | null {
  if (!Number.isFinite(ts) || !ts || ts <= 0) return null;
  const date = new Date(ts * 1000);
  if (Number.isNaN(date.getTime())) return null;
  return date.toLocaleString(lang === "zh" ? "zh-CN" : "en-US", {
    hour12: false,
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

export function buildTaskLifecycleView(
  lifecycle: TaskLifecycleProjection | null | undefined,
  dbStatus: string,
  lang: TaskLifecycleLang,
): TaskLifecycleView {
  const state = stateToken(lifecycle, dbStatus);
  const stateCopy = STATE_LABELS[state] ?? {
    zh: state || dbStatus || "--",
    en: state || dbStatus || "--",
    tone: "attention" as const,
  };
  const nextCheck = timestampLabel(lang, lifecycle?.next_check_after);
  const nextPoll = timestampLabel(lang, lifecycle?.next_poll_after);
  const heartbeat = timestampLabel(lang, lifecycle?.last_heartbeat_ts);
  const meta: string[] = [];
  if (lifecycle?.next_action_kind) meta.push(`${t(lang, "下一步", "Next action")}: ${lifecycle.next_action_kind}`);
  if (lifecycle?.next_action_ref !== undefined && lifecycle?.next_action_ref !== null) {
    meta.push(`${t(lang, "下一步引用", "Next action ref")}: ${String(lifecycle.next_action_ref)}`);
  }
  if (lifecycle?.waiting_reason_code) meta.push(`${t(lang, "等待原因", "Wait reason")}: ${lifecycle.waiting_reason_code}`);
  if (Number.isFinite(lifecycle?.resume_wait_seconds)) {
    meta.push(`${t(lang, "恢复等待", "Resume wait")}: ${Math.max(0, Number(lifecycle?.resume_wait_seconds))}s`);
  }
  if (lifecycle?.checkpoint_id) meta.push(`${t(lang, "检查点", "Checkpoint")}: ${lifecycle.checkpoint_id}`);
  if (lifecycle?.pending_job_ref) meta.push(`${t(lang, "后台任务", "Background job")}: ${lifecycle.pending_job_ref}`);
  if (nextPoll) meta.push(`${t(lang, "下次轮询", "Next poll")}: ${nextPoll}`);
  meta.push(
    `${t(lang, "可查询", "Pollable")}: ${boolLabel(lang, lifecycle?.can_poll)}`,
    `${t(lang, "可取消", "Cancelable")}: ${boolLabel(lang, lifecycle?.can_cancel)}`,
  );
  if (heartbeat) meta.push(`${t(lang, "最近心跳", "Last heartbeat")}: ${heartbeat}`);
  if (nextCheck) meta.push(`${t(lang, "下次检查", "Next check")}: ${nextCheck}`);
  if (lifecycle?.state_source) meta.push(`${t(lang, "状态来源", "State source")}: ${lifecycle.state_source}`);
  if (lifecycle?.poll_ref) meta.push(`${t(lang, "轮询引用", "Poll ref")}: ${lifecycle.poll_ref}`);
  if (lifecycle?.cancel_ref) meta.push(`${t(lang, "取消引用", "Cancel ref")}: ${lifecycle.cancel_ref}`);
  if (lifecycle?.resume_owner) meta.push(`${t(lang, "恢复执行者", "Resume owner")}: ${lifecycle.resume_owner}`);
  if (lifecycle?.resume_entrypoint) meta.push(`${t(lang, "恢复入口", "Resume entrypoint")}: ${lifecycle.resume_entrypoint}`);
  if (lifecycle?.last_safe_step_id) meta.push(`${t(lang, "安全步骤", "Safe step")}: ${lifecycle.last_safe_step_id}`);
  if (lifecycle?.terminal_reason) meta.push(`${t(lang, "结束原因", "Terminal reason")}: ${lifecycle.terminal_reason}`);

  let detail = t(lang, "任务状态来自当前任务记录。", "Status comes from the current task record.");
  if (state === "waiting" || state === "background") {
    if (lifecycle?.resume_due === true) {
      detail = t(lang, "恢复窗口已到，系统可以继续处理。", "The resume window is due and the system can continue.");
    } else {
      detail = lifecycle?.resume_reason
        ? `${t(lang, "恢复原因", "Resume reason")}: ${lifecycle.resume_reason}`
        : t(lang, "任务已进入可恢复状态。", "The task is in a resumable state.");
    }
  } else if (state === "queued" || state === "running") {
    detail = t(lang, "可以稍后刷新或取消。", "You can refresh later or cancel it.");
  } else if (state === "succeeded") {
    detail = t(lang, "任务已经完成。", "The task has completed.");
  } else if (state === "failed" || state === "cancelled" || state === "canceled") {
    detail = t(lang, "任务不会继续执行。", "The task will not continue.");
  }

  return {
    stateLabel: lang === "zh" ? stateCopy.zh : stateCopy.en,
    detail,
    tone: stateCopy.tone,
    meta,
  };
}

export function buildTaskPollingView(
  lifecycle: TaskLifecycleProjection | null | undefined,
  lang: TaskLifecycleLang,
): TaskPollingView | null {
  if (!lifecycle) return null;
  const nextPoll = timestampLabel(lang, lifecycle.next_poll_after);
  const nextCheck = timestampLabel(lang, lifecycle.next_check_after);
  const visible = Boolean(
    lifecycle.can_poll ||
      lifecycle.pending_job_ref ||
      lifecycle.poll_ref ||
      lifecycle.cancel_ref ||
      lifecycle.next_poll_after ||
      lifecycle.next_check_after,
  );
  if (!visible) return null;

  const meta: string[] = [];
  if (lifecycle.pending_job_ref) {
    meta.push(`${t(lang, "后台任务", "Background job")}: ${lifecycle.pending_job_ref}`);
  }
  if (lifecycle.poll_ref) {
    meta.push(`${t(lang, "轮询引用", "Poll ref")}: ${lifecycle.poll_ref}`);
  }
  if (nextPoll) {
    meta.push(`${t(lang, "下次轮询", "Next poll")}: ${nextPoll}`);
  }
  if (nextCheck) {
    meta.push(`${t(lang, "下次检查", "Next check")}: ${nextCheck}`);
  }
  meta.push(`${t(lang, "可查询", "Pollable")}: ${boolLabel(lang, lifecycle.can_poll)}`);
  meta.push(`${t(lang, "可取消", "Cancelable")}: ${boolLabel(lang, lifecycle.can_cancel)}`);
  if (lifecycle.cancel_ref) {
    meta.push(`${t(lang, "取消引用", "Cancel ref")}: ${lifecycle.cancel_ref}`);
  }

  return {
    detail:
      lifecycle.resume_due === true
        ? t(lang, "轮询窗口已到，可以继续检查后台结果。", "The polling window is due; the background result can be checked.")
        : t(lang, "这个任务可以在后台等待，并通过机器字段继续轮询。", "This task can wait in the background and continue polling through machine fields."),
    meta,
  };
}

export function buildTaskStatusSummary(
  tasks: TaskStatusSummaryInput[],
  lang: TaskLifecycleLang,
): TaskStatusSummaryItem[] {
  const counts: Record<TaskStatusSummaryKind, number> = {
    active: 0,
    waiting: 0,
    needs_user: 0,
    failed: 0,
  };
  for (const task of tasks) {
    const state = stateToken(task.lifecycle, task.status);
    if (state === "needs_user") {
      counts.needs_user += 1;
    } else if (state === "waiting") {
      counts.waiting += 1;
    } else if (state === "failed" || state === "cancelled" || state === "canceled" || state === "timeout") {
      counts.failed += 1;
    } else if (state === "queued" || state === "running" || state === "background") {
      counts.active += 1;
    }
  }
  return [
    {
      kind: "active",
      label: t(lang, "运行中", "Active"),
      count: counts.active,
      tone: "running",
    },
    {
      kind: "waiting",
      label: t(lang, "可恢复", "Resumable"),
      count: counts.waiting,
      tone: "attention",
    },
    {
      kind: "needs_user",
      label: t(lang, "待确认", "Needs input"),
      count: counts.needs_user,
      tone: "attention",
    },
    {
      kind: "failed",
      label: t(lang, "已停止", "Stopped"),
      count: counts.failed,
      tone: "failed",
    },
  ];
}
