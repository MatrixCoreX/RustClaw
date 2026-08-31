import type { ApiResponse } from "../types/api";

type Translate = (zh: string, en: string) => string;

function machineErrorCode(body: ApiResponse<Record<string, unknown>>): string {
  const data = body.data;
  if (data && typeof data === "object") {
    const errorCode = data.error_code;
    if (typeof errorCode === "string" && errorCode.trim()) return errorCode.trim();
    const statusCode = data.status_code;
    if (typeof statusCode === "string" && statusCode.trim()) return statusCode.trim();
  }
  return body.error?.trim() ?? "";
}

export function formatSystemActionError(
  body: ApiResponse<Record<string, unknown>>,
  status: number,
  t: Translate,
): string {
  const code = machineErrorCode(body);
  if (code === "admin_role_required") {
    return t("此操作需要管理员权限。", "This action requires administrator access.");
  }
  if (code === "system_restart_schedule_failed") {
    return t("系统重启未能启动，请查看服务日志后重试。", "The system restart could not be scheduled. Check service logs and try again.");
  }
  if (code === "pi_app_restart_unavailable") {
    return t("当前设备不支持 Pi App 重启。", "Pi App restart is not available on this device.");
  }
  if (code === "pi_app_restart_schedule_failed") {
    return t("Pi App 重启未能启动，请查看服务日志后重试。", "The Pi App restart could not be scheduled. Check service logs and try again.");
  }
  return t(
    `系统操作未完成 (${status})，请稍后重试；如果仍然失败，请查看日志。`,
    `The system action did not complete (${status}). Try again later; if it still fails, check the logs.`,
  );
}
