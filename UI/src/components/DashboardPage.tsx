import {
  AlertTriangle,
  BellRing,
  Cpu,
  Download,
  LayoutDashboard,
  Loader2,
  RefreshCw,
  X,
} from "lucide-react";

import type { DashboardOverviewItem, DashboardStepStatus } from "../lib/dashboard-home";
import type { WorkspaceUpdateNotice } from "../lib/workspace-update";
import type {
  ConsolePage,
  DashboardCommunicationRow,
  PiAppStatusResponse,
  WorkspaceUpdateMode,
  WorkspaceUpdateStatus,
} from "../types/api";

type Translate = (zh: string, en: string) => string;

export interface DashboardOnboardingStep {
  key: string;
  title: string;
  description: string;
  status: DashboardStepStatus;
  page: ConsolePage;
  cta: string;
}

export interface DashboardPageProps {
  t: Translate;
  onboardingSteps: DashboardOnboardingStep[];
  dashboardOverviewItems: DashboardOverviewItem[];
  isAdminIdentity: boolean;
  workspaceUpdateLoading: boolean;
  workspaceUpdateRunning: boolean;
  workspaceUpdateHasRemoteDiff: boolean;
  workspaceUpdateStatus: WorkspaceUpdateStatus | null;
  workspaceUpdateCanceling: boolean;
  workspaceUpdateMessage: string | null;
  workspaceUpdateRestarting: boolean;
  workspaceUpdateDisplayStatus: string | undefined;
  workspaceUpdateProgressPercent: number;
  workspaceUpdateProgressActive: boolean;
  workspaceUpdateProgressLabel: string;
  workspaceUpdateLogPreview: string;
  workspaceUpdateNotice: WorkspaceUpdateNotice | null;
  systemRestarting: boolean;
  systemRestartMessage: string | null;
  piAppStatus: PiAppStatusResponse | null;
  piAppRestarting: boolean;
  piAppRestartMessage: string | null;
  dashboardCommunicationRows: DashboardCommunicationRow[];
  queuePressureHigh: boolean;
  runningTooOld: boolean;
  isOnline: boolean;
  queueLength: number;
  runningOldestAgeLabel: string;
  onSetCurrentPage: (page: ConsolePage) => void;
  onFetchWorkspaceUpdateStatus: () => unknown | Promise<unknown>;
  onStartWorkspaceUpdate: (mode: WorkspaceUpdateMode) => unknown | Promise<unknown>;
  onCancelWorkspaceUpdate: () => unknown | Promise<unknown>;
  onRestartSystem: () => unknown | Promise<unknown>;
  onRestartPiApp: () => unknown | Promise<unknown>;
  workspaceUpdateStepLabel: (step?: string) => string;
  workspaceUpdateStatusLabel: (status?: string) => string;
  workspaceUpdateTimeLabel: (ts?: number | null) => string;
}

