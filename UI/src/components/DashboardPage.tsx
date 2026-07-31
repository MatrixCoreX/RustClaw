import { useState } from "react";
import {
  AlertTriangle,
  BellRing,
  ChevronUp,
  Cpu,
  Download,
  GitBranch,
  LayoutDashboard,
  Loader2,
  PowerOff,
  RefreshCw,
  ServerCog,
  ShieldCheck,
  Settings2,
  X,
} from "lucide-react";

import {
  areRequiredDashboardStepsComplete,
  type DashboardOverviewItem,
  type DashboardStepStatus,
} from "../lib/dashboard-home";
import {
  buildWorkspaceVersionDisplay,
  type WorkspaceUpdateNotice,
} from "../lib/workspace-update";
import { useUiDialog } from "./UiDialogProvider";
import { HostSystemSummaryPanel } from "./HostSystemSummaryPanel";
import { SystemDependenciesPanel } from "./SystemDependenciesPanel";
import { AgentPersonaCard } from "./AgentPersonaCard";
import type {
  AgentConfigResponse,
  ConsolePage,
  DashboardCommunicationRow,
  HostSystemSummary,
  HostDependenciesSnapshot,
  NginxUiStatus,
  PiAppStatusResponse,
  ServiceActionNotice,
  WebdExposureStatus,
  WorkspaceUpdateMode,
  WorkspaceUpdateStatus,
} from "../types/api";

type Translate = (zh: string, en: string) => string;

export interface DashboardOnboardingStep {
  key: string;
  required: boolean;
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
  hostSystemSummary: HostSystemSummary | null;
  hostSystemLoading: boolean;
  hostSystemErrorCode: string | null;
  hostDependencies: HostDependenciesSnapshot | null;
  hostDependenciesLoading: boolean;
  hostDependenciesErrorCode: string | null;
  dependencyInstallingId: string | null;
  isAdminIdentity: boolean;
  workspaceUpdateLoading: boolean;
  workspaceUpdateRunning: boolean;
  workspaceUpdateHasRemoteDiff: boolean;
  workspaceUpdateStatus: WorkspaceUpdateStatus | null;
  workspaceUpdateCanceling: boolean;
  workspaceUpdateMessage: string | null;
  nginxStatus: NginxUiStatus | null;
  nginxStatusLoading: boolean;
  nginxStatusError: string | null;
  webdExposureStatus: WebdExposureStatus | null;
  webdExposureLoading: boolean;
  webdExposureUpdating: boolean;
  webdExposureError: string | null;
  webdExposureMessage: string | null;
  workspaceUpdateRestarting: boolean;
  workspaceUpdateDisplayStatus: string | undefined;
  workspaceUpdateProgressVisible: boolean;
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
  serviceActionLoading: Record<string, boolean>;
  serviceActionMessage: ServiceActionNotice | null;
  queuePressureHigh: boolean;
  runningTooOld: boolean;
  isOnline: boolean;
  queueLength: number;
  runningOldestAgeLabel: string;
  agentConfig: AgentConfigResponse | null;
  agentConfigLoading: boolean;
  agentConfigSaving: boolean;
  agentConfigError: string | null;
  agentConfigMessage: string | null;
  onSetCurrentPage: (page: ConsolePage) => void;
  onFetchWorkspaceUpdateStatus: () => unknown | Promise<unknown>;
  onFetchNginxStatus: () => unknown | Promise<unknown>;
  onFetchWebdExposureStatus: () => unknown | Promise<unknown>;
  onSetWebdExternalAccess: (externallyAccessible: boolean) => unknown | Promise<unknown>;
  onStartWorkspaceUpdate: (mode: WorkspaceUpdateMode) => unknown | Promise<unknown>;
  onCancelWorkspaceUpdate: () => unknown | Promise<unknown>;
  onRestartSystem: () => unknown | Promise<unknown>;
  onRestartPiApp: () => unknown | Promise<unknown>;
  onControlService: (
    serviceName: DashboardCommunicationRow["serviceName"],
    action: "stop",
  ) => unknown | Promise<unknown>;
  onFetchHostSystemSummary: () => unknown | Promise<unknown>;
  onFetchHostDependencies: () => unknown | Promise<unknown>;
  onInstallHostDependency: (dependencyId: string) => unknown | Promise<unknown>;
  onFetchAgentConfig: () => unknown | Promise<unknown>;
  onSaveAgentPersona: (agentId: string, profile: string, customPersona: string) => Promise<boolean>;
  workspaceUpdateStepLabel: (step?: string) => string;
  workspaceUpdateStatusLabel: (status?: string) => string;
  workspaceUpdateTimeLabel: (ts?: number | null) => string;
}

