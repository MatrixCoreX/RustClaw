import { ApiHttpError } from "./api-response";

export type UiErrorTranslate = (zh: string, en: string) => string;

const MACHINE_ERROR_TOKEN = /^[a-z][a-z0-9]*(?:[._:-][a-z0-9]+)+$/i;

export function formatUiError(
  cause: unknown,
  t: UiErrorTranslate,
  fallbackZh: string,
  fallbackEn: string,
): string {
  if (cause instanceof ApiHttpError) {
    if (cause.status === 413) return t("附件过大。每个文件最多 20 MiB，每次合计最多 60 MiB，请缩小后重试。", "Attachments are too large. Use at most 20 MiB per file and 60 MiB per message, then retry.");
    if (cause.status === 401 || cause.status === 403) return t("登录状态或权限已变更，请重新登录后重试。", "Your login or permissions have changed. Sign in again and retry.");
    if (cause.status === 429) return t("请求较多，请稍后重试。", "Too many requests. Please try again shortly.");
    return t(`服务器拒绝了请求（${cause.status}），请检查内容后重试。`, `The server rejected the request (${cause.status}). Check the content and retry.`);
  }
  const message = cause instanceof Error
    ? cause.message.trim()
    : typeof cause === "string"
      ? cause.trim()
      : "";
  if (!message || MACHINE_ERROR_TOKEN.test(message)) return t(fallbackZh, fallbackEn);
  return message;
}
