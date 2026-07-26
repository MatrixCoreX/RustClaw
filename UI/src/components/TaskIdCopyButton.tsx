import { useEffect, useState } from "react";
import { Check, Copy, TriangleAlert } from "lucide-react";

import { writeTextToClipboard } from "../lib/clipboard";

type Translate = (zh: string, en: string) => string;
type CopyState = "idle" | "copied" | "error";

export interface TaskIdCopyButtonProps {
  taskId: string;
  t: Translate;
  className?: string;
}

export function TaskIdCopyButton({ taskId, t, className = "" }: TaskIdCopyButtonProps) {
  const [copyState, setCopyState] = useState<CopyState>("idle");

  useEffect(() => {
    setCopyState("idle");
  }, [taskId]);

  const copyTaskId = async () => {
    const exactTaskId = taskId.trim();
    if (!exactTaskId) return;
    try {
      await writeTextToClipboard(exactTaskId);
      setCopyState("copied");
    } catch {
      setCopyState("error");
    }
  };

  const label = copyState === "copied"
    ? t("已复制", "Copied")
    : copyState === "error"
      ? t("复制失败", "Copy failed")
      : t("复制 ID", "Copy ID");
  const Icon = copyState === "copied" ? Check : copyState === "error" ? TriangleAlert : Copy;

  return (
    <button
      type="button"
      onClick={() => void copyTaskId()}
      className={`theme-secondary-btn px-3 py-2 text-xs ${className}`.trim()}
      title={copyState === "error"
        ? t("浏览器不允许自动复制，请选中任务 ID 手动复制。", "Automatic copy is unavailable. Select the task ID and copy it manually.")
        : t("复制完整任务 ID", "Copy the complete task ID")}
      aria-live="polite"
    >
      <Icon className="h-3.5 w-3.5" />
      {label}
    </button>
  );
}