export function DashboardPage({
  t,
  onboardingSteps,
  dashboardOverviewItems,
  hostSystemSummary,
  hostSystemLoading,
  hostSystemErrorCode,
  hostDependencies,
  hostDependenciesLoading,
  hostDependenciesErrorCode,
  dependencyInstallingId,
  isAdminIdentity,
  workspaceUpdateLoading,
  workspaceUpdateRunning,
  workspaceUpdateHasRemoteDiff,
  workspaceUpdateStatus,
  workspaceUpdateCanceling,
  workspaceUpdateMessage,
  nginxStatus,
  nginxStatusLoading,
  nginxStatusError,
  webdExposureStatus,
  webdExposureLoading,
  webdExposureUpdating,
  webdExposureError,
  webdExposureMessage,
  workspaceUpdateRestarting,
  workspaceUpdateDisplayStatus,
  workspaceUpdateProgressVisible,
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
  serviceActionLoading,
  serviceActionMessage,
  queuePressureHigh,
  runningTooOld,
  isOnline,
  queueLength,
  runningOldestAgeLabel,
  agentConfig,
  agentConfigLoading,
  agentConfigSaving,
  agentConfigError,
  agentConfigMessage,
  onSetCurrentPage,
  onFetchWorkspaceUpdateStatus,
  onFetchNginxStatus,
  onFetchWebdExposureStatus,
  onSetWebdExternalAccess,
  onStartWorkspaceUpdate,
  onCancelWorkspaceUpdate,
  onRestartSystem,
  onRestartPiApp,
  onControlService,
  onFetchHostSystemSummary,
  onFetchHostDependencies,
  onInstallHostDependency,
  onFetchAgentConfig,
  onSaveAgentPersona,
  workspaceUpdateStepLabel,
  workspaceUpdateStatusLabel,
  workspaceUpdateTimeLabel,
}: DashboardPageProps) {
  const { confirm: showConfirm } = useUiDialog();
  const latestReleaseStatus = workspaceUpdateStatus?.latest_release_check_status;
  const sourceUpdateAvailable = workspaceUpdateStatus?.source_update_available === true;
  const canEnableSourceCheckout = workspaceUpdateStatus?.installation_kind === "release_package";
  const latestReleaseDisplay =
    workspaceUpdateStatus?.latest_release_tag ||
    (latestReleaseStatus === "unavailable"
      ? t("暂时无法获取", "Temporarily unavailable")
      : t("正在检查...", "Checking..."));
  const workspaceVersionDisplay = buildWorkspaceVersionDisplay(workspaceUpdateStatus);
  const requiredSetupComplete = areRequiredDashboardStepsComplete(onboardingSteps);
  const [completedSetupExpanded, setCompletedSetupExpanded] = useState(false);
  const showOnboarding = !requiredSetupComplete || completedSetupExpanded;
  const nginxReady = Boolean(nginxStatus?.running && nginxStatus.configured && nginxStatus.ui_deployed);

  return (
    <>
      <AgentPersonaCard
        t={t}
        config={agentConfig}
        loading={agentConfigLoading}
        saving={agentConfigSaving}
        error={agentConfigError}
        message={agentConfigMessage}
        onRefresh={onFetchAgentConfig}
        onSave={onSaveAgentPersona}
        onOpenChat={() => onSetCurrentPage("chat")}
      />

      {showOnboarding ? (
        <section className="theme-panel setup-hero p-5 sm:p-6">
          <div className="flex flex-wrap items-start justify-between gap-4">
            <div className="max-w-3xl">
              <p className="theme-kicker text-[10px] uppercase tracking-[0.35em]">{t("首次使用", "First run")}</p>
              <h3 className="mt-2 text-xl font-semibold tracking-tight sm:text-3xl">
                {t("开始使用 {product_name}", "Start using {product_name}")}
              </h3>
              <p className="mt-3 text-sm leading-7 text-white/70 sm:text-base">
                {t(
                  "请先完成大模型配置和消息测试；通信接入是可选项，只在需要时配置。",
                  "Complete the model setup and a test message first. Communication setup is optional and only needed when you want it.",
                )}
              </p>
            </div>
            {requiredSetupComplete ? (
              <button
                type="button"
                onClick={() => setCompletedSetupExpanded(false)}
                className="theme-topbar-btn px-3 py-2 text-sm"
              >
                <ChevronUp className="h-4 w-4" />
                {t("收起", "Collapse")}
              </button>
            ) : null}
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
      ) : (
        <div className="flex justify-end">
          <button
            type="button"
            onClick={() => setCompletedSetupExpanded(true)}
            className="theme-topbar-btn px-3 py-2 text-sm"
            title={t("重新查看模型、消息测试和通信接入设置", "Review model, message test, and communication settings")}
          >
            <Settings2 className="h-4 w-4" />
            {t("重新配置", "Reconfigure")}
          </button>
        </div>
      )}

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

      <HostSystemSummaryPanel
        t={t}
        summary={hostSystemSummary}
        loading={hostSystemLoading}
        errorCode={hostSystemErrorCode}
        onRefresh={onFetchHostSystemSummary}
      />

      <SystemDependenciesPanel
        t={t}
        snapshot={hostDependencies}
        loading={hostDependenciesLoading}
        errorCode={hostDependenciesErrorCode}
        isAdmin={isAdminIdentity}
        installingId={dependencyInstallingId}
        onRefresh={onFetchHostDependencies}
        onInstall={onInstallHostDependency}
      />

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
          <div className={sourceUpdateAvailable ? "grid gap-4 lg:grid-cols-2" : "grid gap-4"}>
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
                  <div className="mt-3 grid gap-2 text-xs sm:grid-cols-2">
                    <div className="rounded-lg border border-white/8 bg-black/15 px-3 py-2">
                      <p className="text-white/45">
                        {workspaceVersionDisplay.kind === "git"
                          ? t("本地 Git", "Local Git")
                          : workspaceVersionDisplay.kind === "release"
                            ? t("当前 Release", "Current Release")
                            : t("当前版本", "Current version")}
                      </p>
                      <p className="mt-1 font-mono text-white/85">
                        {workspaceVersionDisplay.current}
                      </p>
                    </div>
                    <div className="rounded-lg border border-white/8 bg-black/15 px-3 py-2">
                      <p className="text-white/45">{t("最新 Release", "Latest Release")}</p>
                      <p className="mt-1 break-all font-mono text-white/85">
                        {latestReleaseDisplay}
                      </p>
                      {latestReleaseStatus === "stale" ? (
                        <p className="mt-1 text-[11px] leading-4 text-amber-200/75">
                          {t("当前显示缓存版本，远端检查暂时失败。", "Showing the cached version because the remote check failed.")}
                        </p>
                      ) : null}
                      {latestReleaseStatus === "git_tag" ? (
                        <p className="mt-1 text-[11px] leading-4 text-white/55">
                          {t(
                            "已从远端 Git 标签识别版本；Release 附件将在更新时再次验证。",
                            "Version identified from the remote Git tag; the Release asset will be verified again during update.",
                          )}
                        </p>
                      ) : null}
                      {latestReleaseStatus === "unavailable" ? (
                        <p
                          className="mt-1 text-[11px] leading-4 text-amber-200/75"
                          title={workspaceUpdateStatus?.latest_release_check_error || undefined}
                        >
                          {t("请点击“检查远端版本”重试。", "Click Check remote to retry.")}
                        </p>
                      ) : null}
                    </div>
                  </div>
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
              {canEnableSourceCheckout ? (
                <button
                  type="button"
                  onClick={() => void onStartWorkspaceUpdate("source_checkout")}
                  disabled={workspaceUpdateLoading || workspaceUpdateRunning || systemRestarting}
                  className="theme-secondary-btn mt-2 px-3 py-2 text-sm"
                  title={t(
                    "获取完整 Git 源码并切换到可拉取、可编译的开发模式",
                    "Fetch the complete Git source and switch to pull-and-build development mode",
                  )}
                >
                  {workspaceUpdateRunning && workspaceUpdateStatus?.mode === "source_checkout" ? (
                    <Loader2 className="h-4 w-4 animate-spin" />
                  ) : (
                    <GitBranch className="h-4 w-4" />
                  )}
                  {workspaceUpdateRunning && workspaceUpdateStatus?.mode === "source_checkout"
                    ? t("切换中", "Switching")
                    : t("切换到源码模式", "Switch to source mode")}
                </button>
              ) : null}
            </div>

            {sourceUpdateAvailable ? (
              <div className="rounded-lg border border-amber-400/25 bg-amber-400/[0.06] p-4 sm:p-5">
                <div className="flex items-start gap-3">
                  <span className="rounded-lg bg-amber-400/10 p-2 text-amber-200">
                    <Cpu className="h-5 w-5" />
                  </span>
                  <div>
                    <h4 className="text-sm font-semibold text-white">
                      {t("拉取源码并编译/部署", "Pull, Build, and Deploy Source")}
                    </h4>
                    <p className="mt-2 text-sm leading-6 text-white/65">
                      {t(
                        "用于开发或排障。完整流程会拉取并编译全部内容；如果已配置 nginx，还会自动替换为最新 UI。也可以只编译 UI 或 clawd。",
                        "For development or troubleshooting. The full flow pulls and builds everything, then replaces the nginx-hosted UI when nginx is already configured. You can also build only the UI or clawd.",
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
                    {workspaceUpdateHasRemoteDiff
                      ? t("拉取、编译并部署", "Pull, Build, and Deploy")
                      : t("完整编译/部署", "Build and Deploy All")}
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
            ) : null}
          </div>
        ) : null}

        {isAdminIdentity ? (
          <div className="grid items-stretch gap-4 lg:grid-cols-2">
          <div className="rounded-lg border border-emerald-400/20 bg-emerald-400/[0.05] p-4 sm:p-5">
            <div className="flex flex-wrap items-start justify-between gap-4">
              <div className="flex min-w-0 items-start gap-3">
                <span className="rounded-lg bg-emerald-400/10 p-2 text-emerald-200">
                  <ShieldCheck className="h-5 w-5" />
                </span>
                <div className="min-w-0">
                  <div className="flex flex-wrap items-center gap-2">
                    <h4 className="text-sm font-semibold text-white">
                      {t("Agent系统WEBD组件", "Agent system WEBD component")}
                    </h4>
                    <span className={`rounded-full px-2 py-0.5 text-[11px] ${webdExposureStatus?.externally_accessible ? "bg-amber-400/10 text-amber-100" : "bg-emerald-400/10 text-emerald-100"}`}>
                      {!webdExposureStatus
                        ? t("读取中", "Checking")
                        : webdExposureStatus.externally_accessible
                          ? t("允许直接访问", "Direct access on")
                          : t("仅本机", "Local only")}
                    </span>
                  </div>
                  <p className="mt-2 text-sm leading-6 text-white/65">
                    {t(
                      "关闭后 webd 只接受本机连接，nginx 仍可通过回环地址转发 UI 和 API。需要直接使用设备 IP 访问时再开放。",
                      "When closed, webd accepts local connections only while nginx continues proxying the UI and API over loopback. Expose it only when direct device-IP access is required.",
                    )}
                  </p>
                </div>
              </div>
              <button
                type="button"
                onClick={() => void onFetchWebdExposureStatus()}
                disabled={webdExposureLoading || webdExposureUpdating || workspaceUpdateRunning}
                className="theme-topbar-btn shrink-0 px-3 py-2 text-sm"
                title={t("刷新 webd 状态", "Refresh webd status")}
              >
                {webdExposureLoading ? <Loader2 className="h-4 w-4 animate-spin" /> : <RefreshCw className="h-4 w-4" />}
                {t("刷新状态", "Refresh status")}
              </button>
            </div>

            <div className="mt-4 flex flex-wrap gap-2 text-xs">
              <span className={`rounded-full border px-2.5 py-1 ${webdExposureStatus?.running ? "border-emerald-400/20 bg-emerald-400/[0.08] text-emerald-100" : "border-white/10 bg-black/15 text-white/50"}`}>
                {!webdExposureStatus
                  ? t("状态未知", "Unknown")
                  : webdExposureStatus.running
                    ? t("运行中", "Running")
                    : t("未运行", "Not running")}
              </span>
              <span className="rounded-full border border-white/10 bg-black/15 px-2.5 py-1 text-white/65">
                {t("端口", "Port")} {webdExposureStatus?.port ?? 8788}
              </span>
            </div>

            <div className="mt-4 flex items-center justify-between gap-4 rounded-md border border-white/10 bg-black/10 px-3 py-3">
              <div className="min-w-0">
                <p className="text-sm font-medium text-white/85">{t("监听地址", "Listen address")}</p>
                <p className="mt-1 text-xs leading-5 text-white/50">
                  {webdExposureStatus?.externally_accessible ? "IP" : "127.0.0.1"}
                  {`:${webdExposureStatus?.port ?? 8788}`}
                </p>
              </div>
              <div className="flex shrink-0 items-center gap-3">
                <span className="text-sm font-medium text-white/70">
                  {webdExposureStatus?.externally_accessible ? t("开启", "On") : t("关闭", "Off")}
                </span>
                <button
                  type="button"
                  role="switch"
                  aria-checked={webdExposureStatus?.externally_accessible === true}
                  aria-label={t("切换 webd 对外端口", "Toggle the public webd port")}
                  onClick={() => void onSetWebdExternalAccess(!webdExposureStatus?.externally_accessible)}
                  disabled={!webdExposureStatus || webdExposureUpdating || workspaceUpdateRunning || systemRestarting || webdExposureStatus.supported === false}
                  className={`relative h-6 w-11 shrink-0 overflow-hidden rounded-full border p-0 transition ${webdExposureStatus?.externally_accessible ? "border-amber-300/50 bg-amber-400/35" : "border-white/15 bg-white/10"} disabled:cursor-not-allowed disabled:opacity-45`}
                >
                  <span className={`absolute left-0.5 top-0.5 h-4 w-4 rounded-full bg-white shadow-sm transition-transform ${webdExposureStatus?.externally_accessible ? "translate-x-5" : "translate-x-0"}`} />
                </button>
              </div>
            </div>

            {webdExposureError ? (
              <p className="mt-3 text-sm text-amber-100">{t("状态读取或修改失败", "Status read or update failed")}: {webdExposureError}</p>
            ) : null}
            {webdExposureMessage ? <p className="mt-3 text-sm text-emerald-100">{webdExposureMessage}</p> : null}
          </div>

          <div className="rounded-lg border border-sky-400/20 bg-sky-400/[0.06] p-4 sm:p-5">
            <div className="flex flex-wrap items-start justify-between gap-4">
              <div className="flex max-w-3xl items-start gap-3">
                <span className="rounded-lg bg-sky-400/10 p-2 text-sky-200">
                  <ServerCog className="h-5 w-5" />
                </span>
                <div>
                  <div className="flex flex-wrap items-center gap-2">
                    <h4 className="text-sm font-semibold text-white">
                      {t(
                        "WEB服务器配置入口（本地运行可以不配置，但是要保持webd对外端口打开）",
                        "Web server entry configuration (optional for local use; keep the webd public port open when omitted)",
                      )}
                    </h4>
                    <span className={`rounded-full px-2 py-0.5 text-[11px] ${nginxReady ? "bg-emerald-400/10 text-emerald-200" : "bg-white/8 text-white/60"}`}>
                      {nginxReady ? t("已就绪", "Ready") : t("待配置", "Setup needed")}
                    </span>
                  </div>
                  <p className="mt-2 text-sm leading-6 text-white/65">
                    {t(
                      "nginx 提供对外页面并把 API 交给 webd；clawd 只保留本机内部 API，不直接对外开放。",
                      "nginx serves the public UI and passes API traffic to webd. clawd remains an internal-only API and is not exposed directly.",
                    )}
                  </p>
                </div>
              </div>
              <button
                type="button"
                onClick={() => void onFetchNginxStatus()}
                disabled={nginxStatusLoading || workspaceUpdateRunning}
                className="theme-topbar-btn px-3 py-2 text-sm"
              >
                {nginxStatusLoading ? <Loader2 className="h-4 w-4 animate-spin" /> : <RefreshCw className="h-4 w-4" />}
                {t("刷新状态", "Refresh status")}
              </button>
            </div>

            <div className="mt-4 flex flex-wrap gap-2 text-xs">
              {[
                [t("已安装", "Installed"), nginxStatus?.installed],
                [t("运行中", "Running"), nginxStatus?.running],
                [t("站点已配置", "Site configured"), nginxStatus?.configured],
                [t("UI 已部署", "UI deployed"), nginxStatus?.ui_deployed],
              ].map(([label, ready]) => (
                <span
                  key={String(label)}
                  className={`rounded-full border px-2.5 py-1 ${
                    ready
                      ? "border-emerald-400/20 bg-emerald-400/[0.08] text-emerald-100"
                      : "border-white/10 bg-black/15 text-white/50"
                  }`}
                >
                  {label}
                </span>
              ))}
              <span className="rounded-full border border-emerald-400/20 bg-emerald-400/[0.08] px-2.5 py-1 text-emerald-100">
                {t("clawd 仅本机", "clawd local only")}
              </span>
            </div>

            {nginxStatus?.supported === false ? (
              <p className="mt-3 text-sm text-amber-100">
                {t("当前系统不支持自动管理 nginx。", "Automatic nginx management is not supported on this system.")}
              </p>
            ) : null}
            {nginxStatusError ? (
              <p className="mt-3 text-sm text-amber-100">{t("状态读取失败", "Status check failed")}: {nginxStatusError}</p>
            ) : null}

            <div className="mt-4 flex flex-wrap gap-2">
              <button
                type="button"
                onClick={() => void onStartWorkspaceUpdate("nginx_enable")}
                disabled={workspaceUpdateLoading || workspaceUpdateRunning || systemRestarting || nginxStatus?.supported === false}
                className="theme-secondary-btn px-3 py-2 text-sm"
              >
                {workspaceUpdateRunning && workspaceUpdateStatus?.mode === "nginx_enable" ? (
                  <Loader2 className="h-4 w-4 animate-spin" />
                ) : (
                  <ServerCog className="h-4 w-4" />
                )}
                {t("启用/修复 nginx", "Enable/Repair nginx")}
              </button>
              {nginxStatus?.running || nginxStatus?.configured || nginxStatus?.ui_deployed ? (
                <button
                  type="button"
                  onClick={() => void onStartWorkspaceUpdate("nginx_disable")}
                  disabled={workspaceUpdateLoading || workspaceUpdateRunning || systemRestarting || nginxStatus?.supported === false}
                  className="theme-secondary-btn px-3 py-2 text-sm text-red-100 hover:border-red-400/35 hover:bg-red-500/10"
                >
                  {workspaceUpdateRunning && workspaceUpdateStatus?.mode === "nginx_disable" ? (
                    <Loader2 className="h-4 w-4 animate-spin" />
                  ) : (
                    <PowerOff className="h-4 w-4" />
                  )}
                  {t("关闭 nginx", "Disable nginx")}
                </button>
              ) : null}
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
                    : workspaceUpdateStatus.mode === "nginx_enable" || workspaceUpdateStatus.mode === "nginx_disable"
                      ? t("停止 nginx 操作", "Stop nginx operation")
                    : t("停止编译", "Stop Build")}
              </button>
            ) : null}
            <button
              type="button"
              onClick={async () => {
                const confirmed = await showConfirm({
                  title: t("重启 {product_name}", "Restart {product_name}"),
                  message: t(
                    "现在重启 {product_name}？重启期间页面会短暂断开，稍后会自动恢复。",
                    "Restart {product_name} now? The page may disconnect briefly and then recover.",
                  ),
                  confirmLabel: t("重启", "Restart"),
                });
                if (confirmed) void onRestartSystem();
              }}
              disabled={workspaceUpdateLoading || workspaceUpdateStatus?.status === "running" || systemRestarting}
              className="theme-secondary-btn px-3 py-2 text-sm"
            >
              {systemRestarting ? <Loader2 className="h-4 w-4 animate-spin" /> : <RefreshCw className="h-4 w-4" />}
              {systemRestarting ? t("重启中", "Restarting") : t("重启 {product_name}", "Restart {product_name}")}
            </button>
            {piAppStatus?.available ? (
              <button
                type="button"
                onClick={async () => {
                  const confirmed = await showConfirm({
                    title: t("重启 Pi App", "Restart Pi App"),
                    message: t(
                      "现在重启 Pi App 小程序？小屏界面会短暂关闭后重新打开。",
                      "Restart the Pi App now? The small-screen app will close briefly and reopen.",
                    ),
                    confirmLabel: t("重启", "Restart"),
                  });
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

        {workspaceUpdateProgressVisible ? (
          <div className="mt-4 rounded-xl border border-white/8 bg-black/20 px-3 py-3">
            <div className="flex items-center justify-between gap-3">
              <p className="text-sm font-medium text-white/85">{t("操作进度", "Operation Progress")}</p>
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
        ) : null}

        <div className="mt-4 grid gap-3 md:grid-cols-4">
          <div className="rounded-xl border border-white/8 bg-black/20 px-3 py-3">
            <p className="text-[11px] tracking-[0.14em] text-white/45">
              {t("更新任务状态", "Update task status")}
            </p>
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
            <p className="text-[11px] tracking-[0.14em] text-white/45">
              {workspaceVersionDisplay.kind === "git"
                ? t("本地 Git", "Local Git")
                : workspaceVersionDisplay.kind === "release"
                  ? t("当前 Release", "Current Release")
                  : t("本地版本", "Local version")}
            </p>
            <p className="mt-2 text-sm font-semibold text-white/90">
              {workspaceVersionDisplay.current}
            </p>
            <p className="mt-1 text-xs text-white/50">
              {workspaceVersionDisplay.kind === "git"
                ? t("远端 Git", "Remote Git")
                : workspaceVersionDisplay.kind === "release"
                  ? t("最新 Release", "Latest Release")
                  : t("最新版本", "Latest version")}
              : {workspaceVersionDisplay.latest}
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

          {serviceActionMessage ? (
            <p
              className={`mt-4 rounded-xl border px-3 py-3 text-sm ${
                serviceActionMessage.tone === "error"
                  ? "border-red-500/30 bg-red-500/10 text-red-100"
                  : "border-emerald-500/30 bg-emerald-500/10 text-emerald-100"
              }`}
            >
              {serviceActionMessage.text}
            </p>
          ) : null}

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

                <div className="mt-4 flex justify-end border-t border-white/8 pt-3">
                  <button
                    type="button"
                    onClick={async () => {
                      const confirmed = await showConfirm({
                        title: t(`关闭${row.label}`, `Stop ${row.label}`),
                        message: t(
                          "关闭后，这个通信端会停止接收和发送消息。之后可以在“通信接入”页面重新启动。确定关闭吗？",
                          "This communication service will stop receiving and sending messages. You can start it again later from Communication Setup. Stop it now?",
                        ),
                        confirmLabel: t("确认关闭", "Stop"),
                        tone: "danger",
                      });
                      if (confirmed) void onControlService(row.serviceName, "stop");
                    }}
                    disabled={Boolean(serviceActionLoading[row.serviceName])}
                    className="theme-secondary-btn px-3 py-2 text-sm"
                    aria-label={t(`关闭${row.label}`, `Stop ${row.label}`)}
                  >
                    {serviceActionLoading[row.serviceName] ? (
                      <Loader2 className="h-4 w-4 animate-spin" />
                    ) : (
                      <PowerOff className="h-4 w-4" />
                    )}
                    {serviceActionLoading[row.serviceName] ? t("关闭中", "Stopping") : t("关闭", "Stop")}
                  </button>
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
                  - {t("面板现在连不上 {product_name}。先检查服务地址是否正确，或者服务是否已经启动。", "The console cannot reach {product_name} right now. Check the service URL or start the service.")}
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