export function DashboardPage({
  t,
  onboardingSteps,
  dashboardOverviewItems,
  isAdminIdentity,
  workspaceUpdateLoading,
  workspaceUpdateRunning,
  workspaceUpdateHasRemoteDiff,
  workspaceUpdateStatus,
  workspaceUpdateCanceling,
  workspaceUpdateMessage,
  workspaceUpdateRestarting,
  workspaceUpdateDisplayStatus,
  workspaceUpdateProgressPercent,
  workspaceUpdateProgressActive,
  workspaceUpdateProgressLabel,
  workspaceUpdateLogPreview,
  workspaceUpdateNotice,
  systemRestarting,
  systemRestartMessage,
  piAppStatus,
  piAppRestarting,
  piAppRestartMessage,
  dashboardCommunicationRows,
  queuePressureHigh,
  runningTooOld,
  isOnline,
  queueLength,
  runningOldestAgeLabel,
  onSetCurrentPage,
  onFetchWorkspaceUpdateStatus,
  onStartWorkspaceUpdate,
  onCancelWorkspaceUpdate,
  onRestartSystem,
  onRestartPiApp,
  workspaceUpdateStepLabel,
  workspaceUpdateStatusLabel,
  workspaceUpdateTimeLabel,
}: DashboardPageProps) {
  return (
    <>
      <section className="theme-panel setup-hero p-5 sm:p-6">
        <div className="max-w-3xl">
          <p className="theme-kicker text-[10px] uppercase tracking-[0.35em]">{t("首次使用", "First run")}</p>
          <h3 className="mt-2 text-xl font-semibold tracking-tight sm:text-3xl">
            {t("开始使用 RustClaw", "Start using RustClaw")}
          </h3>
          <p className="mt-3 text-sm leading-7 text-white/70 sm:text-base">
            {t(
              "请先完成大模型配置和消息测试；如需通过微信使用 RustClaw，再继续完成微信接入。Telegram 仅在你需要时再补充配置。",
              "Please complete the model setup and a test message first. If you want to use RustClaw through WeChat, continue with the WeChat setup. Add Telegram later only if you need it.",
            )}
          </p>
        </div>

        <div className="mt-6 grid gap-3 xl:grid-cols-3">
          {onboardingSteps.map((step, index) => (
            <button
              key={step.key}
              type="button"
              onClick={() => onSetCurrentPage(step.page)}
              className="setup-step-card setup-step-card-compact text-left"
            >
              <span className="setup-step-index setup-step-index-floating">{index + 1}</span>
              {step.key !== "chat" ? (
                <span
                  className={
                    step.status === "done"
                      ? "setup-status setup-step-status setup-status-done"
                      : step.status === "attention"
                        ? "setup-status setup-step-status setup-status-attention"
                        : "setup-status setup-step-status setup-status-todo"
                  }
                >
                  {step.status === "done"
                    ? t("已完成", "Done")
                    : step.status === "attention"
                      ? t("待完成", "Needs attention")
                      : t("未开始", "Not started")}
                </span>
              ) : null}
              <div className="setup-step-card-body">
                <h4 className="text-base font-semibold text-white">{step.title}</h4>
                <p className="mt-2 text-sm leading-7 text-white/65">{step.description}</p>
              </div>
            </button>
          ))}
        </div>
      </section>

      <section className="theme-panel-soft rounded-[22px] border border-white/10 px-4 py-3 sm:px-5">
        <div className="grid gap-3 md:grid-cols-3">
          {dashboardOverviewItems.map((item, index) => (
            <div key={item.key} className={`py-2 ${index > 0 ? "md:border-l md:border-white/8 md:pl-5" : ""}`}>
              <p className="text-[11px] tracking-[0.16em] text-white/42">{item.label}</p>
              <p
                className={`mt-2 text-base font-semibold ${
                  item.tone === "good"
                    ? "text-emerald-200"
                    : item.tone === "warning"
                      ? "text-amber-200"
                      : "text-white/92"
                }`}
              >
                {item.value}
              </p>
            </div>
          ))}
        </div>
      </section>

      <section className="space-y-4">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div className="max-w-2xl">
            <p className="theme-kicker text-[10px] uppercase tracking-[0.28em]">
              {t("系统更新", "System Update")}
            </p>
            <h3 className="mt-2 text-base font-semibold text-white">
              {t("选择适合当前设备的更新方式", "Choose an update method for this device")}
            </h3>
            <p className="mt-2 text-sm leading-7 text-white/65">
              {t(
                "普通用户建议使用 Release 更新。只有需要源码改动或排障时，才在当前设备上拉取并编译。",
                "Release updates are recommended for most users. Pull and compile source on this device only for source changes or troubleshooting.",
              )}
            </p>
          </div>
          {isAdminIdentity ? (
            <button
              type="button"
              onClick={() => void onFetchWorkspaceUpdateStatus()}
              disabled={workspaceUpdateLoading || systemRestarting}
              className="theme-topbar-btn px-3 py-2 text-sm"
            >
              {workspaceUpdateLoading && !workspaceUpdateRunning ? (
                <Loader2 className="h-4 w-4 animate-spin" />
              ) : (
                <RefreshCw className="h-4 w-4" />
              )}
              {t("检查远端版本", "Check remote")}
            </button>
          ) : (
            <span className="rounded-full border border-white/10 bg-white/5 px-3 py-2 text-xs text-white/55">
              {t("仅管理员可更新", "Admin only")}
            </span>
          )}
        </div>

        {isAdminIdentity ? (
          <div className="grid gap-4 lg:grid-cols-2">
            <div className="rounded-lg border border-emerald-400/20 bg-emerald-400/[0.06] p-4 sm:p-5">
              <div className="flex items-start gap-3">
                <span className="rounded-lg bg-emerald-400/10 p-2 text-emerald-200">
                  <Download className="h-5 w-5" />
                </span>
                <div>
                  <div className="flex flex-wrap items-center gap-2">
                    <h4 className="text-sm font-semibold text-white">{t("Release 更新", "Release Update")}</h4>
                    <span className="rounded-full bg-emerald-400/10 px-2 py-0.5 text-[11px] text-emerald-200">
                      {t("推荐", "Recommended")}
                    </span>
                  </div>
                  <p className="mt-2 text-sm leading-6 text-white/65">
                    {t(
                      "下载适合当前系统和架构的预编译包，保留本地配置与数据后更新并重启。无需在本机编译。",
                      "Downloads the prebuilt package for this system and architecture, preserves local configuration and data, then updates and restarts without compiling locally.",
                    )}
                  </p>
                </div>
              </div>
              <button
                type="button"
                onClick={() => void onStartWorkspaceUpdate("release_deploy")}
                disabled={workspaceUpdateLoading || workspaceUpdateRunning || systemRestarting}
                className="theme-accent-btn mt-4"
              >
                {workspaceUpdateRunning && workspaceUpdateStatus?.mode === "release_deploy" ? (
                  <Loader2 className="h-4 w-4 animate-spin" />
                ) : (
                  <Download className="h-4 w-4" />
                )}
                {workspaceUpdateRunning && workspaceUpdateStatus?.mode === "release_deploy"
                  ? t("更新中", "Updating")
                  : t("更新", "Update")}
              </button>
            </div>

            <div className="rounded-lg border border-amber-400/25 bg-amber-400/[0.06] p-4 sm:p-5">
              <div className="flex items-start gap-3">
                <span className="rounded-lg bg-amber-400/10 p-2 text-amber-200">
                  <Cpu className="h-5 w-5" />
                </span>
                <div>
                  <h4 className="text-sm font-semibold text-white">
                    {t("拉取源码并编译", "Pull Source and Compile")}
                  </h4>
                  <p className="mt-2 text-sm leading-6 text-white/65">
                    {t(
                      "用于开发或排障，可完整拉取并编译，也可只编译 UI 或 clawd。",
                      "For development or troubleshooting. Pull and build everything, or build only the UI or clawd.",
                    )}
                  </p>
                </div>
              </div>
              <div className="mt-3 flex items-start gap-2 rounded-lg border border-amber-300/20 bg-amber-300/[0.06] px-3 py-2 text-xs leading-5 text-amber-100/85">
                <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
                <span>
                  {t(
                    "自己编译存在风险：耗时较长，会占用较多 CPU、内存和磁盘；依赖、网络或本地源码冲突都可能导致失败，低配置设备可能暂时无法响应。",
                    "Compiling locally carries risk: it can take a long time and consume significant CPU, memory, and disk. Dependencies, network issues, or local source conflicts can fail the build, and low-resource devices may become temporarily unresponsive.",
                  )}
                </span>
              </div>
              <div className="mt-4 flex flex-wrap gap-2">
                <button
                  type="button"
                  onClick={() => void onStartWorkspaceUpdate("full")}
                  disabled={workspaceUpdateLoading || workspaceUpdateRunning || systemRestarting}
                  className="theme-secondary-btn px-3 py-2 text-sm"
                >
                  <RefreshCw className="h-4 w-4" />
                  {workspaceUpdateHasRemoteDiff ? t("拉取并编译", "Pull and Build") : t("完整编译", "Build All")}
                </button>
                <button
                  type="button"
                  onClick={() => void onStartWorkspaceUpdate("ui_only")}
                  disabled={workspaceUpdateLoading || workspaceUpdateRunning || systemRestarting}
                  className="theme-secondary-btn px-3 py-2 text-sm"
                >
                  <LayoutDashboard className="h-4 w-4" />
                  {t("只编译 UI", "Build UI")}
                </button>
                <button
                  type="button"
                  onClick={() => void onStartWorkspaceUpdate("clawd_only")}
                  disabled={workspaceUpdateLoading || workspaceUpdateRunning || systemRestarting}
                  className="theme-secondary-btn px-3 py-2 text-sm"
                >
                  <Cpu className="h-4 w-4" />
                  {t("只编译 clawd", "Build clawd")}
                </button>
              </div>
            </div>
          </div>
        ) : null}

        {isAdminIdentity ? (
          <div className="flex flex-wrap items-center gap-2">
            {workspaceUpdateStatus?.status === "running" ? (
              <button
                type="button"
                onClick={() => void onCancelWorkspaceUpdate()}
                disabled={workspaceUpdateCanceling || systemRestarting}
                className="theme-secondary-btn px-3 py-2 text-sm text-red-100 hover:border-red-400/35 hover:bg-red-500/10"
              >
                {workspaceUpdateCanceling ? (
                  <Loader2 className="h-4 w-4 animate-spin" />
                ) : (
                  <X className="h-4 w-4" />
                )}
                {workspaceUpdateCanceling
                  ? t("停止中", "Stopping")
                  : workspaceUpdateStatus.mode === "release_deploy"
                    ? t("停止更新", "Stop Update")
                    : t("停止编译", "Stop Build")}
              </button>
            ) : null}
            <button
              type="button"
              onClick={() => {
                const confirmed = window.confirm(
                  t(
                    "现在重启 RustClaw？重启期间页面会短暂断开，稍后会自动恢复。",
                    "Restart RustClaw now? The page may disconnect briefly and then recover.",
                  ),
                );
                if (confirmed) void onRestartSystem();
              }}
              disabled={workspaceUpdateLoading || workspaceUpdateStatus?.status === "running" || systemRestarting}
              className="theme-secondary-btn px-3 py-2 text-sm"
            >
              {systemRestarting ? <Loader2 className="h-4 w-4 animate-spin" /> : <RefreshCw className="h-4 w-4" />}
              {systemRestarting ? t("重启中", "Restarting") : t("重启 RustClaw", "Restart RustClaw")}
            </button>
            {piAppStatus?.available ? (
              <button
                type="button"
                onClick={() => {
                  const confirmed = window.confirm(
                    t(
                      "现在重启 Pi App 小程序？小屏界面会短暂关闭后重新打开。",
                      "Restart the Pi App now? The small-screen app will close briefly and reopen.",
                    ),
                  );
                  if (confirmed) void onRestartPiApp();
                }}
                disabled={piAppRestarting || systemRestarting}
                className="theme-secondary-btn px-3 py-2 text-sm"
                title={piAppStatus.model || undefined}
              >
                {piAppRestarting ? <Loader2 className="h-4 w-4 animate-spin" /> : <RefreshCw className="h-4 w-4" />}
                {piAppRestarting ? t("重启中", "Restarting") : t("重启 Pi App", "Restart Pi App")}
              </button>
            ) : null}
          </div>
        ) : null}

        {workspaceUpdateMessage ? (
          <p className="mt-4 rounded-xl border border-sky-400/25 bg-sky-400/10 px-3 py-2 text-sm text-sky-100">
            {workspaceUpdateMessage}
          </p>
        ) : null}
        {systemRestartMessage ? (
          <p className="mt-3 rounded-xl border border-emerald-400/25 bg-emerald-400/10 px-3 py-2 text-sm text-emerald-100">
            {systemRestartMessage}
          </p>
        ) : null}
        {piAppRestartMessage ? (
          <p className="mt-3 rounded-xl border border-emerald-400/25 bg-emerald-400/10 px-3 py-2 text-sm text-emerald-100">
            {piAppRestartMessage}
          </p>
        ) : null}

        <div className="mt-4 rounded-xl border border-white/8 bg-black/20 px-3 py-3">
          <div className="flex items-center justify-between gap-3">
            <p className="text-sm font-medium text-white/85">{t("编译进度", "Build Progress")}</p>
            <span className="font-mono text-xs text-white/55">{workspaceUpdateProgressPercent}%</span>
          </div>
          <div className="mt-3 h-2 overflow-hidden rounded-full bg-white/10">
            <div
              className={`workspace-build-progress-bar h-full rounded-full transition-all duration-500 ${
                workspaceUpdateProgressActive ? "workspace-build-progress-bar-active" : ""
              } ${
                workspaceUpdateDisplayStatus === "failed"
                  ? "bg-red-300"
                  : workspaceUpdateDisplayStatus === "canceled"
                    ? "bg-amber-300"
                    : workspaceUpdateDisplayStatus === "up_to_date" || workspaceUpdateRestarting
                      ? "bg-emerald-300"
                      : "bg-sky-300"
              }`}
              style={{ width: `${workspaceUpdateProgressPercent}%` }}
            />
          </div>
          <p className="mt-2 text-xs leading-5 text-white/50">{workspaceUpdateProgressLabel}</p>
        </div>

        <div className="mt-4 grid gap-3 md:grid-cols-4">
          <div className="rounded-xl border border-white/8 bg-black/20 px-3 py-3">
            <p className="text-[11px] tracking-[0.14em] text-white/45">{t("状态", "Status")}</p>
            <p
              className={`mt-2 text-sm font-semibold ${
                workspaceUpdateDisplayStatus === "failed"
                  ? "text-red-200"
                  : workspaceUpdateDisplayStatus === "up_to_date"
                    ? "text-emerald-200"
                    : workspaceUpdateRunning
                      ? "text-sky-200"
                      : "text-white/90"
              }`}
            >
              {workspaceUpdateStatusLabel(workspaceUpdateDisplayStatus)}
            </p>
          </div>
          <div className="rounded-xl border border-white/8 bg-black/20 px-3 py-3">
            <p className="text-[11px] tracking-[0.14em] text-white/45">{t("当前步骤", "Current step")}</p>
            <p className="mt-2 text-sm font-semibold text-white/90">
              {workspaceUpdateStepLabel(workspaceUpdateStatus?.step)}
            </p>
          </div>
          <div className="rounded-xl border border-white/8 bg-black/20 px-3 py-3">
            <p className="text-[11px] tracking-[0.14em] text-white/45">{t("本地版本", "Local version")}</p>
            <p className="mt-2 text-sm font-semibold text-white/90">
              {workspaceUpdateStatus?.old_commit || "--"}
              {workspaceUpdateStatus?.new_commit && workspaceUpdateStatus.new_commit !== workspaceUpdateStatus.old_commit
                ? ` -> ${workspaceUpdateStatus.new_commit}`
                : ""}
            </p>
            <p className="mt-1 text-xs text-white/50">
              {t("远端最新", "Remote latest")}: {workspaceUpdateStatus?.remote_commit || "--"}
            </p>
          </div>
          <div className="rounded-xl border border-white/8 bg-black/20 px-3 py-3">
            <p className="text-[11px] tracking-[0.14em] text-white/45">{t("开始时间", "Started")}</p>
            <p className="mt-2 text-sm font-semibold text-white/90">
              {workspaceUpdateTimeLabel(workspaceUpdateStatus?.started_ts)}
            </p>
          </div>
        </div>

        {workspaceUpdateNotice ? (
          <div
            className={`mt-4 rounded-xl border px-3 py-3 text-sm ${
              workspaceUpdateNotice.tone === "error"
                ? "border-red-500/30 bg-red-500/10 text-red-100"
                : workspaceUpdateNotice.tone === "success"
                  ? "border-emerald-500/25 bg-emerald-500/10 text-emerald-100"
                  : "border-sky-400/25 bg-sky-400/10 text-sky-100"
            }`}
          >
            <p className="font-semibold">{workspaceUpdateNotice.title}</p>
            <p className="mt-1 opacity-80">{workspaceUpdateNotice.detail}</p>
          </div>
        ) : null}

        {workspaceUpdateLogPreview ? (
          <details className="mt-4 rounded-xl border border-white/10 bg-black/20 p-3">
            <summary className="cursor-pointer text-sm font-medium text-white/75">
              {workspaceUpdateRunning
                ? t("查看实时编译日志", "View live build logs")
                : t("查看最近日志摘要", "View recent log summary")}
            </summary>
            <pre className="mt-3 max-h-64 overflow-auto whitespace-pre-wrap break-words rounded-lg bg-black/30 p-3 text-xs leading-5 text-white/65">
              {workspaceUpdateLogPreview}
            </pre>
          </details>
        ) : null}
      </section>

      {dashboardCommunicationRows.length > 0 ? (
        <section className="rounded-2xl border border-white/10 bg-white/5 p-4 sm:p-5">
          <div className="flex flex-wrap items-start justify-between gap-3">
            <div>
              <h3 className="text-base font-semibold">{t("已启动的通信端", "Running communication services")}</h3>
              <p className="mt-2 text-sm text-white/65">
                {t(
                  "首页只显示当前已经启动的通信端，并展示它们的运行状态、进程数量和内存占用。",
                  "Home only shows communication services that are currently running, together with their runtime status, process count, and memory usage.",
                )}
              </p>
            </div>
            <button type="button" onClick={() => onSetCurrentPage("services")} className="theme-topbar-btn px-3 py-2 text-sm">
              {t("去通信接入", "Open Communication Setup")}
            </button>
          </div>

          <div className="mt-4 grid gap-3 xl:grid-cols-2">
            {dashboardCommunicationRows.map((row) => (
              <div key={row.key} className="rounded-2xl border border-white/10 bg-black/20 p-4">
                <div className="flex items-start justify-between gap-3">
                  <div>
                    <p className="text-sm font-semibold text-white">{row.label}</p>
                    <p className="mt-1 text-xs text-white/55">{row.statusLabel}</p>
                  </div>
                  <span
                    className={
                      row.category === "ready"
                        ? "setup-status setup-status-done"
                        : row.category === "attention"
                          ? "setup-status setup-status-attention"
                          : row.category === "stopped"
                            ? "setup-status setup-status-todo"
                            : "setup-status"
                    }
                  >
                    {row.category === "ready"
                      ? t("运行中", "Running")
                      : row.category === "attention"
                        ? t("待处理", "Needs attention")
                        : row.category === "stopped"
                          ? t("未运行", "Stopped")
                          : t("未知", "Unknown")}
                  </span>
                </div>

                <p className="mt-3 text-sm leading-6 text-white/68">{row.detail}</p>

                <div className="mt-4 grid gap-3 sm:grid-cols-2">
                  <div className="rounded-xl border border-white/8 bg-white/5 px-3 py-3">
                    <p className="text-[11px] tracking-[0.14em] text-white/45">{t("内存占用", "Memory usage")}</p>
                    <p className="mt-2 text-sm font-semibold text-white/92">{row.memoryLabel}</p>
                    <p className="mt-1 text-xs text-white/50">
                      {row.usesSharedGatewayMemory
                        ? t("当前显示的是共享 channel-gateway 内存。", "Currently showing shared channel-gateway memory.")
                        : t("当前显示的是该通信端进程内存。", "Currently showing memory for this service process.")}
                    </p>
                  </div>
                  <div className="rounded-xl border border-white/8 bg-white/5 px-3 py-3">
                    <p className="text-[11px] tracking-[0.14em] text-white/45">{t("进程数量", "Process count")}</p>
                    <p className="mt-2 text-sm font-semibold text-white/92">{row.processCount ?? "--"}</p>
                    <p className="mt-1 text-xs text-white/50">{row.statusLabel}</p>
                  </div>
                </div>
              </div>
            ))}
          </div>
        </section>
      ) : null}

      {(queuePressureHigh || runningTooOld || !isOnline) && (
        <section className="rounded-2xl border border-amber-500/30 bg-amber-500/10 p-4">
          <div className="flex items-start gap-3">
            <BellRing className="mt-0.5 h-5 w-5 shrink-0 text-amber-300" />
            <div className="space-y-1 text-sm">
              <p className="font-semibold text-amber-200">{t("现在有几项需要注意", "A few things need attention")}</p>
              {!isOnline ? (
                <p className="text-amber-100">
                  - {t("面板现在连不上 RustClaw。先检查服务地址是否正确，或者服务是否已经启动。", "The console cannot reach RustClaw right now. Check the service URL or start the service.")}
                </p>
              ) : null}
              {queuePressureHigh ? (
                <p className="text-amber-100">
                  - {t(`排队中的任务有 ${queueLength} 个，数量偏多，可能会让回复变慢。`, `There are ${queueLength} queued tasks, so replies may be slower than usual.`)}
                </p>
              ) : null}
              {runningTooOld ? (
                <p className="text-amber-100">
                  - {t(`有任务已经运行了 ${runningOldestAgeLabel}，时间偏长，建议留意。`, `One task has been running for ${runningOldestAgeLabel}, which is longer than expected.`)}
                </p>
              ) : null}
            </div>
          </div>
        </section>
      )}
    </>
  );
}
