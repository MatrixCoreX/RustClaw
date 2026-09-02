import { Database, Loader2, LogOut, Power, QrCode, RefreshCw, RotateCcw, Server, Square } from "lucide-react";

import type {
  ServiceActionNotice,
  TelegramBotConfigItem,
  WhatsappWebLoginStatus,
  WechatLoginStatus,
} from "../types/api";
import {
  isFeishuBindTerminalStatus,
  type FeishuBindSessionResponse,
  type FeishuBindStatusCopy,
  type FeishuSetupGuidance,
} from "../lib/feishu-bind";
import {
  serviceControlActions,
  type CommunicationServiceAction,
} from "../lib/communication-service-controls";
import { formatBytes } from "../lib/display-format";
import { formatUiError } from "../lib/ui-error";

type UiLanguage = "zh" | "en";
type Translate = (zh: string, en: string) => string;
type SetupStepStatus = "done" | "attention" | "todo";
type ServiceName = "telegramd" | "whatsappd" | "whatsapp_webd" | "wechatd" | "feishud" | "larkd";
type ServiceAction = CommunicationServiceAction;

function whatsappWebLoginErrorCopy(t: Translate, errorCode?: string | null): string | null {
  if (!errorCode) return null;
  switch (errorCode) {
    case "connection_closed":
      return t("连接已断开，适配器正在尝试重连。", "The connection closed and the adapter is reconnecting.");
    case "logged_out":
      return t("登录设备已移除，请重新扫码连接。", "The linked device was removed. Scan again to reconnect.");
    case "qr_render_failed":
      return t("二维码暂时无法显示，请刷新状态。", "The QR code could not be displayed. Refresh the status.");
    case "reconnect_failed":
      return t("自动重连失败，请重启服务后重新扫码。", "Automatic reconnection failed. Restart the service and scan again.");
    default:
      return t("实验适配器遇到连接错误，请按诊断编号排查。", "The experimental adapter encountered a connection error. Use the diagnostic ID for troubleshooting.");
  }
}

interface ChannelServiceControlsProps {
  t: Translate;
  serviceName: ServiceName;
  serviceLabelZh: string;
  serviceLabelEn: string;
  healthy: boolean;
  loading: boolean;
  disabled: boolean;
  allowReset?: boolean;
  className?: string;
  onControlService: CommunicationSetupPageProps["onControlService"];
}

export interface AgentAppSetupState {
  statusLoading: boolean;
  stepStatus: SetupStepStatus;
  statusSummary: string;
  configError: string | null;
  setupGuidance: FeishuSetupGuidance;
  currentKeyBound: boolean;
  bindQrDataUrl: string | null;
  bindStatusCopy: FeishuBindStatusCopy;
  bindSession: FeishuBindSessionResponse | null;
  bindError: string | null;
  bindLoading: boolean;
  resetLoading: boolean;
  serviceHealthy: boolean;
  canControlService: boolean;
  onBeginBind: () => unknown | Promise<unknown>;
  onResetSetup: () => unknown | Promise<unknown>;
}

function ChannelServiceControls({
  t,
  serviceName,
  serviceLabelZh,
  serviceLabelEn,
  healthy,
  loading,
  disabled,
  allowReset = true,
  className = "",
  onControlService,
}: ChannelServiceControlsProps) {
  const actionLabel = (action: ServiceAction) => {
    if (action === "start") {
      return t(`启动${serviceLabelZh}服务`, `Start ${serviceLabelEn} service`);
    }
    if (action === "restart") {
      return t(`重启${serviceLabelZh}服务`, `Restart ${serviceLabelEn} service`);
    }
    if (action === "reset") {
      return t(`重置${serviceLabelZh}`, `Reset ${serviceLabelEn}`);
    }
    return t(`关闭${serviceLabelZh}服务`, `Stop ${serviceLabelEn} service`);
  };

  return serviceControlActions(healthy, allowReset).map((action) => {
    const ActionIcon = action === "stop"
      ? Square
      : action === "restart"
        ? RefreshCw
        : action === "reset"
          ? RotateCcw
          : Server;
    return (
      <button
        key={action}
        type="button"
        onClick={() => void onControlService(serviceName, action)}
        disabled={loading || disabled}
        className={`theme-secondary-btn px-3 py-2 text-sm ${action === "stop" || action === "reset" ? "channel-service-stop-button" : ""} ${className}`}
      >
        {loading
          ? <Loader2 className="h-4 w-4 animate-spin" />
          : <ActionIcon className={`h-4 w-4 ${action === "stop" ? "fill-current" : ""}`} />}
        {actionLabel(action)}
      </button>
    );
  });
}

