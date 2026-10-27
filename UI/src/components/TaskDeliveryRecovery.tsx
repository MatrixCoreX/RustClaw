import { Check, RefreshCw } from "lucide-react";
import { taskStatusSource } from "../lib/task-status-facts";
import type { TaskLifecycleLang } from "../lib/task-lifecycle";
import type { TaskQueryResponse } from "../types/api";
import { useUiDialog } from "./UiDialogProvider";

export type DeliveryOperation = "confirm_received" | "retry_unknown" | "retry_failed";
export type DeliveryAction = (taskId: string, operation: DeliveryOperation, digest: string) => unknown | Promise<unknown>;

export function TaskDeliveryRecovery(props: { task: TaskQueryResponse; lang: TaskLifecycleLang; busy: boolean; onAction?: DeliveryAction }) {
  const raw = taskStatusSource(props.task).delivery;
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) return null;
  const delivery = raw as Record<string, unknown>;
  if (delivery.can_reconcile !== true || typeof delivery.snapshot_digest !== "string" || !props.onAction) return null;
  return <DeliveryActions {...props} onAction={props.onAction} digest={delivery.snapshot_digest}
    unknown={Number(delivery.unknown_count) || 0} retryable={Number(delivery.retryable_count) || 0} />;
}

function DeliveryActions({ task, lang, busy, onAction, digest, unknown, retryable }: {
  task: TaskQueryResponse; lang: TaskLifecycleLang; busy: boolean; onAction: DeliveryAction;
  digest: string; unknown: number; retryable: number;
}) {
  const { confirm } = useUiDialog();
  const t = (zh: string, en: string) => lang === "zh" ? zh : en;
  async function act(operation: DeliveryOperation) {
    const retry = operation === "retry_unknown";
    const accepted = await confirm({
      title: retry ? t("确认补发", "Confirm retry") : t("核对投送结果", "Reconcile delivery"),
      message: retry
        ? t("渠道尚未确认这些内容是否送达。请先检查原通信端；补发可能造成重复。确认没有收到，仍要补发吗？", "The channel has not confirmed these parts. Check the original chat first; retrying may cause duplicates. Confirm that you did not receive them and want to retry?")
        : operation === "confirm_received"
          ? t("仅将未确认的部分标记为你已收到，不会重新发送，也不会把你的确认当作渠道送达证明。", "Mark only the unconfirmed parts as received by you. Nothing is resent, and your confirmation is not provider delivery proof.")
          : t("仅补发渠道明确拒绝且允许重试的部分，已接收的内容不会重复发送。", "Retry only explicitly rejected, retryable parts. Accepted content is not resent."),
      confirmLabel: retry ? t("确认未收到，补发", "Not received, retry") : operation === "confirm_received" ? t("我已收到", "I received them") : t("补发失败部分", "Retry failed parts"),
      cancelLabel: t("取消", "Cancel"),
      tone: retry ? "danger" : "default",
    });
    if (accepted) await onAction(task.task_id, operation, digest);
  }
  return <div className="mt-3 flex flex-wrap gap-2">
    {unknown > 0 ? <>
      <button type="button" disabled={busy} className="theme-secondary-btn px-3 py-2 text-xs disabled:opacity-50" onClick={() => void act("confirm_received")}>
        <Check className="h-3.5 w-3.5" />{t("我已收到", "I received them")}
      </button>
      <button type="button" disabled={busy} className="theme-secondary-btn px-3 py-2 text-xs disabled:opacity-50" onClick={() => void act("retry_unknown")}>
        <RefreshCw className="h-3.5 w-3.5" />{t("核查并补发", "Check and retry")}
      </button>
    </> : null}
    {retryable > 0 ? <button type="button" disabled={busy} className="theme-secondary-btn px-3 py-2 text-xs disabled:opacity-50" onClick={() => void act("retry_failed")}>
      <RefreshCw className="h-3.5 w-3.5" />{t("补发失败部分", "Retry failed parts")}
    </button> : null}
  </div>;
}
