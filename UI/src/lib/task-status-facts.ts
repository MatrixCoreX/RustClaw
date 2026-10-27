import type { TaskQueryResponse } from "../types/api";
import type { TaskLifecycleLang } from "./task-lifecycle";

export interface TaskStatusFact {
  kind: "execution" | "verification" | "delivery";
  label: string;
  status: string;
  text: string;
  tone: "ok" | "running" | "attention" | "neutral";
}

function record(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown> : null;
}

export function taskStatusSource(task: TaskQueryResponse): Record<string, unknown> {
  const result = record(task.result_json);
  return record(record(result?.task_journal)?.summary) ?? {};
}

export function buildTaskStatusFacts(task: TaskQueryResponse, lang: TaskLifecycleLang): TaskStatusFact[] {
  const summary = taskStatusSource(task);
  const pick = (zh: string, en: string) => lang === "zh" ? zh : en;
  const definitions: Array<[TaskStatusFact["kind"], string, unknown, Record<string, [string, string, TaskStatusFact["tone"]]>]> = [
    ["execution", pick("执行", "Execution"), summary.execution_status, {
      succeeded: ["已完成", "Completed", "ok"], partial: ["部分完成", "Partially completed", "attention"],
      failed: ["执行失败", "Failed", "attention"], waiting: ["等待中", "Waiting", "running"],
      needs_user: ["需要确认", "Needs input", "attention"], not_attempted: ["未执行工具", "No tool execution", "neutral"],
    }],
    ["verification", pick("答案验证", "Answer verification"), record(summary.answer_verification)?.status, {
      passed: ["已通过", "Passed", "ok"], failed: ["未通过", "Not passed", "attention"],
      unavailable: ["暂不可用", "Unavailable", "attention"], not_required: ["无需验证", "Not required", "neutral"],
    }],
    ["delivery", pick("投送", "Delivery"), record(summary.delivery)?.status, {
      accepted: ["渠道已接收", "Accepted by channel", "ok"], delivered: ["已送达", "Delivered", "ok"],
      user_confirmed: ["你已确认收到", "Confirmed received by you", "ok"],
      read: ["已读", "Read", "ok"], partial: ["部分已接收", "Partially accepted", "attention"],
      query_required: ["需要核查", "Needs reconciliation", "attention"], failed: ["未送达", "Not delivered", "attention"],
      pending: ["等待投送", "Pending", "running"], in_progress: ["投送中", "Sending", "running"],
      not_required: ["当前页面交付", "Shown in this page", "neutral"],
      unavailable: ["状态暂不可用", "Status unavailable", "attention"],
    }],
  ];
  return definitions.flatMap(([kind, label, raw, labels]) => {
    if (typeof raw !== "string" || !Object.hasOwn(labels, raw)) return [];
    const [zh, en, tone] = labels[raw];
    return [{ kind, label, status: raw, text: pick(zh, en), tone }];
  });
}