export interface CommunicationSetupPageProps {
  lang: UiLanguage;
  t: Translate;
  serviceActionMessage: ServiceActionNotice | null;
  serviceActionLoading: Record<string, boolean>;
  wechatStatusLoading: boolean;
  wechatStepStatus: SetupStepStatus;
  wechatStatusSummary: string;
  wechatQrStarting: boolean;
  wechatLoginStatus: WechatLoginStatus | null;
  wechatQrPreviewRequested: boolean;
  wechatLoginError: string | null;
  wechatConfigError: string | null;
  wechatConfigEnabled: boolean;
  wechatConfigured: boolean;
  wechatConfigSaving: boolean;
  wechatServiceHealthy: boolean;
  whatsappWebQrRequested: boolean;
  whatsappWebLoginLoading: boolean;
  whatsappWebLoginStatus: WhatsappWebLoginStatus | null;
  whatsappWebBridgeReachable: boolean;
  whatsappWebLoginError: string | null;
  whatsappWebLogoutLoading: boolean;
  whatsappWebServiceHealthy: boolean;
  telegramStatusLoading: boolean;
  telegramStepStatus: SetupStepStatus;
  telegramStatusSummary: string;
  primaryTelegramBot: TelegramBotConfigItem;
  telegramBotTokenConfigured: boolean;
  telegramConfigError: string | null;
  telegramConfigSaveMessage: string | null;
  telegramConfigSaving: boolean;
  telegramConfigLoading: boolean;
  hasUnsavedTelegramConfigChanges: boolean;
  telegramServiceHealthy: boolean;
  feishuSetup: AgentAppSetupState;
  larkSetup: AgentAppSetupState;
  isAdminIdentity: boolean;
  onControlService: (serviceName: ServiceName, action: ServiceAction) => unknown | Promise<unknown>;
  onEnableWechat: () => unknown | Promise<unknown>;
  onStartWechatQrLogin: (force?: boolean) => unknown | Promise<unknown>;
  onShowWhatsappWebQr: () => unknown | Promise<unknown>;
  onRefreshWhatsappWebLogin: () => unknown | Promise<unknown>;
  onLogoutWhatsappWeb: () => unknown | Promise<unknown>;
  onTelegramBotTokenChange: (value: string) => void;
  onSaveTelegramConfig: () => unknown | Promise<unknown>;
}

function setupStatusClass(loading: boolean, status: SetupStepStatus): string {
  if (loading) return "setup-status";
  if (status === "done") return "setup-status setup-status-done";
  if (status === "attention") return "setup-status setup-status-attention";
  return "setup-status setup-status-todo";
}

function setupStatusLabel(t: Translate, loading: boolean, status: SetupStepStatus, attentionZh = "还差一步"): string {
  if (loading) return t("载入中", "Loading");
  if (status === "done") return t("已可用", "Ready");
  if (status === "attention") return t(attentionZh, "In progress");
  return t("还没开始", "Not started");
}

interface AgentAppSetupCardProps {
  lang: UiLanguage;
  t: Translate;
  platform: "feishu" | "lark";
  className?: string;
  setup: AgentAppSetupState;
  isAdminIdentity: boolean;
  serviceActionLoading: Record<string, boolean>;
  onControlService: CommunicationSetupPageProps["onControlService"];
}

