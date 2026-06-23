import type { AdapterHealthRow, ApiResponse } from "../types/api";

type Translate = (zh: string, en: string) => string;

export type ServiceRuntimeName = AdapterHealthRow["serviceName"];
export type ServiceActionName = "start" | "stop" | "restart";

function recordAt(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

function stringAt(root: unknown, path: string[]): string | undefined {
  let current: unknown = root;
  for (const key of path) {
    const record = recordAt(current);
    if (!record) return undefined;
    current = record[key];
  }
  return typeof current === "string" && current.trim() ? current.trim() : undefined;
}

export function serviceDisplayName(key: AdapterHealthRow["key"], t: Translate): string {
  const labels: Record<AdapterHealthRow["key"], string> = {
    telegram_bot: t("Telegram 机器人", "Telegram Bot"),
    whatsapp_web: t("WhatsApp 网页版", "WhatsApp Web"),
    whatsapp_cloud: t("WhatsApp 云接口", "WhatsApp Cloud"),
    wechat_bot: t("微信通道", "WeChat Channel"),
    feishu_bot: t("飞书机器人", "Feishu Bot"),
    lark_bot: t("Lark 机器人", "Lark Bot"),
  };
  return labels[key];
}

export function serviceActionLabel(serviceName: ServiceRuntimeName, t: Translate): string {
  const labels: Record<ServiceRuntimeName, string> = {
    telegramd: "Telegram",
    whatsappd: "WhatsApp",
    whatsapp_webd: "WhatsApp Web",
    wechatd: t("微信", "WeChat"),
    feishud: t("飞书", "Feishu"),
    larkd: "Lark",
  };
  return labels[serviceName];
}

export function serviceActionErrorCode(body: ApiResponse<Record<string, unknown>>): string {
  return stringAt(body.data, ["error_code"]) || stringAt(body.data, ["status_code"]) || body.error?.trim() || "";
}

export function formatServiceActionError(
  serviceName: ServiceRuntimeName,
  action: ServiceActionName,
  errorCode: string,
  t: Translate,
): string {
  const serviceLabel = serviceActionLabel(serviceName, t);
  const actionLabel =
    action === "start" ? t("启动", "start") : action === "restart" ? t("重启", "restart") : t("停止", "stop");

  if (errorCode === "service_start_not_running" || errorCode === "service_restart_not_running") {
    return t(
      `${serviceLabel}服务还没有准备好，${actionLabel}暂时没有完成。请先确认配置已保存，稍等 2 到 3 秒后再试；如果还是失败，再到日志页面查看 ${serviceName}.log。`,
      `${serviceLabel} is not ready yet, so the ${actionLabel} action did not finish. Make sure the configuration is saved, wait 2 to 3 seconds, and try again. If it still fails, check ${serviceName}.log on the Logs page.`,
    );
  }

  if (errorCode === "service_disabled") {
    return t(
      `${serviceLabel}服务当前没有启用，请先完成配置并保存后再试。`,
      `${serviceLabel} is not enabled yet. Finish the configuration and save it before trying again.`,
    );
  }

  if (errorCode === "feishu_credentials_missing") {
    return t(
      `${serviceLabel}还缺少 App ID 或 App Secret。先把这两项填好并保存，再启动服务。`,
      `${serviceLabel} still needs an App ID or App Secret. Fill them in, save, and then start the service.`,
    );
  }

  if (errorCode === "feishu_webhook_credentials_missing") {
    return t(
      `${serviceLabel}当前是 webhook 模式，还需要 Verification Token 或 Encrypt Key，补齐后才能启动。`,
      `${serviceLabel} is in webhook mode and still needs a Verification Token or Encrypt Key before it can start.`,
    );
  }

  if (errorCode === "service_gateway_managed") {
    return t(
      `${serviceLabel}当前是由 channel-gateway 统一托管的，不能在这个单独按钮里${actionLabel}。请改为重启 channel-gateway，或先切回独立 ${serviceLabel} 进程。`,
      `${serviceLabel} is currently managed by channel-gateway, so it cannot be ${actionLabel}ed from this per-service button. Restart channel-gateway instead, or switch back to a dedicated ${serviceLabel} process first.`,
    );
  }

  return t(
    `${serviceLabel}服务操作没有成功，请稍后再试。需要的话，可以到日志页面查看 ${serviceName}.log。`,
    `The ${serviceLabel} action did not complete. Please try again shortly. If needed, check ${serviceName}.log on the Logs page.`,
  );
}

export function serviceActionSuccessMessage(
  serviceName: ServiceRuntimeName,
  action: ServiceActionName,
  t: Translate,
): string {
  const label = serviceActionLabel(serviceName, t);
  if (action === "restart") return t(`${label}服务已重启。`, `${label} was restarted.`);
  if (action === "start") return t(`${label}服务已启动。`, `${label} started.`);
  return t(`${label}服务已停止。`, `${label} stopped.`);
}
