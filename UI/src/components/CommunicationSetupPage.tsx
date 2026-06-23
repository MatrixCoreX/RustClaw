import { Database, Loader2, RefreshCw, Server } from "lucide-react";

import type {
  ServiceActionNotice,
  TelegramBotConfigItem,
  WechatLoginStatus,
} from "../types/api";
import {
  isFeishuBindTerminalStatus,
  type FeishuBindSessionResponse,
  type FeishuBindStatusCopy,
  type FeishuSetupGuidance,
} from "../lib/feishu-bind";

type UiLanguage = "zh" | "en";
type Translate = (zh: string, en: string) => string;
type SetupStepStatus = "done" | "attention" | "todo";
type ServiceName = "telegramd" | "whatsappd" | "whatsapp_webd" | "wechatd" | "feishud" | "larkd";
type ServiceAction = "start" | "stop" | "restart";

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
  wechatConfigEnabled: boolean;
  wechatServiceHealthy: boolean;
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
  feishuStatusLoading: boolean;
  feishuStepStatus: SetupStepStatus;
  feishuStatusSummary: string;
  feishuConfigError: string | null;
  feishuSetupGuidance: FeishuSetupGuidance;
  feishuCurrentKeyBound: boolean;
  feishuBindQrDataUrl: string | null;
  feishuBindStatusCopy: FeishuBindStatusCopy;
  feishuBindSession: FeishuBindSessionResponse | null;
  feishuBindError: string | null;
  feishuBindLoading: boolean;
  feishuResetLoading: boolean;
  isAdminIdentity: boolean;
  feishuServiceHealthy: boolean;
  canControlFeishuService: boolean;
  onControlService: (serviceName: ServiceName, action: ServiceAction) => unknown | Promise<unknown>;
  onStartWechatQrLogin: (force?: boolean) => unknown | Promise<unknown>;
  onTelegramBotTokenChange: (value: string) => void;
  onSaveTelegramConfig: () => unknown | Promise<unknown>;
  onBeginFeishuBind: () => unknown | Promise<unknown>;
  onResetFeishuSetup: () => unknown | Promise<unknown>;
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
  wechatConfigEnabled,
  wechatServiceHealthy,
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
  feishuStatusLoading,
  feishuStepStatus,
  feishuStatusSummary,
  feishuConfigError,
  feishuSetupGuidance,
  feishuCurrentKeyBound,
  feishuBindQrDataUrl,
  feishuBindStatusCopy,
  feishuBindSession,
  feishuBindError,
  feishuBindLoading,
  feishuResetLoading,
  isAdminIdentity,
  feishuServiceHealthy,
  canControlFeishuService,
  onControlService,
  onStartWechatQrLogin,
  onTelegramBotTokenChange,
  onSaveTelegramConfig,
  onBeginFeishuBind,
  onResetFeishuSetup,
}: CommunicationSetupPageProps) {
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
              {t("微信、Telegram 和飞书都可以在这里接入。", "WeChat, Telegram, and Feishu can all be connected here.")}
            </h3>
            <p className="mt-3 text-sm leading-7 text-white/70">
              {t(
                "按你要使用的通信方式完成配置即可。微信支持扫码登录，Telegram 支持 Bot Token 接入，飞书支持扫码打开机器人后发送绑定码完成绑定。",
                "Configure only the communication method you plan to use. WeChat supports QR sign-in, Telegram uses a bot token, and Feishu lets you scan to open the bot and then send a bind code to finish binding.",
              )}
            </p>
          </div>
        </div>

        <div className="mt-5 grid items-start gap-4 xl:grid-cols-3">
          <div className="setup-channel-card channel-setup-card flex self-start flex-col">
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
                    {t("当前登录状态可继续使用；如果要更换登录，也可以重新生成二维码。", "The current login is active. If you want to switch accounts, you can also regenerate the QR code.")}
                  </div>
                ) : null}

                {wechatLoginStatus?.last_error ? (
                  <p className="rounded-lg border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-sm text-amber-200">
                    {wechatLoginStatus.last_error}
                  </p>
                ) : null}
                {wechatLoginError ? (
                  <p className="rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">
                    {wechatLoginError}
                  </p>
                ) : null}

                <div className="mt-auto flex flex-wrap gap-2">
                  <button
                    type="button"
                    onClick={() => void onControlService("wechatd", wechatServiceHealthy ? "restart" : "start")}
                    disabled={Boolean(serviceActionLoading.wechatd) || !wechatConfigEnabled}
                    className="theme-secondary-btn px-4 py-2.5 text-sm"
                  >
                    {serviceActionLoading.wechatd ? <Loader2 className="h-4 w-4 animate-spin" /> : <RefreshCw className="h-4 w-4" />}
                    {wechatServiceHealthy ? t("重启微信服务", "Restart the WeChat service") : t("启动微信服务", "Start the WeChat service")}
                  </button>
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

          <div className="setup-channel-card channel-setup-card flex self-start flex-col">
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
              <button
                type="button"
                onClick={() => void onControlService("telegramd", telegramServiceHealthy ? "restart" : "start")}
                disabled={Boolean(serviceActionLoading.telegramd) || !telegramBotTokenConfigured}
                className="theme-secondary-btn theme-key-create-btn px-3 py-2 text-sm"
              >
                {serviceActionLoading.telegramd ? <Loader2 className="h-4 w-4 animate-spin" /> : <Server className="h-4 w-4" />}
                {telegramServiceHealthy ? t("重启 Telegram 服务", "Restart the Telegram service") : t("启动 Telegram 服务", "Start the Telegram service")}
              </button>
            </div>
          </div>

          <div className="setup-channel-card channel-setup-card flex self-start flex-col">
            <div className="flex items-start justify-between gap-3">
              <div>
                <h4 className="text-lg font-semibold text-white">{t("飞书", "Feishu")}</h4>
                <p className="mt-2 text-sm leading-7 text-white/65">
                  {t(
                    "开始后会生成二维码，扫码打开机器人，再发送绑定码完成绑定。",
                    "Start to generate a QR code, then scan to open the bot and send the bind code to finish binding.",
                  )}
                </p>
              </div>
              <span className={setupStatusClass(feishuStatusLoading, feishuStepStatus)}>
                {feishuStatusLoading ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : null}
                {setupStatusLabel(t, feishuStatusLoading, feishuStepStatus, "进行中")}
              </span>
            </div>

            <p className="mt-4 text-sm leading-7 text-white/65">{feishuStatusSummary}</p>

            {feishuConfigError ? (
              <p className="mt-3 rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">{feishuConfigError}</p>
            ) : null}
            <p className="mt-3 text-sm text-white/55">
              {lang === "zh" ? feishuSetupGuidance.zhHint : feishuSetupGuidance.enHint}
            </p>

            {!feishuCurrentKeyBound && feishuBindQrDataUrl ? (
              <div className="mt-4 rounded-2xl border border-white/10 bg-black/18 p-4">
                <div className="flex items-start justify-between gap-3">
                  <div>
                    <p className="text-sm font-medium text-white/92">{lang === "zh" ? feishuBindStatusCopy.zhLabel : feishuBindStatusCopy.enLabel}</p>
                    <p className="mt-2 text-xs leading-6 text-white/58">
                      {lang === "zh" ? feishuBindStatusCopy.zhDescription : feishuBindStatusCopy.enDescription}
                    </p>
                  </div>
                </div>

                <div className="mt-4 flex min-h-52 items-center justify-center rounded-[24px] border border-dashed border-white/12 bg-white/4">
                  <div className="inline-block rounded-[24px] border border-white/12 bg-white p-4 shadow-[0_24px_70px_rgba(6,10,18,0.22)]">
                    <img src={feishuBindQrDataUrl} alt="Feishu QR" className="h-52 w-52" />
                  </div>
                </div>
                {feishuBindSession && !isFeishuBindTerminalStatus(feishuBindSession.status) ? (
                  <div className="mt-4 rounded-2xl border border-sky-400/20 bg-sky-500/10 p-4">
                    <p className="text-xs font-medium uppercase tracking-[0.22em] text-sky-100/70">
                      {t("绑定码", "Bind code")}
                    </p>
                    <p className="mt-3 break-all rounded-xl bg-black/25 px-3 py-3 font-mono text-sm text-sky-50">
                      {feishuBindSession.bind_token}
                    </p>
                    <p className="mt-3 text-xs leading-6 text-sky-100/80">
                      {t(
                        "1. 扫码打开 RustClaw 飞书机器人。2. 把这串绑定码原样发给机器人。3. 页面会自动刷新为绑定成功。",
                        "1. Scan to open the RustClaw Feishu bot. 2. Send this bind code to the bot exactly as shown. 3. The page will refresh when binding succeeds.",
                      )}
                    </p>
                  </div>
                ) : null}
                {feishuBindSession && !feishuBindSession.entry_url ? (
                  <div className="mt-4 rounded-xl border border-amber-400/20 bg-amber-500/10 p-3 text-xs leading-6 text-amber-100/85">
                    {t(
                      "这次飞书接入还没有拿到可用二维码。稍等 1 到 2 秒后重试；如果还是不行，再去日志页面看 feishud.log。",
                      "This Feishu setup did not get a usable QR code yet. Wait 1-2 seconds and try again. If it still fails, check feishud.log on the logs page.",
                    )}
                  </div>
                ) : null}
              </div>
            ) : null}

            {feishuBindError ? (
              <p className="mt-3 rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-200">{feishuBindError}</p>
            ) : null}

            <div className="channel-setup-actions mt-auto flex flex-wrap gap-2 pt-5">
              <button
                type="button"
                onClick={() => void onBeginFeishuBind()}
                disabled={feishuBindLoading || feishuResetLoading || !isAdminIdentity || !feishuSetupGuidance.canStartBind}
                className="theme-accent-btn px-3 py-2 text-sm"
              >
                {feishuBindLoading ? <Loader2 className="h-4 w-4 animate-spin" /> : <RefreshCw className="h-4 w-4" />}
                {feishuBindSession ? t("重新生成二维码", "Refresh QR") : t("开始飞书接入", "Start Feishu setup")}
              </button>
              {feishuSetupGuidance.canStartService || feishuServiceHealthy ? (
                <button
                  type="button"
                  onClick={() => void onControlService("feishud", feishuServiceHealthy ? "restart" : "start")}
                  disabled={Boolean(serviceActionLoading.feishud) || !canControlFeishuService}
                  className="theme-secondary-btn px-3 py-2 text-sm"
                >
                  {serviceActionLoading.feishud ? <Loader2 className="h-4 w-4 animate-spin" /> : <Server className="h-4 w-4" />}
                  {feishuServiceHealthy
                    ? t("重启飞书服务", "Restart Feishu service")
                    : t("启动飞书服务", "Start Feishu service")}
                </button>
              ) : null}
              <button
                type="button"
                onClick={() => void onResetFeishuSetup()}
                disabled={feishuResetLoading || feishuBindLoading || !isAdminIdentity}
                className="theme-secondary-btn px-3 py-2 text-sm"
              >
                {feishuResetLoading ? <Loader2 className="h-4 w-4 animate-spin" /> : <RefreshCw className="h-4 w-4" />}
                {t("重置飞书", "Reset Feishu")}
              </button>
            </div>
          </div>
        </div>
      </section>
    </div>
  );
}