function AgentAppSetupCard({
  lang,
  t,
  platform,
  className = "",
  setup,
  isAdminIdentity,
  serviceActionLoading,
  onControlService,
}: AgentAppSetupCardProps) {
  const isLark = platform === "lark";
  const title = isLark ? "Lark" : t("飞书", "Feishu");
  const serviceName: ServiceName = isLark ? "larkd" : "feishud";
  const logName = isLark ? "larkd.log" : "feishud.log";

  return (
    <div className={`setup-channel-card channel-setup-card flex self-start flex-col ${className}`}>
      <div className="flex items-start justify-between gap-3">
        <div>
          <h4 className="text-lg font-semibold text-white">{title}</h4>
          <p className="mt-2 text-sm leading-6 text-white/65">
            {isLark
              ? t(
                  "使用 Lark 官方一键创建应用：扫码创建或选择应用，再把绑定码发给机器人即可。",
                  "Use Lark's official one-click app setup: scan to create or select an app, then send the bind code to the bot.",
                )
              : t(
                  "开始后生成二维码，扫码打开机器人，再发送绑定码完成绑定。",
                  "Generate a QR code, scan to open the bot, then send the bind code to finish binding.",
                )}
          </p>
        </div>
        <span className={setupStatusClass(setup.statusLoading, setup.stepStatus)}>
          {setup.statusLoading ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : null}
          {setupStatusLabel(t, setup.statusLoading, setup.stepStatus, "进行中")}
        </span>
      </div>

      <p className="mt-3 text-sm leading-6 text-white/65">{setup.statusSummary}</p>
      {setup.configError ? (
        <p className="mt-3 rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">{setup.configError}</p>
      ) : null}
      <p className="mt-2 text-xs leading-6 text-white/55">
        {lang === "zh" ? setup.setupGuidance.zhHint : setup.setupGuidance.enHint}
      </p>

      {!setup.currentKeyBound && setup.bindQrDataUrl ? (
        <div className="mt-3 rounded-2xl border border-white/10 bg-black/18 p-3">
          <p className="text-sm font-medium text-white/92">
            {lang === "zh" ? setup.bindStatusCopy.zhLabel : setup.bindStatusCopy.enLabel}
          </p>
          <p className="mt-1 text-xs leading-5 text-white/58">
            {lang === "zh" ? setup.bindStatusCopy.zhDescription : setup.bindStatusCopy.enDescription}
          </p>
          <div className="mt-3 flex items-center justify-center rounded-2xl border border-dashed border-white/12 bg-white/4 p-3">
            <div className="rounded-2xl border border-white/12 bg-white p-3 shadow-[0_18px_50px_rgba(6,10,18,0.2)]">
              <img src={setup.bindQrDataUrl} alt={`${title} QR`} className="h-40 w-40" />
            </div>
          </div>
          {setup.bindSession && !isFeishuBindTerminalStatus(setup.bindSession.status) ? (
            <div className="mt-3 rounded-xl border border-sky-400/20 bg-sky-500/10 p-3">
              <p className="text-[11px] font-medium uppercase tracking-[0.2em] text-sky-100/70">
                {t("绑定码", "Bind code")}
              </p>
              <p className="mt-2 break-all rounded-lg bg-black/25 px-3 py-2 font-mono text-xs text-sky-50">
                {setup.bindSession.bind_token}
              </p>
              <p className="mt-2 text-xs leading-5 text-sky-100/80">
                {t(
                  `扫码完成 ${title} 应用接入后，把这串绑定码原样发给机器人；页面会自动显示绑定成功。`,
                  `After scanning to finish ${title} app setup, send this bind code to the bot exactly as shown. This page will update automatically.`,
                )}
              </p>
            </div>
          ) : null}
          {setup.bindSession && !setup.bindSession.entry_url ? (
            <p className="mt-3 rounded-xl border border-amber-400/20 bg-amber-500/10 p-3 text-xs leading-5 text-amber-100/85">
              {t(
                `暂时没有拿到可用二维码。稍等后重试；仍失败时请到日志页查看 ${logName}。`,
                `No usable QR code was returned yet. Try again shortly; if it still fails, check ${logName} on the Logs page.`,
              )}
            </p>
          ) : null}
        </div>
      ) : null}

      {setup.bindError ? (
        <p className="mt-3 rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">{setup.bindError}</p>
      ) : null}

      <div className="channel-setup-actions mt-auto flex flex-wrap gap-2 pt-4">
        <button
          type="button"
          onClick={() => void setup.onBeginBind()}
          disabled={setup.bindLoading || setup.resetLoading || !isAdminIdentity || !setup.setupGuidance.canStartBind}
          className="theme-accent-btn px-3 py-2 text-sm"
        >
          {setup.bindLoading ? <Loader2 className="h-4 w-4 animate-spin" /> : <QrCode className="h-4 w-4" />}
          {setup.bindSession ? t("刷新二维码", "Refresh QR") : t(`开始${title}接入`, `Start ${title} setup`)}
        </button>
        {setup.setupGuidance.canStartService || setup.serviceHealthy ? (
          <ChannelServiceControls
            t={t}
            serviceName={serviceName}
            serviceLabelZh={isLark ? "Lark" : "飞书"}
            serviceLabelEn={isLark ? "Lark" : "Feishu"}
            healthy={setup.serviceHealthy}
            loading={Boolean(serviceActionLoading[serviceName])}
            disabled={!setup.canControlService}
            allowReset={false}
            onControlService={onControlService}
          />
        ) : null}
        <button
          type="button"
          onClick={() => void setup.onResetSetup()}
          disabled={setup.resetLoading || setup.bindLoading || !isAdminIdentity}
          className="theme-secondary-btn px-3 py-2 text-sm"
        >
          {setup.resetLoading ? <Loader2 className="h-4 w-4 animate-spin" /> : <RefreshCw className="h-4 w-4" />}
          {t(`重置${title}`, `Reset ${title}`)}
        </button>
      </div>
    </div>
  );
}

export function CommunicationSetupPage({
  lang,
  t,
  serviceActionMessage,
  serviceActionLoading,
  wechatStatusLoading,
  wechatStepStatus,
  wechatStatusSummary,
  wechatQrStarting,
  wechatLoginStatus,
  wechatQrPreviewRequested,
  wechatLoginError,
  wechatConfigError,
  wechatConfigEnabled,
  wechatConfigured,
  wechatConfigSaving,
  wechatServiceHealthy,
  whatsappWebQrRequested,
  whatsappWebLoginLoading,
  whatsappWebLoginStatus,
  whatsappWebBridgeReachable,
  whatsappWebLoginError,
  whatsappWebLogoutLoading,
  whatsappWebServiceHealthy,
  telegramStatusLoading,
  telegramStepStatus,
  telegramStatusSummary,
  primaryTelegramBot,
  telegramBotTokenConfigured,
  telegramConfigError,
  telegramConfigSaveMessage,
  telegramConfigSaving,
  telegramConfigLoading,
  hasUnsavedTelegramConfigChanges,
  telegramServiceHealthy,
  feishuSetup,
  larkSetup,
  isAdminIdentity,
  onControlService,
  onEnableWechat,
  onStartWechatQrLogin,
  onShowWhatsappWebQr,
  onRefreshWhatsappWebLogin,
  onLogoutWhatsappWeb,
  onTelegramBotTokenChange,
  onSaveTelegramConfig,
}: CommunicationSetupPageProps) {
  const whatsappWebConnected = whatsappWebServiceHealthy && whatsappWebLoginStatus?.connected === true;
  const whatsappWebStepStatus: SetupStepStatus = whatsappWebConnected
    ? "done"
    : whatsappWebServiceHealthy && (whatsappWebBridgeReachable || whatsappWebLoginStatus?.qr_ready)
      ? "attention"
      : "todo";
  const whatsappWebStatusSummary = whatsappWebConnected
    ? t("WhatsApp 已登录，可以接收和回复消息。", "WhatsApp is signed in and ready to receive and reply to messages.")
    : !whatsappWebServiceHealthy
      ? t("服务尚未启动。启动后，这里会显示可用手机扫描的二维码。", "The service is not running. Start it to show a QR code for your phone.")
      : whatsappWebLoginStatus?.qr_ready
        ? t("二维码已生成，请用手机 WhatsApp 扫描。", "The QR code is ready. Scan it with WhatsApp on your phone.")
        : whatsappWebBridgeReachable
          ? t("服务已启动，正在等待二维码。", "The service is running and waiting for a QR code.")
          : t("服务正在连接 WhatsApp，请稍候。", "The service is connecting to WhatsApp. Please wait.");

  return (
    <div className="space-y-5">
      {serviceActionMessage ? (
        <p
          className={
            serviceActionMessage.tone === "error"
              ? "rounded-2xl border border-red-500/30 bg-red-500/10 px-4 py-3 text-sm text-red-100"
              : "rounded-2xl border border-emerald-500/30 bg-emerald-500/10 px-4 py-3 text-sm text-emerald-100"
          }
        >
          {serviceActionMessage.text}
        </p>
      ) : null}

      <section className="theme-panel-soft channel-setup-hero p-5">
        <div className="flex flex-col gap-4 xl:flex-row xl:items-start xl:justify-between">
          <div className="max-w-2xl">
            <p className="theme-kicker text-[10px] uppercase tracking-[0.35em]">{t("通信接入", "Communication setup")}</p>
            <h3 className="mt-2 text-xl font-semibold tracking-tight">
              {t("微信、WhatsApp、Telegram、飞书和 Lark 都可以在这里接入。", "WeChat, WhatsApp, Telegram, Feishu, and Lark can all be connected here.")}
            </h3>
            <p className="mt-3 text-sm leading-7 text-white/70">
              {t(
                "按你要使用的通信方式完成配置即可。微信和 WhatsApp Web 支持扫码登录，Telegram 使用 Bot Token，飞书和 Lark 使用官方扫码创建应用并发送绑定码。",
                "Configure only the communication method you plan to use. WeChat and WhatsApp Web support QR sign-in, Telegram uses a bot token, and Feishu/Lark use official QR app setup followed by a bind code.",
              )}
            </p>
            <p className="mt-2 text-xs leading-6 text-white/55">
              {t(
                "通信端的默认语言只在无法识别用户或会话语言时使用，不会强制所有用户使用同一种语言。",
                "A channel's default language is used only when the user or conversation language cannot be identified; it does not force one language for everyone.",
              )}
            </p>
          </div>
        </div>

        <div className="mt-5 grid items-start gap-3 md:grid-cols-2 xl:grid-cols-3">
          <div className="setup-channel-card channel-setup-card order-1 flex self-start flex-col">
            <div className="flex items-start justify-between gap-3">
              <div>
                <h4 className="text-lg font-semibold text-white">{t("微信", "WeChat")}</h4>
                <p className="mt-2 text-sm leading-7 text-white/65">
                  {t(
                    "可以直接在当前卡片里完成设置、启动服务和扫码登录。",
                    "Complete configuration, start the service, and sign in with a QR code directly in this card.",
                  )}
                </p>
              </div>
              <span className={setupStatusClass(wechatStatusLoading, wechatStepStatus)}>
                {wechatStatusLoading ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : null}
                {setupStatusLabel(t, wechatStatusLoading, wechatStepStatus)}
              </span>
            </div>

            <p className="mt-4 text-sm leading-7 text-white/65">{wechatStatusSummary}</p>

            <div className="mt-4 flex flex-1 flex-col gap-4 border-t border-white/10 pt-4">
              {wechatQrStarting || wechatLoginStatus?.qr_status === "generating" || (wechatQrPreviewRequested && wechatLoginStatus?.qrcode_url) ? (
                <div className="wechat-login-visual space-y-3">
                  {wechatQrStarting || wechatLoginStatus?.qr_status === "generating" ? (
                    <div className="wechat-login-stage flex min-h-[20rem] items-center justify-center rounded-[24px] border border-dashed border-sky-500/25 bg-sky-500/6 p-5">
                      <div className="flex flex-col items-center gap-3 text-center">
                        <Loader2 className="h-8 w-8 animate-spin text-sky-200" />
                        <p className="text-sm font-medium text-sky-100">{t("正在生成二维码", "Generating QR code")}</p>
                        <p className="max-w-sm text-xs leading-6 text-sky-100/70">
                          {t("生成完成后，这里会自动切换成可扫码的二维码。", "This panel will switch to a scannable QR code automatically once generation finishes.")}
                        </p>
                      </div>
                    </div>
                  ) : wechatQrPreviewRequested && wechatLoginStatus?.qrcode_url ? (
                    <div className="space-y-3">
                      <div className="inline-block rounded-[24px] border border-white/12 bg-white p-4 shadow-[0_24px_70px_rgba(6,10,18,0.22)]">
                        <img src={wechatLoginStatus.qrcode_url} alt="WeChat QR" className="wechat-login-qr-image h-72 w-72" />
                      </div>
                      <p className="text-xs text-white/52">
                        {t("二维码有效期较短，过期后点击“刷新二维码”即可。", "The QR code expires quickly. Click Refresh QR if it expires.")}
                      </p>
                    </div>
                  ) : null}
                </div>
              ) : null}

              <div className="flex flex-1 flex-col gap-4">
                {wechatLoginStatus?.connected ? (
                  <div className="rounded-xl border border-emerald-500/20 bg-emerald-500/8 px-3 py-2 text-sm text-emerald-100/85">
                    {t("微信已登录，并已自动绑定到当前用户。", "WeChat is signed in and automatically bound to the current user.")}
                  </div>
                ) : wechatLoginStatus?.provider_connected && !wechatLoginStatus?.current_user_bound ? (
                  <div className="rounded-xl border border-amber-400/25 bg-amber-400/8 px-3 py-2 text-sm text-amber-100/85">
                    {t("现有微信登录不属于当前用户。请重新生成二维码并扫码，完成后会自动绑定。", "The current WeChat sign-in does not belong to this user. Generate and scan a new QR code to bind automatically.")}
                  </div>
                ) : null}

                {wechatLoginStatus?.last_error ? (
                  <p className="rounded-lg border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-sm text-amber-200">
                    {formatUiError(
                      wechatLoginStatus.last_error,
                      t,
                      "微信连接遇到问题，请刷新二维码或重启通信端。",
                      "The WeChat connection encountered a problem. Refresh the QR code or restart the channel.",
                    )}
                  </p>
                ) : null}
                {wechatLoginError ? (
                  <p className="rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">
                    {wechatLoginError}
                  </p>
                ) : null}
                {wechatConfigError ? (
                  <p className="rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">
                    {wechatConfigError}
                  </p>
                ) : null}

                <div className="channel-setup-actions mt-auto flex flex-wrap gap-2">
                  {wechatConfigEnabled || wechatConfigured ? (
                    <ChannelServiceControls
                      t={t}
                      serviceName="wechatd"
                      serviceLabelZh="微信"
                      serviceLabelEn="WeChat"
                      healthy={wechatServiceHealthy}
                      loading={Boolean(serviceActionLoading.wechatd)}
                      disabled={!isAdminIdentity}
                      className="px-4 py-2.5"
                      onControlService={onControlService}
                    />
                  ) : (
                    <button
                      type="button"
                      onClick={() => void onEnableWechat()}
                      disabled={wechatConfigSaving || Boolean(serviceActionLoading.wechatd) || !isAdminIdentity}
                      className="theme-accent-btn px-4 py-2.5 text-sm"
                    >
                      {wechatConfigSaving || serviceActionLoading.wechatd
                        ? <Loader2 className="h-4 w-4 animate-spin" />
                        : <Power className="h-4 w-4" />}
                      {t("启用微信并开始绑定", "Enable WeChat and start setup")}
                    </button>
                  )}
                  <button
                    type="button"
                    onClick={() => void onStartWechatQrLogin(true)}
                    disabled={Boolean(serviceActionLoading.wechatd) || wechatQrStarting || !wechatServiceHealthy}
                    className="theme-accent-btn px-4 py-2.5 text-sm"
                  >
                    {wechatQrStarting ? <Loader2 className="h-4 w-4 animate-spin" /> : <RefreshCw className="h-4 w-4" />}
                    {wechatLoginStatus?.connected
                      ? t("重新生成二维码", "Regenerate QR")
                      : wechatQrPreviewRequested && wechatLoginStatus?.qrcode_url
                        ? t("刷新二维码", "Refresh QR")
                        : t("生成二维码", "Generate QR")}
                  </button>
                </div>
              </div>
            </div>
          </div>

          <div className="setup-channel-card channel-setup-card order-3 flex self-start flex-col">
            <div className="flex items-start justify-between gap-3">
              <div>
                <div className="flex flex-wrap items-center gap-2">
                  <h4 className="text-lg font-semibold text-white">WhatsApp Web</h4>
                  <span className="rounded-full border border-amber-400/30 bg-amber-400/10 px-2.5 py-1 text-[11px] font-semibold tracking-wide text-amber-100">
                    {t("实验性连接", "Experimental")}
                  </span>
                </div>
                <p className="mt-2 text-sm leading-7 text-white/65">
                  {t(
                    "这是通过 Baileys 兼容桥实现的扫码连接，不是 Meta 官方 Bot API。客户端变化可能影响稳定性和账号可用性。",
                    "This QR connection uses a Baileys-compatible bridge; it is not Meta's official Bot API. Client changes may affect stability and account availability.",
                  )}
                </p>
              </div>
              <span className={setupStatusClass(whatsappWebLoginLoading, whatsappWebStepStatus)}>
                {whatsappWebLoginLoading ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : null}
                {setupStatusLabel(t, whatsappWebLoginLoading, whatsappWebStepStatus, "等待扫码")}
              </span>
            </div>

            <p className="mt-4 text-sm leading-7 text-white/65">{whatsappWebStatusSummary}</p>

            <div className="mt-4 space-y-2 rounded-xl border border-amber-400/20 bg-amber-400/8 px-3 py-3 text-xs leading-6 text-amber-50/80">
              <p>
                {t(
                  "二维码、登录、重连和移除设备只属于连接状态，不会写入 Agent 对话历史或记忆。",
                  "QR, sign-in, reconnect, and device removal are connection state only and are not written to Agent chat history or memory.",
                )}
              </p>
              <p>
                {whatsappWebLoginStatus?.proactive_send_enabled
                  ? t(
                      "主动发送已显式开启。实验适配器可能有账号和稳定性风险，请谨慎使用。",
                      "Proactive sending is explicitly enabled. Use it carefully because this experimental adapter may carry account and stability risks.",
                    )
                  : t(
                      "主动发送默认关闭；用户发来消息后的任务结果回复不受影响。",
                      "Proactive sending is off by default; task replies following an inbound user message are unaffected.",
                    )}
              </p>
              {whatsappWebLoginStatus?.local_safety_limits ? (
                <p>
                  {t("本地保护上限（不是 WhatsApp 官方上限）", "Local safety limits (not official WhatsApp limits)")}：
                  {t("图片", "image")} {formatBytes(whatsappWebLoginStatus.local_safety_limits.image_bytes)} · {t("视频", "video")} {formatBytes(whatsappWebLoginStatus.local_safety_limits.video_bytes)} · {t("音频", "audio")} {formatBytes(whatsappWebLoginStatus.local_safety_limits.audio_bytes)} · {t("文件", "file")} {formatBytes(whatsappWebLoginStatus.local_safety_limits.file_bytes)}
                </p>
              ) : null}
            </div>

            <div className="mt-4 flex flex-1 flex-col gap-4 border-t border-white/10 pt-4">
              {whatsappWebConnected ? (
                <div className="rounded-xl border border-emerald-500/20 bg-emerald-500/8 px-3 py-3 text-sm leading-6 text-emerald-100/85">
                  {t(
                    "手机端已确认登录。保持服务运行即可收发消息；首次使用的用户请私聊发送 /key <登录密钥>，不要在群聊中发送密钥。",
                    "Sign-in was confirmed on your phone. Keep the service running to send and receive messages. First-time users should privately send /key <login key>; never post a key in a group.",
                  )}
                </div>
              ) : whatsappWebServiceHealthy && whatsappWebQrRequested && whatsappWebLoginStatus?.qr_data_url ? (
                <div className="space-y-3">
                  <div className="inline-block rounded-[24px] border border-white/12 bg-white p-4 shadow-[0_24px_70px_rgba(6,10,18,0.22)]">
                    <img
                      src={whatsappWebLoginStatus.qr_data_url}
                      alt="WhatsApp Web QR"
                      className="h-72 w-72 max-w-full"
                    />
                  </div>
                  <p className="text-xs leading-6 text-white/52">
                    {t(
                      "打开手机 WhatsApp 的“已关联设备”，选择“关联设备”后扫描。二维码过期时点击“刷新状态”。",
                      "Open Linked devices in WhatsApp on your phone, choose Link a device, and scan. If the QR expires, select Refresh status.",
                    )}
                  </p>
                </div>
              ) : whatsappWebQrRequested && whatsappWebServiceHealthy ? (
                <div className="flex min-h-64 items-center justify-center rounded-[24px] border border-dashed border-sky-500/25 bg-sky-500/6 p-5">
                  <div className="flex flex-col items-center gap-3 text-center">
                    <Loader2 className="h-8 w-8 animate-spin text-sky-200" />
                    <p className="text-sm font-medium text-sky-100">{t("正在生成二维码", "Generating QR code")}</p>
                    <p className="max-w-sm text-xs leading-6 text-sky-100/70">
                      {t("二维码生成后会自动显示在这里。", "The QR code will appear here automatically when it is ready.")}
                    </p>
                  </div>
                </div>
              ) : (
                <div className="flex min-h-40 items-center justify-center rounded-[24px] border border-dashed border-white/12 bg-white/4 p-5 text-center">
                  <div className="flex max-w-sm flex-col items-center gap-3">
                    <QrCode className="h-9 w-9 text-white/45" />
                    <p className="text-sm leading-6 text-white/58">
                      {t(
                        "先启动 WhatsApp Web 服务，再点击“显示二维码”。",
                        "Start the WhatsApp Web service, then select Show QR code.",
                      )}
                    </p>
                  </div>
                </div>
              )}

              {whatsappWebLoginErrorCopy(t, whatsappWebLoginStatus?.last_error_code) ? (
                <p className="rounded-lg border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-sm text-amber-200">
                  {whatsappWebLoginErrorCopy(t, whatsappWebLoginStatus?.last_error_code)}
                  {whatsappWebLoginStatus?.last_diagnostic_id ? (
                    <span className="mt-1 block font-mono text-xs text-amber-100/65">
                      {t("诊断编号", "Diagnostic ID")}: {whatsappWebLoginStatus.last_diagnostic_id}
                    </span>
                  ) : null}
                </p>
              ) : null}
              {whatsappWebLoginError ? (
                <p className="rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">
                  {whatsappWebLoginError}
                </p>
              ) : null}

              <div className="channel-setup-actions mt-auto flex flex-wrap gap-2">
                <ChannelServiceControls
                  t={t}
                  serviceName="whatsapp_webd"
                  serviceLabelZh=" WhatsApp Web "
                  serviceLabelEn="WhatsApp Web"
                  healthy={whatsappWebServiceHealthy}
                  loading={Boolean(serviceActionLoading.whatsapp_webd)}
                  disabled={!isAdminIdentity}
                  className="px-4 py-2.5"
                  onControlService={onControlService}
                />
                {!whatsappWebConnected ? (
                  <button
                    type="button"
                    onClick={() => void (whatsappWebQrRequested ? onRefreshWhatsappWebLogin() : onShowWhatsappWebQr())}
                    disabled={whatsappWebLoginLoading || !whatsappWebServiceHealthy}
                    className="theme-accent-btn px-4 py-2.5 text-sm"
                  >
                    {whatsappWebLoginLoading
                      ? <Loader2 className="h-4 w-4 animate-spin" />
                      : whatsappWebQrRequested
                        ? <RefreshCw className="h-4 w-4" />
                        : <QrCode className="h-4 w-4" />}
                    {whatsappWebQrRequested ? t("刷新状态", "Refresh status") : t("显示二维码", "Show QR code")}
                  </button>
                ) : (
                  <button
                    type="button"
                    onClick={() => void onLogoutWhatsappWeb()}
                    disabled={whatsappWebLogoutLoading}
                    className="theme-secondary-btn px-4 py-2.5 text-sm"
                  >
                    {whatsappWebLogoutLoading ? <Loader2 className="h-4 w-4 animate-spin" /> : <LogOut className="h-4 w-4" />}
                    {t("退出登录", "Sign out")}
                  </button>
                )}
              </div>
            </div>
          </div>

          <div className="setup-channel-card channel-setup-card order-2 flex self-start flex-col">
            <div className="flex items-start justify-between gap-3">
              <div>
                <h4 className="text-lg font-semibold text-white">Telegram</h4>
                <p className="mt-2 text-sm leading-7 text-white/65">
                  {t(
                    "如果你要用 Telegram，只需要填好 Bot Token，然后保存并启动服务。",
                    "If you plan to use Telegram, just enter the bot token, save it, and start the service.",
                  )}
                </p>
              </div>
              <span className={setupStatusClass(telegramStatusLoading, telegramStepStatus)}>
                {telegramStatusLoading ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : null}
                {setupStatusLabel(t, telegramStatusLoading, telegramStepStatus)}
              </span>
            </div>

            <p className="mt-4 text-sm leading-7 text-white/65">{telegramStatusSummary}</p>

            <div className="channel-setup-form mt-4 grid gap-3">
              <label className="block space-y-2">
                <span className="text-xs uppercase tracking-widest text-white/50">{t("Bot Token", "Bot Token")}</span>
                <input
                  className="theme-input"
                  value={primaryTelegramBot.bot_token}
                  onChange={(event) => onTelegramBotTokenChange(event.target.value)}
                />
                <p className="text-xs text-white/45">
                  {t("这里只填 Bot Token 就够了。更复杂的设置以后再说。", "Only the Bot Token is needed here. More advanced settings can wait until later.")}
                </p>
                {primaryTelegramBot.bot_token_masked ? (
                  <p className="rounded-lg border border-white/10 bg-black/20 px-3 py-2 text-xs text-white/65">
                    {t("当前正在使用：", "Currently in use: ")}
                    <span className="ml-1 font-mono text-white/88">{primaryTelegramBot.bot_token_masked}</span>
                  </p>
                ) : null}
                <p className="text-xs text-white/35">
                  {telegramBotTokenConfigured
                    ? t("出于安全考虑，当前已保存的 Bot Token 不会回显到输入框。", "For safety, the currently saved bot token is not echoed back into the input.")
                    : t("这里不会回显已保存的 Token。需要更新时，直接输入新的 Bot Token 即可。", "Saved tokens are not echoed here. To update it, just enter a new bot token.")}
                </p>
              </label>
            </div>

            {telegramConfigError ? (
              <p className="mt-3 rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">{telegramConfigError}</p>
            ) : null}
            {telegramConfigSaveMessage ? (
              <p className="mt-3 rounded-lg border border-emerald-500/30 bg-emerald-500/10 px-3 py-2 text-sm text-emerald-200">{telegramConfigSaveMessage}</p>
            ) : null}
            <div className="channel-setup-actions mt-auto flex flex-wrap gap-2 pt-5">
              <button
                type="button"
                onClick={() => void onSaveTelegramConfig()}
                disabled={telegramConfigSaving || telegramConfigLoading || !hasUnsavedTelegramConfigChanges}
                className="theme-accent-btn theme-key-create-btn px-3 py-2 text-sm"
              >
                {telegramConfigSaving ? <Loader2 className="h-4 w-4 animate-spin" /> : <Database className="h-4 w-4" />}
                {t("保存 Telegram", "Save Telegram")}
              </button>
              <ChannelServiceControls
                t={t}
                serviceName="telegramd"
                serviceLabelZh=" Telegram "
                serviceLabelEn="Telegram"
                healthy={telegramServiceHealthy}
                loading={Boolean(serviceActionLoading.telegramd)}
                disabled={!telegramBotTokenConfigured || !isAdminIdentity}
                className="theme-key-create-btn"
                onControlService={onControlService}
              />
            </div>
          </div>

          <AgentAppSetupCard
            lang={lang}
            t={t}
            platform="feishu"
            className="order-4"
            setup={feishuSetup}
            isAdminIdentity={isAdminIdentity}
            serviceActionLoading={serviceActionLoading}
            onControlService={onControlService}
          />
          <AgentAppSetupCard
            lang={lang}
            t={t}
            platform="lark"
            className="order-5"
            setup={larkSetup}
            isAdminIdentity={isAdminIdentity}
            serviceActionLoading={serviceActionLoading}
            onControlService={onControlService}
          />
        </div>
      </section>
    </div>
  );
}
