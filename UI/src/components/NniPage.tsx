import {
  ArrowLeftRight,
  ChevronLeft,
  ChevronRight,
  Copy,
  Cpu,
  Fingerprint,
  KeyRound,
  Loader2,
  Network,
  Percent,
  RefreshCw,
  ShieldAlert,
  ShieldCheck,
  Trash2,
} from "lucide-react";
import { useEffect, useRef, useState } from "react";

import { writeTextToClipboard } from "../lib/auth-keys";
import { NniHistoryTabs, type NniHistoryView } from "./NniHistoryTabs";
import { NniNetworkDeviceStats } from "./NniNetworkDeviceStats";
import { NniPublicKeyDisplay } from "./NniPublicKeyDisplay";
import { NniRewardsPanel } from "./NniRewardsPanel";
import {
  NNI_RUNTIME_TILES,
  nniActionLabel,
  nniDeviceMessage,
  nniDeviceNextStep,
  nniPayloadHexField,
  parseNniRemoteNodeUrls,
  nniSimulationControlMode,
  nniTimestampSignatureReady,
  shortenHex,
  shortNniValue,
} from "../lib/nni-display";
import type {
  NniDeviceActionResponse,
  NniDeviceStatusResponse,
  NniHeartbeatErrorRecord,
  NniHeartbeatRecord,
  NniNetworkStatsResponse,
  NniRewardsResponse,
} from "../types/api";

type UiLanguage = "zh" | "en";
type Translate = (zh: string, en: string) => string;

export interface NniPageProps {
  lang: UiLanguage;
  t: Translate;
  nniStatus: NniDeviceStatusResponse | null;
  nniStatusLoading: boolean;
  nniStatusError: string | null;
  nniActionLoading: string | null;
  nniActionResult: NniDeviceActionResponse | null;
  nniActionError: string | null;
  nniActionMessage: string | null;
  nniDeviceAuthorizationDenied: boolean;
  nniJoined: boolean;
  nniRemoteNodes: string;
  nniSelectedNodeUrl: string;
  nniRemoteNodeCount: number;
  nniHeartbeatIntervalSeconds: number | null;
  nniHeartbeatRequestCount: number;
  nniHeartbeatRetryLimit: number;
  nniLastHeartbeatAtTs: number | null;
  nniLastHeartbeatNetworkFailures: number;
  nniHeartbeatRecords: NniHeartbeatRecord[];
  nniHeartbeatRecordsPage: number;
  nniHeartbeatRecordsTotal: number;
  nniHeartbeatRecordsTotalPages: number;
  nniHeartbeatRecordsLoading: boolean;
  nniHeartbeatRecordsClearing: boolean;
  nniHeartbeatRecordsError: string | null;
  nniHeartbeatRecordsMessage: string | null;
  nniHeartbeatRecordsPageSize: number;
  nniHeartbeatErrors: NniHeartbeatErrorRecord[];
  nniHeartbeatErrorsPage: number;
  nniHeartbeatErrorsTotal: number;
  nniHeartbeatErrorsTotalPages: number;
  nniHeartbeatErrorsLoading: boolean;
  nniHeartbeatErrorsClearing: boolean;
  nniHeartbeatErrorsError: string | null;
  nniHeartbeatErrorsMessage: string | null;
  nniHeartbeatErrorsPageSize: number;
  nniRewards: NniRewardsResponse | null;
  nniRewardsLoading: boolean;
  nniRewardsError: string | null;
  nniRewardsPageSize: number;
  nniNetworkStats: NniNetworkStatsResponse | null;
  nniNetworkStatsLoading: boolean;
  nniNetworkStatsError: string | null;
  nniCurrentPointBalance: string | null;
  nniCurrentPointBalanceLoading: boolean;
  nniConfigLoading: boolean;
  nniConfigSaving: boolean;
  nniConfigError: string | null;
  nniConfigMessage: string | null;
  formatUnixDateTime: (ts: number | null | undefined) => string;
  onFetchDeviceStatus: () => unknown | Promise<unknown>;
  onSetJoinedPersisted: (joined: boolean) => unknown | Promise<unknown>;
  onJoin: () => unknown | Promise<unknown>;
  onTestJoin: () => unknown | Promise<unknown>;
  onFetchConfig: () => unknown | Promise<unknown>;
  onSaveConfig: () => unknown | Promise<unknown>;
  onRemoteNodesChange: (value: string) => void;
  onSelectedNodeChange: (value: string) => void;
  onFetchHeartbeatRecords: (page: number) => unknown | Promise<unknown>;
  onClearHeartbeatRecords: () => unknown | Promise<unknown>;
  onFetchHeartbeatErrors: (page: number) => unknown | Promise<unknown>;
  onClearHeartbeatErrors: () => unknown | Promise<unknown>;
  onFetchRewards: (page: number) => unknown | Promise<unknown>;
  onFetchNetworkStats: () => unknown | Promise<unknown>;
  onFetchCurrentPointBalance: () => unknown | Promise<unknown>;
  onRunDeviceAction: (action: string) => unknown | Promise<unknown>;
  onSetDeviceSimulation: (enabled: boolean) => unknown | Promise<unknown>;
  onOpenApr: () => void;
  onOpenBancor: () => void;
  onActionMessageChange: (message: string | null) => void;
  onActionErrorChange: (message: string | null) => void;
}

const NNI_DEVICE_ACTIONS = [
  "pubkey",
  "sign_timestamp",
  "tng_device_pubkey",
  "tng_device_cert",
  "tng_signer_cert",
  "tng_root_cert",
];

const NNI_TEST_JOIN_ACTIVITY_MS = 2200;
export const NNI_DEVICE_MANAGEMENT_COPY = {
  zh: "这里管理硬件设备的 NNI 入口和设备签名能力。",
  en: "This page manages the hardware device's NNI entry and device-signing capability.",
} as const;
export const NNI_DEVICE_AUTHORIZATION_DENIED_COPY = {
  zh: "你不是合法设备，不能参与 NNI 网络。",
  en: "This is not an authorized device and cannot participate in the NNI network.",
} as const;

export function NniPage({
  lang,
  t,
  nniStatus,
  nniStatusLoading,
  nniStatusError,
  nniActionLoading,
  nniActionResult,
  nniActionError,
  nniActionMessage,
  nniDeviceAuthorizationDenied,
  nniJoined,
  nniRemoteNodes,
  nniSelectedNodeUrl,
  nniRemoteNodeCount,
  nniHeartbeatIntervalSeconds,
  nniHeartbeatRequestCount,
  nniHeartbeatRetryLimit,
  nniLastHeartbeatAtTs,
  nniLastHeartbeatNetworkFailures,
  nniHeartbeatRecords,
  nniHeartbeatRecordsPage,
  nniHeartbeatRecordsTotal,
  nniHeartbeatRecordsTotalPages,
  nniHeartbeatRecordsLoading,
  nniHeartbeatRecordsClearing,
  nniHeartbeatRecordsError,
  nniHeartbeatRecordsMessage,
  nniHeartbeatRecordsPageSize,
  nniHeartbeatErrors,
  nniHeartbeatErrorsPage,
  nniHeartbeatErrorsTotal,
  nniHeartbeatErrorsTotalPages,
  nniHeartbeatErrorsLoading,
  nniHeartbeatErrorsClearing,
  nniHeartbeatErrorsError,
  nniHeartbeatErrorsMessage,
  nniHeartbeatErrorsPageSize,
  nniRewards,
  nniRewardsLoading,
  nniRewardsError,
  nniRewardsPageSize,
  nniNetworkStats,
  nniNetworkStatsLoading,
  nniNetworkStatsError,
  nniCurrentPointBalance,
  nniCurrentPointBalanceLoading,
  nniConfigLoading,
  nniConfigSaving,
  nniConfigError,
  nniConfigMessage,
  formatUnixDateTime,
  onFetchDeviceStatus,
  onSetJoinedPersisted,
  onJoin,
  onTestJoin,
  onFetchConfig,
  onSaveConfig,
  onRemoteNodesChange,
  onSelectedNodeChange,
  onFetchHeartbeatRecords,
  onClearHeartbeatRecords,
  onFetchHeartbeatErrors,
  onClearHeartbeatErrors,
  onFetchRewards,
  onFetchNetworkStats,
  onFetchCurrentPointBalance,
  onRunDeviceAction,
  onSetDeviceSimulation,
  onOpenApr,
  onOpenBancor,
  onActionMessageChange,
  onActionErrorChange,
}: NniPageProps) {
  const [nniTestJoinPulse, setNniTestJoinPulse] = useState(false);
  const [nniHistoryView, setNniHistoryView] = useState<NniHistoryView>("overview");
  const nniTestJoinPulseTimer = useRef<number | null>(null);
  const nniChipPresent = nniStatus?.signature_chip_present === true;
  const nniChipMissing = nniStatus?.status === "signature_chip_missing";
  const nniDetectionUnavailable = nniStatus?.status === "detection_unavailable";
  const nniSimulated = nniStatus?.simulated === true;
  const nniSimulationControl = nniSimulationControlMode(nniStatus, nniStatusLoading);
  const nniPrimaryHex = nniPayloadHexField(nniActionResult?.payload);
  const nniHeartbeatRecordsCanPrev = nniHeartbeatRecordsPage > 1;
  const nniHeartbeatRecordsCanNext = nniHeartbeatRecordsPage < nniHeartbeatRecordsTotalPages;
  const nniHeartbeatErrorsCanPrev = nniHeartbeatErrorsPage > 1;
  const nniHeartbeatErrorsCanNext = nniHeartbeatErrorsPage < nniHeartbeatErrorsTotalPages;
  const nniHeartbeatIntervalMinutes =
    nniHeartbeatIntervalSeconds !== null && nniHeartbeatIntervalSeconds % 60 === 0
      ? nniHeartbeatIntervalSeconds / 60
      : null;
  const nniHeartbeatIntervalZh = nniHeartbeatIntervalMinutes !== null
    ? `${nniHeartbeatIntervalMinutes} 分钟`
    : nniHeartbeatIntervalSeconds !== null
      ? `${nniHeartbeatIntervalSeconds} 秒`
      : "系统设定的周期";
  const nniHeartbeatIntervalEn = nniHeartbeatIntervalMinutes !== null
    ? `${nniHeartbeatIntervalMinutes} minutes`
    : nniHeartbeatIntervalSeconds !== null
      ? `${nniHeartbeatIntervalSeconds} seconds`
      : "the configured interval";
  const actionLabel = (action: string) => nniActionLabel(action, lang);
  const nniRuntimeActivity =
    nniJoined || nniTestJoinPulse || ["join_nni", "sign_challenge", "sign_timestamp"].includes(nniActionLoading || "");
  const nniStatusMessage = nniStatusLoading
    ? t(
        "正在检测真实芯片，请稍候。",
        "Checking for a real chip. Please wait.",
      )
    : nniDeviceMessage(
        nniStatus,
        lang,
        t("还没有读取状态。点击刷新状态开始检测。", "Status has not been loaded yet. Click Refresh status to check."),
      ) ?? "";
  const nniStatusNextStep = nniDeviceNextStep(nniStatus, lang);

  useEffect(() => {
    return () => {
      if (nniTestJoinPulseTimer.current !== null) {
        window.clearTimeout(nniTestJoinPulseTimer.current);
      }
    };
  }, []);

  const runTestJoinWithRuntimePulse = async () => {
    if (nniTestJoinPulseTimer.current !== null) {
      window.clearTimeout(nniTestJoinPulseTimer.current);
      nniTestJoinPulseTimer.current = null;
    }
    setNniTestJoinPulse(true);
    let shouldHoldPulse = false;
    try {
      const result = await Promise.resolve(onTestJoin());
      shouldHoldPulse = nniTimestampSignatureReady(result as NniDeviceActionResponse | null);
    } finally {
      if (shouldHoldPulse) {
        nniTestJoinPulseTimer.current = window.setTimeout(() => {
          setNniTestJoinPulse(false);
          nniTestJoinPulseTimer.current = null;
        }, NNI_TEST_JOIN_ACTIVITY_MS);
      } else {
        setNniTestJoinPulse(false);
      }
    }
  };

  const copyPrimaryHex = (value?: string) => {
    const copyValue = value ?? nniPrimaryHex?.value;
    if (!copyValue) return;
    void writeTextToClipboard(copyValue)
      .then(() => onActionMessageChange(t("已复制结果。", "Result copied.")))
      .catch((err) => onActionErrorChange(err instanceof Error ? err.message : t("复制失败", "Copy failed")));
  };

  const refreshNniPageStatus = async () => {
    await Promise.allSettled([
      Promise.resolve(onFetchDeviceStatus()),
      Promise.resolve(onFetchNetworkStats()),
    ]);
    if (nniJoined) {
      // Both private reads use the device signer. Keep them sequential so a
      // hardware-backed signer never receives overlapping challenges.
      await Promise.resolve(onFetchRewards(1));
      await Promise.resolve(onFetchCurrentPointBalance());
    }
  };

  const refreshNniRewards = async () => {
    await Promise.resolve(onFetchRewards(1));
    await Promise.resolve(onFetchCurrentPointBalance());
  };

  return (
    <div className="flex flex-col gap-4">
      <section className="theme-panel p-5 sm:p-6">
        <div className="grid gap-5">
          <div className="max-w-3xl">
            <p className="theme-kicker text-[10px] uppercase tracking-[0.35em]">Network Native Intelligence</p>
            <h3 className="mt-2 flex items-center gap-2 text-xl font-semibold tracking-tight sm:text-2xl">
              <Network className="h-6 w-6 theme-icon-accent" />
              <span>{t("NNI 网络原生智能", "NNI Network-Native Intelligence")}</span>
            </h3>
            <p className="mt-3 text-sm leading-7 text-white/70">
              {t(
                NNI_DEVICE_MANAGEMENT_COPY.zh,
                NNI_DEVICE_MANAGEMENT_COPY.en,
              )}
            </p>
            {nniDeviceAuthorizationDenied ? (
              <div
                role="alert"
                className="mt-3 flex items-start gap-2 rounded-md border border-rose-400/35 bg-rose-500/10 px-3 py-2.5 text-sm leading-6 text-rose-100"
              >
                <ShieldAlert className="mt-0.5 h-4 w-4 shrink-0" />
                <span>
                  {t(
                    NNI_DEVICE_AUTHORIZATION_DENIED_COPY.zh,
                    NNI_DEVICE_AUTHORIZATION_DENIED_COPY.en,
                  )}
                </span>
              </div>
            ) : null}
          </div>

          <div className="grid w-full gap-3">
            <NniNetworkDeviceStats
              stats={nniNetworkStats?.network_devices ?? null}
              networkRewards={nniNetworkStats?.network_rewards ?? null}
              rewardPolicy={nniNetworkStats?.reward_policy ?? null}
              loading={nniNetworkStatsLoading}
              t={t}
              formatUnixDateTime={formatUnixDateTime}
            />
            {nniNetworkStatsError ? (
              <p className="text-xs text-amber-200/85">
                {t("网络概览暂时无法读取，请稍后刷新。", "Network overview is temporarily unavailable. Refresh later.")}
              </p>
            ) : null}
            <div className="flex flex-wrap justify-end gap-2">
              <button
                type="button"
                onClick={onOpenApr}
                className="theme-secondary-btn px-3 py-2 text-sm"
              >
                <Percent className="h-4 w-4" />
                APR
              </button>
              <button
                type="button"
                onClick={onOpenBancor}
                className="theme-secondary-btn px-3 py-2 text-sm"
              >
                <ArrowLeftRight className="h-4 w-4" />
                {t("交易", "Trade")}
              </button>
              <button
                type="button"
                onClick={() => void refreshNniPageStatus()}
                disabled={nniStatusLoading || nniRewardsLoading}
                className="theme-secondary-btn px-3 py-2 text-sm"
              >
                {nniStatusLoading || nniRewardsLoading ? <Loader2 className="h-4 w-4 animate-spin" /> : <RefreshCw className="h-4 w-4" />}
                {t("刷新状态", "Refresh status")}
              </button>
              <button
                type="button"
                onClick={() => (nniJoined ? void onSetJoinedPersisted(false) : void onJoin())}
                disabled={Boolean(nniActionLoading) || nniStatusLoading || (!nniJoined && (!nniChipPresent || nniRemoteNodeCount === 0))}
                className={nniJoined ? "theme-secondary-btn px-3 py-2 text-sm" : "theme-accent-btn px-3 py-2 text-sm"}
                title={
                  !nniJoined && !nniChipPresent
                    ? nniDetectionUnavailable
                      ? t("芯片检测暂时未完成，请稍后刷新状态。", "Chip detection is temporarily unavailable. Refresh the status later.")
                      : t("当前设备缺少芯片，不能加入需要设备签名的 NNI。", "This device has no chip, so it cannot join signed NNI.")
                    : nniRemoteNodeCount === 0
                      ? t("请先填写远程 NNI 节点地址。", "Enter a remote NNI node URL first.")
                      : undefined
                }
              >
                {["join_nni", "sign_challenge"].includes(nniActionLoading || "") ? (
                  <Loader2 className="h-4 w-4 animate-spin" />
                ) : (
                  <KeyRound className="h-4 w-4" />
                )}
                {nniJoined ? t("停止", "Stop") : t("加入", "Join")}
              </button>
              {!nniJoined ? (
                <button
                  type="button"
                  onClick={() => void runTestJoinWithRuntimePulse()}
                  disabled={Boolean(nniActionLoading) || nniStatusLoading}
                  className="theme-secondary-btn px-3 py-2 text-sm"
                  title={
                    nniChipMissing
                      ? t(
                          "上次检测未找到芯片；测试加入会重新尝试本机时间戳签名，不请求远程 NNI 服务端。",
                          "The last check did not find a chip. Test Join retries a local timestamp signature and does not contact the remote NNI server.",
                        )
                      : t(
                          "测试加入只做本机时间戳签名，不请求远程 NNI 服务端。",
                          "Test join only signs a local timestamp and does not contact the remote NNI server.",
                        )
                  }
                >
                  {nniActionLoading === "sign_timestamp" ? (
                    <Loader2 className="h-4 w-4 animate-spin" />
                  ) : (
                    <KeyRound className="h-4 w-4" />
                  )}
                  {t("测试加入", "Test Join")}
                </button>
              ) : null}
            </div>
          </div>
        </div>
      </section>

      {nniStatusError ? (
        <p className="rounded-2xl border border-red-500/30 bg-red-500/10 px-4 py-3 text-sm text-red-100">
          {nniStatusError}
        </p>
      ) : null}

      <NniHistoryTabs
        activeView={nniHistoryView}
        recordsTotal={nniHeartbeatRecordsTotal}
        errorsTotal={nniHeartbeatErrorsTotal}
        t={t}
        onChange={setNniHistoryView}
      />

      {nniHistoryView === "overview" ? (
        <section
          id="nni-overview-primary-panel"
          role="tabpanel"
          aria-labelledby="nni-overview-tab"
          className="grid gap-4 xl:grid-cols-[0.95fr_1.05fr]"
        >
        <div className="theme-panel-soft p-5">
          <div className="flex items-start justify-between gap-3">
            <div>
              <p className="theme-kicker text-[10px] uppercase tracking-[0.28em]">{t("设备状态", "Device status")}</p>
              <h4 className="mt-2 text-lg font-semibold">{t("芯片", "Chip")}</h4>
            </div>
            <div className="flex flex-col items-end gap-2">
              {nniSimulationControl ? (
                <button
                  type="button"
                  onClick={() => void onSetDeviceSimulation(nniSimulationControl === "enable")}
                  disabled={Boolean(nniActionLoading) || nniStatusLoading}
                  className="theme-secondary-btn px-3 py-1.5 text-xs disabled:cursor-not-allowed disabled:opacity-50"
                  title={
                    nniSimulated
                      ? t("停止软件模拟并重新检测真实芯片。", "Stop software simulation and check for a real chip again.")
                      : t(
                          "仅用于本机协议测试；模拟密钥不受真实硬件保护。",
                          "For local protocol testing only; simulated keys are not protected by real hardware.",
                        )
                  }
                >
                  {["simulation_enable", "simulation_disable"].includes(nniActionLoading || "") ? (
                    <Loader2 className="h-3.5 w-3.5 animate-spin" />
                  ) : (
                    <Cpu className="h-3.5 w-3.5" />
                  )}
                  {nniSimulated ? t("停止模拟", "Stop simulation") : t("模拟芯片", "Simulate chip")}
                </button>
              ) : null}
              <span
                className={
                  nniStatusLoading || nniSimulated
                    ? "setup-status"
                    : nniStatus == null
                      ? "setup-status setup-status-todo"
                      : nniChipPresent
                        ? "setup-status setup-status-done"
                        : "setup-status setup-status-attention"
                }
              >
                {nniStatusLoading ? (
                  <>
                    <Loader2 className="h-3.5 w-3.5 animate-spin" />
                    {t("检测中", "Checking")}
                  </>
                ) : nniSimulated ? (
                  <>
                    <Cpu className="h-3.5 w-3.5" />
                    {t("模拟中", "Simulated")}
                  </>
                ) : nniChipPresent ? (
                  <>
                    <ShieldCheck className="h-3.5 w-3.5" />
                    {t("可用", "Ready")}
                  </>
                ) : nniDetectionUnavailable ? (
                  <>
                    <ShieldAlert className="h-3.5 w-3.5" />
                    {t("暂不可用", "Unavailable")}
                  </>
                ) : nniStatus == null ? (
                  t("未检测", "Not checked")
                ) : (
                  <>
                    <ShieldAlert className="h-3.5 w-3.5" />
                    {t("缺少芯片", "Chip missing")}
                  </>
                )}
              </span>
            </div>
          </div>

          <div
            className={
              nniStatusLoading
                ? "mt-4 rounded-xl border border-sky-400/25 bg-sky-400/10 px-3 py-3 text-sm text-sky-50"
                : nniChipPresent && !nniSimulated
                  ? "mt-4 rounded-xl border border-emerald-500/25 bg-emerald-500/10 px-3 py-3 text-sm text-emerald-100"
                  : "mt-4 rounded-xl border border-amber-500/30 bg-amber-500/10 px-3 py-3 text-sm text-amber-100"
            }
            role={nniStatusLoading ? "status" : undefined}
            aria-live="polite"
          >
            {nniStatusLoading ? (
              <>
                <div className="flex items-start gap-3">
                  <Loader2 className="mt-0.5 h-5 w-5 shrink-0 animate-spin" />
                  <div>
                    <p className="font-medium">{nniStatusMessage}</p>
                    <p className="mt-1 text-sm opacity-75">
                      {t(
                        "请保持芯片连接。检测完成前不会显示模拟入口。",
                        "Keep the chip connected. Simulation will remain hidden until detection finishes.",
                      )}
                    </p>
                  </div>
                </div>
                <div className="mt-3 h-1.5 overflow-hidden rounded-full bg-black/20">
                  <div className="h-full w-2/3 animate-pulse rounded-full bg-sky-300" />
                </div>
              </>
            ) : (
              <>
                <p className="font-medium">{nniStatusMessage}</p>
                {nniStatusNextStep ? <p className="mt-1 text-sm opacity-80">{nniStatusNextStep}</p> : null}
              </>
            )}
          </div>

          <div className="mt-4 grid gap-3 sm:grid-cols-2">
            <div className="rounded-xl border border-white/10 bg-black/20 px-3 py-3">
              <p className="text-[11px] tracking-[0.14em] text-white/45">slot</p>
              <p className="mt-2 text-sm font-semibold text-white/90">{nniStatus?.meta?.slot ?? "--"}</p>
            </div>
            <div className="rounded-xl border border-white/10 bg-black/20 px-3 py-3">
              <p className="text-[11px] tracking-[0.14em] text-white/45">I2C</p>
              <p className="mt-2 text-sm font-semibold text-white/90">
                {nniStatus?.meta?.i2c_address || "--"}
                {nniStatus?.meta?.i2c_bus != null ? ` / bus ${nniStatus?.meta?.i2c_bus}` : ""}
              </p>
            </div>
            <div className="rounded-xl border border-white/10 bg-black/20 px-3 py-3 sm:col-span-2">
              <p className="text-[11px] tracking-[0.14em] text-white/45">{t("公钥指纹", "Public key fingerprint")}</p>
              <p className="mt-2 break-all font-mono text-sm font-semibold text-white/90">
                {nniStatus?.pubkey_fingerprint || nniStatus?.pubkey_preview || "--"}
              </p>
            </div>
          </div>
        </div>

        <div className="theme-panel-soft p-5">
          <div className="flex items-start justify-between gap-3">
            <div>
              <p className="theme-kicker text-[10px] uppercase tracking-[0.28em]">{t("加入状态", "Join state")}</p>
              <h4 className="mt-2 text-lg font-semibold">{t("NNI 运行入口", "NNI runtime entry")}</h4>
            </div>
            <span
              className={
                nniJoined
                  ? "setup-status setup-status-done"
                  : nniRuntimeActivity
                    ? "setup-status"
                    : "setup-status setup-status-todo"
              }
            >
              {nniJoined ? (
                t("心跳挑战中", "Heartbeat active")
              ) : nniRuntimeActivity ? (
                <>
                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                  {t("测试中", "Testing")}
                </>
              ) : (
                t("未加入", "Not joined")
              )}
            </span>
          </div>

          <div
            className={`nni-runtime-board mt-4 min-h-[180px] rounded-2xl border p-4 ${
              nniRuntimeActivity ? "nni-runtime-board-active" : "nni-runtime-board-idle"
            }`}
          >
            <div className="grid h-full min-h-[148px] grid-cols-6 gap-2 sm:grid-cols-8">
              {NNI_RUNTIME_TILES.map((tile, index) => (
                <div
                  key={index}
                  className={`nni-runtime-tile rounded-lg border ${
                    nniRuntimeActivity ? "nni-runtime-tile-active" : "nni-runtime-tile-idle"
                  }`}
                  style={{
                    animationDelay: `${tile.delay}s`,
                    animationDuration: `${tile.duration}s`,
                    opacity: nniRuntimeActivity ? undefined : tile.idleOpacity,
                  }}
                />
              ))}
            </div>
          </div>

          <div className="mt-4 grid gap-3 border-t border-white/10 pt-4 sm:grid-cols-3">
            <div>
              <p className="text-[11px] font-semibold tracking-[0.16em] text-white/45">
                {t("心跳请求次数", "Heartbeat requests")}
              </p>
              <p className="mt-1 text-xl font-semibold text-white/90">{nniHeartbeatRequestCount}</p>
            </div>
            <div>
              <p className="text-[11px] font-semibold tracking-[0.16em] text-white/45">
                {t("最近请求", "Latest request")}
              </p>
              <p className="mt-1 text-sm font-medium text-white/75">{formatUnixDateTime(nniLastHeartbeatAtTs)}</p>
            </div>
            <div>
              <p className="text-[11px] font-semibold tracking-[0.16em] text-white/45">
                {t("最近网络重试", "Latest network retries")}
              </p>
              <p className="mt-1 text-sm font-medium text-white/75">
                {nniLastHeartbeatNetworkFailures > 0
                  ? `${nniLastHeartbeatNetworkFailures} / ${nniHeartbeatRetryLimit}`
                  : `0 / ${nniHeartbeatRetryLimit}`}
              </p>
            </div>
          </div>

          <p className="mt-4 text-sm leading-7 text-white/65">
            {nniDetectionUnavailable
              ? t(
                  "本次芯片检测暂时未完成，不代表设备缺少芯片。请等待设备负载降低后刷新状态。",
                  "This chip check did not finish. It does not mean the chip is missing. Refresh after the device load decreases.",
                )
              : nniChipMissing
              ? t(
                  "当前设备缺少芯片，因此不会显示为已加入。你仍可以继续使用 {product_name} 的其它功能。",
                  "This device has no chip, so it will not be marked as joined. Other {product_name} features remain available.",
                )
              : nniJoined
                ? t(
                    `服务端已验证设备签名，NNI 运行入口已开启。Agent 会每 ${nniHeartbeatIntervalZh} 向服务器发送一次硬件签名心跳。`,
                    `The server verified the device signature, and the NNI runtime entry is active. The Agent will send a hardware-signed heartbeat to the server every ${nniHeartbeatIntervalEn}.`,
                  )
                : t(
                    "点击加入会向远程服务端请求一次随机挑战，验签通过后开启运行入口；测试加入只做本机时间戳签名，不请求远程服务端。",
                    "Click Join to request a random challenge from the remote server. The runtime is enabled after verification. Test Join only signs a local timestamp and does not contact the remote server.",
                  )}
          </p>

          <div className="mt-4 rounded-2xl border border-white/10 bg-black/20 p-3">
            <div className="flex flex-wrap items-center justify-between gap-2">
              <label className="text-[11px] font-semibold tracking-[0.16em] text-white/55">
                {t("远程 NNI 节点", "Remote NNI nodes")}
              </label>
              <div className="flex flex-wrap items-center gap-2">
                <button
                  type="button"
                  onClick={() => void onFetchConfig()}
                  disabled={nniConfigLoading || nniConfigSaving}
                  className="theme-secondary-btn px-3 py-1.5 text-xs"
                >
                  {nniConfigLoading ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <RefreshCw className="h-3.5 w-3.5" />}
                  {t("重新载入", "Reload")}
                </button>
                <button
                  type="button"
                  onClick={() => void onSaveConfig()}
                  disabled={nniConfigLoading || nniConfigSaving}
                  className="theme-accent-btn px-3 py-1.5 text-xs"
                >
                  {nniConfigSaving ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : null}
                  {t("保存节点", "Save nodes")}
                </button>
              </div>
            </div>
            <textarea
              className="theme-input mt-2 min-h-20 resize-y font-mono text-xs"
              placeholder={t(
                "例如：https://nni-node.example.com\n多个节点可以一行一个，再从下方选择当前节点。",
                "Example: https://nni-node.example.com\nUse one node per line, then select the active node below.",
              )}
              value={nniRemoteNodes}
              disabled={nniJoined}
              onChange={(event) => onRemoteNodesChange(event.target.value)}
            />
            {parseNniRemoteNodeUrls(nniRemoteNodes).length > 0 ? (
              <label className="mt-3 block text-xs text-white/65">
                <span className="mb-1.5 block font-semibold">{t("当前节点", "Active node")}</span>
                <select
                  className="theme-input w-full font-mono text-xs"
                  value={nniSelectedNodeUrl}
                  disabled={nniJoined}
                  onChange={(event) => onSelectedNodeChange(event.target.value)}
                >
                  {parseNniRemoteNodeUrls(nniRemoteNodes).map((nodeUrl) => (
                    <option key={nodeUrl} value={nodeUrl}>{nodeUrl}</option>
                  ))}
                </select>
              </label>
            ) : null}
            <p className="mt-2 text-xs leading-5 text-white/50">
              {t(
                "节点列表、当前节点和加入状态会保存。NNI 心跳、奖励与 Bancor 始终使用当前节点；需要切换时请先停止 NNI。",
                "The node list, active node, and Join state are saved. NNI heartbeat, rewards, and Bancor always use the active node; stop NNI before switching nodes.",
              )}
            </p>
            {nniConfigMessage ? <p className="mt-2 text-xs text-emerald-200">{nniConfigMessage}</p> : null}
            {nniConfigError ? <p className="mt-2 break-words text-xs text-red-200">{nniConfigError}</p> : null}
          </div>
        </div>
        </section>
      ) : null}

      {nniHistoryView !== "overview" ? (
        <section className="theme-panel-soft p-5">
        {nniHistoryView === "rewards" ? (
          <NniRewardsPanel
            rewards={nniRewards}
            currentPointBalance={nniCurrentPointBalance}
            currentPointBalanceLoading={nniCurrentPointBalanceLoading}
            loading={nniRewardsLoading}
            error={nniRewardsError}
            pageSize={nniRewardsPageSize}
            t={t}
            formatUnixDateTime={formatUnixDateTime}
            onRefresh={refreshNniRewards}
          />
        ) : null}

        {nniHistoryView === "errors" ? (
          <div
            id="nni-history-errors-panel"
            role="tabpanel"
            aria-labelledby="nni-history-errors-tab"
            className="mt-5"
          >
          <div className="flex flex-wrap items-start justify-between gap-3">
            <div>
              <p className="theme-kicker text-[10px] uppercase tracking-[0.28em]">
                {t("NNI 心跳错误", "NNI heartbeat errors")}
              </p>
              <h4 className="mt-2 text-lg font-semibold">{t("本机心跳错误记录", "Local heartbeat error history")}</h4>
              <p className="mt-2 text-sm leading-6 text-white/60">
                {t(
                  `共 ${nniHeartbeatErrorsTotal} 条错误记录，每页 ${nniHeartbeatErrorsPageSize} 条。`,
                  `${nniHeartbeatErrorsTotal} error records total, ${nniHeartbeatErrorsPageSize} per page.`,
                )}
              </p>
            </div>
            <div className="flex flex-wrap items-center gap-2">
              <button
                type="button"
                onClick={() => void onFetchHeartbeatErrors(nniHeartbeatErrorsPage)}
                disabled={nniHeartbeatErrorsLoading || nniHeartbeatErrorsClearing}
                className="theme-secondary-btn px-3 py-2 text-xs"
              >
                {nniHeartbeatErrorsLoading ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <RefreshCw className="h-3.5 w-3.5" />}
                {t("刷新错误", "Refresh errors")}
              </button>
              <button
                type="button"
                onClick={() => void onClearHeartbeatErrors()}
                disabled={nniHeartbeatErrorsLoading || nniHeartbeatErrorsClearing || nniHeartbeatErrorsTotal === 0}
                className="theme-secondary-btn px-3 py-2 text-xs disabled:cursor-not-allowed disabled:opacity-50"
                title={t(
                  "只清理本机保存的心跳错误历史，不会修改远程服务端请求记录。",
                  "Only clears local heartbeat error history. Remote server request records are not changed.",
                )}
              >
                {nniHeartbeatErrorsClearing ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Trash2 className="h-3.5 w-3.5" />}
                {t("清理错误", "Clear errors")}
              </button>
            </div>
          </div>

          {nniHeartbeatErrorsError ? (
            <p className="mt-3 break-words rounded-xl border border-amber-300/20 bg-amber-300/10 px-3 py-2 text-xs leading-5 text-amber-100">
              {t("NNI 心跳错误暂时无法载入：", "NNI heartbeat errors could not be loaded: ")}
              {nniHeartbeatErrorsError}
            </p>
        ) : null}
        {nniHeartbeatErrorsMessage ? (
          <p className="mt-3 rounded-xl border border-emerald-500/25 bg-emerald-500/10 px-3 py-2 text-xs leading-5 text-emerald-100">
            {nniHeartbeatErrorsMessage}
          </p>
        ) : null}

        <div className="mt-4 overflow-hidden rounded-2xl border border-white/10 bg-black/20">
          {nniHeartbeatErrors.length === 0 ? (
            <p className="px-4 py-5 text-sm text-white/55">
              {nniHeartbeatErrorsLoading
                ? t("正在载入 NNI 心跳错误...", "Loading NNI heartbeat errors...")
                : t("当前没有本机心跳错误记录。后续自动心跳失败时会出现在这里。", "There are no local heartbeat error records. Future automatic heartbeat failures will appear here.")}
            </p>
          ) : (
            nniHeartbeatErrors.map((record) => (
              <div key={`${record.id}-${record.created_at_ts ?? 0}`} className="border-t border-white/10 px-4 py-3 first:border-t-0">
                <div className="flex flex-wrap items-center justify-between gap-2">
                  <div className="flex flex-wrap items-center gap-2">
                    <span className="setup-status setup-status-attention">{t("心跳失败", "Heartbeat failed")}</span>
                    <span className="rounded-full border border-white/10 bg-white/[0.04] px-2 py-0.5 text-[11px] font-semibold text-white/55">
                      {record.network ? t("网络错误", "Network") : t("服务端返回", "Server response")}
                    </span>
                    <span className="font-mono text-xs text-white/45">#{record.id}</span>
                  </div>
                  <span className="text-xs text-white/50">{formatUnixDateTime(record.created_at_ts)}</span>
                </div>
                <p className="mt-3 break-words rounded-xl border border-white/10 bg-black/25 px-3 py-2 font-mono text-xs leading-5 text-white/75">
                  {record.error}
                </p>
              </div>
            ))
          )}
        </div>

        <div className="mt-4 flex flex-wrap items-center justify-between gap-3">
          <p className="text-xs text-white/50">
            {t(
              `第 ${nniHeartbeatErrorsPage} / ${nniHeartbeatErrorsTotalPages} 页`,
              `Page ${nniHeartbeatErrorsPage} of ${nniHeartbeatErrorsTotalPages}`,
            )}
          </p>
          <div className="flex items-center gap-2">
            <button
              type="button"
              onClick={() => void onFetchHeartbeatErrors(nniHeartbeatErrorsPage - 1)}
              disabled={!nniHeartbeatErrorsCanPrev || nniHeartbeatErrorsLoading}
              className="theme-secondary-btn px-3 py-2 text-xs disabled:cursor-not-allowed disabled:opacity-50"
            >
              <ChevronLeft className="h-3.5 w-3.5" />
              {t("上一页", "Previous")}
            </button>
            <button
              type="button"
              onClick={() => void onFetchHeartbeatErrors(nniHeartbeatErrorsPage + 1)}
              disabled={!nniHeartbeatErrorsCanNext || nniHeartbeatErrorsLoading}
              className="theme-secondary-btn px-3 py-2 text-xs disabled:cursor-not-allowed disabled:opacity-50"
            >
              {t("下一页", "Next")}
              <ChevronRight className="h-3.5 w-3.5" />
            </button>
          </div>
        </div>
        </div>
        ) : null}

        {nniHistoryView === "records" ? (
          <div
            id="nni-history-records-panel"
            role="tabpanel"
            aria-labelledby="nni-history-records-tab"
            className="mt-5"
          >
          <div className="flex flex-wrap items-start justify-between gap-3">
            <div>
              <p className="theme-kicker text-[10px] uppercase tracking-[0.28em]">
                {t("NNI 请求记录", "NNI request records")}
              </p>
              <h4 className="mt-2 text-lg font-semibold">{t("本机请求记录", "Local request records")}</h4>
              <p className="mt-2 text-sm leading-6 text-white/60">
                {t(
                  `共 ${nniHeartbeatRecordsTotal} 条记录，每页 ${nniHeartbeatRecordsPageSize} 条。`,
                  `${nniHeartbeatRecordsTotal} records total, ${nniHeartbeatRecordsPageSize} per page.`,
                )}
              </p>
              <p className="mt-1 text-xs leading-5 text-white/45">
                {t(
                  "这里保存本机看到的手动加入和自动心跳结果；远端服务端记录不再从 UI 拉取。",
                  "This stores manual Join and automatic Heartbeat results seen by this device. Remote server records are no longer fetched in the UI.",
                )}
              </p>
            </div>
            <div className="flex flex-wrap items-center gap-2">
              <button
                type="button"
                onClick={() => void onClearHeartbeatRecords()}
                disabled={nniHeartbeatRecordsTotal === 0 || nniHeartbeatRecordsLoading || nniHeartbeatRecordsClearing}
                className="theme-secondary-btn px-3 py-2 text-xs disabled:cursor-not-allowed disabled:opacity-50"
              >
                {nniHeartbeatRecordsClearing ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Trash2 className="h-3.5 w-3.5" />}
                {t("清理记录", "Clear records")}
              </button>
              <button
                type="button"
                onClick={() => void onFetchHeartbeatRecords(nniHeartbeatRecordsPage)}
                disabled={nniHeartbeatRecordsLoading}
                className="theme-secondary-btn px-3 py-2 text-xs"
              >
                {nniHeartbeatRecordsLoading ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <RefreshCw className="h-3.5 w-3.5" />}
                {t("刷新记录", "Refresh records")}
              </button>
            </div>
          </div>

          {nniHeartbeatRecordsError ? (
            <p className="mt-3 break-words rounded-xl border border-amber-300/20 bg-amber-300/10 px-3 py-2 text-xs leading-5 text-amber-100">
              {t("NNI 请求记录暂时无法载入：", "NNI request records could not be loaded: ")}
              {nniHeartbeatRecordsError}
            </p>
        ) : null}
        {nniHeartbeatRecordsMessage ? (
          <p className="mt-3 rounded-xl border border-emerald-300/20 bg-emerald-300/10 px-3 py-2 text-xs leading-5 text-emerald-100">
            {nniHeartbeatRecordsMessage}
          </p>
        ) : null}

        <div className="mt-4 overflow-hidden rounded-2xl border border-white/10 bg-black/20">
          {nniHeartbeatRecords.length === 0 ? (
            <p className="px-4 py-5 text-sm text-white/55">
              {nniHeartbeatRecordsLoading
                ? t("正在载入 NNI 请求记录...", "Loading NNI request records...")
                : t(
                    "本机还没有 NNI 请求记录。手动加入和自动心跳的成功或失败结果都会保存在这里。",
                    "This device has no NNI request records yet. Manual Join and automatic Heartbeat successes or failures will be saved here.",
                  )}
            </p>
          ) : (
            nniHeartbeatRecords.map((record) => {
              const complianceKnown = typeof record.compliant === "boolean";
              const accepted = record.status === "accepted" && record.compliant !== false;
              const attention = ["blocked", "rejected", "expired", "failed"].includes(record.status) || record.compliant === false;
              const statusClass = accepted
                ? "setup-status setup-status-done"
                : attention
                  ? "setup-status setup-status-attention"
                  : "setup-status setup-status-todo";
              const statusLabel =
                record.status === "accepted"
                  ? t("已通过", "Accepted")
                  : record.status === "blocked"
                    ? t("已拦截", "Blocked")
                    : record.status === "rejected"
                      ? t("已拒绝", "Rejected")
                      : record.status === "expired"
                        ? t("已过期", "Expired")
                        : record.status === "challenge_created"
                          ? t("挑战已创建", "Challenge created")
                          : record.status === "failed"
                            ? t("失败", "Failed")
                            : record.status || t("未知", "Unknown");
              const kindLabel =
                record.request_kind === "nni_join"
                  ? t("加入", "Join")
                  : record.request_kind === "nni_heartbeat"
                    ? t("心跳", "Heartbeat")
                    : record.request_kind || t("请求", "Request");
              const resultLabel =
                record.error_code ||
                (record.compliant === true
                  ? t("合规", "Compliant")
                  : record.compliant === false
                    ? t("未合规", "Not compliant")
                    : record.status === "challenge_created"
                      ? t("等待签名验证", "Waiting for signature verification")
                      : record.status === "failed"
                        ? t("失败", "Failed")
                        : record.status === "accepted"
                          ? t("已通过", "Accepted")
                          : t("未返回", "Not reported"));
              return (
                <div
                  key={`${record.id ?? record.task_id ?? "heartbeat"}-${record.created_at_ts ?? 0}`}
                  className="border-t border-white/10 px-4 py-3 first:border-t-0"
                >
                  <div className="flex flex-wrap items-center justify-between gap-2">
                    <div className="flex flex-wrap items-center gap-2">
                      <span className={statusClass}>{statusLabel}</span>
                      <span className="rounded-full border border-white/10 bg-white/[0.04] px-2 py-0.5 text-[11px] font-semibold text-white/55">
                        {kindLabel}
                      </span>
                      <span className="font-mono text-xs text-white/45">#{record.id ?? "--"}</span>
                    </div>
                    <span className="text-xs text-white/50">{formatUnixDateTime(record.created_at_ts)}</span>
                  </div>
                  <div className="mt-3 grid gap-3 text-xs sm:grid-cols-3">
                    <div>
                      <p className="font-semibold tracking-[0.12em] text-white/35">{t("公钥", "Public key")}</p>
                      <NniPublicKeyDisplay
                        value={record.device_pubkey}
                        t={t}
                        shorten={{ head: 10, tail: 8 }}
                        className="mt-1"
                        valueClassName="text-xs text-white/75"
                      />
                    </div>
                    <div>
                      <p className="font-semibold tracking-[0.12em] text-white/35">{t("任务", "Task")}</p>
                      <p className="mt-1 break-all font-mono text-white/75" title={record.task_id || ""}>
                        {shortNniValue(record.task_id)}
                      </p>
                    </div>
                    <div>
                      <p className="font-semibold tracking-[0.12em] text-white/35">{t("结果", "Result")}</p>
                      <p className="mt-1 break-words text-white/75">{resultLabel}</p>
                    </div>
                  </div>
                  {!complianceKnown && record.status !== "accepted" && !record.error_code ? (
                    <p className="mt-2 text-xs leading-5 text-white/40">
                      {t(
                        "这条记录没有合规结果；请以状态标签和错误码为准。",
                        "This record has no compliance result; use the status label and error code.",
                      )}
                    </p>
                  ) : null}
                  <p className="mt-2 text-xs leading-5 text-white/40">
                    {t("签名", "Signature")}: {record.signature_present ? t("已记录", "Recorded") : t("无", "None")} ·{" "}
                    {t("挑战", "Challenge")}: {record.challenge_present ? t("已记录", "Recorded") : t("无", "None")} ·{" "}
                    {t("节点", "Node")}: <span className="font-mono">{shortNniValue(record.node_url)}</span> ·{" "}
                    {t("用户", "User")}: <span className="font-mono">{shortNniValue(record.user_key)}</span>
                  </p>
                </div>
              );
            })
          )}
        </div>

        <div className="mt-4 flex flex-wrap items-center justify-between gap-3">
          <p className="text-xs text-white/50">
            {t(
              `第 ${nniHeartbeatRecordsPage} / ${nniHeartbeatRecordsTotalPages} 页`,
              `Page ${nniHeartbeatRecordsPage} of ${nniHeartbeatRecordsTotalPages}`,
            )}
          </p>
          <div className="flex items-center gap-2">
            <button
              type="button"
              onClick={() => void onFetchHeartbeatRecords(nniHeartbeatRecordsPage - 1)}
              disabled={!nniHeartbeatRecordsCanPrev || nniHeartbeatRecordsLoading}
              className="theme-secondary-btn px-3 py-2 text-xs disabled:cursor-not-allowed disabled:opacity-50"
            >
              <ChevronLeft className="h-3.5 w-3.5" />
              {t("上一页", "Previous")}
            </button>
            <button
              type="button"
              onClick={() => void onFetchHeartbeatRecords(nniHeartbeatRecordsPage + 1)}
              disabled={!nniHeartbeatRecordsCanNext || nniHeartbeatRecordsLoading}
              className="theme-secondary-btn px-3 py-2 text-xs disabled:cursor-not-allowed disabled:opacity-50"
            >
              {t("下一页", "Next")}
              <ChevronRight className="h-3.5 w-3.5" />
            </button>
          </div>
        </div>
        </div>
        ) : null}
        </section>
      ) : null}

      {nniHistoryView === "overview" ? (
        <section
          id="nni-overview-actions-panel"
          role="tabpanel"
          aria-labelledby="nni-overview-tab"
          className="grid gap-4 xl:grid-cols-[0.9fr_1.1fr]"
        >
        <div className="theme-panel-soft p-5">
          <div className="flex items-start gap-3">
            <Fingerprint className="mt-0.5 h-5 w-5 shrink-0 theme-icon-soft" />
            <div>
              <h4 className="text-lg font-semibold">{t("设备签名操作", "Device signing actions")}</h4>
              <p className="mt-2 text-sm leading-7 text-white/65">
                {t(
                  "这些操作对应 Pi App 已预埋的 helper：slot 0 公钥、时间戳签名、TNG 设备公钥和证书链。",
                  "These actions map to the helper already built into the Pi App: Slot 0 public key, timestamp signing, TNG device public key, and certificate chain.",
                )}
              </p>
            </div>
          </div>

          <div className="mt-4 grid gap-2">
            {NNI_DEVICE_ACTIONS.map((action) => (
              <button
                key={action}
                type="button"
                onClick={() => void onRunDeviceAction(action)}
                disabled={Boolean(nniActionLoading) || nniStatusLoading || !nniChipPresent}
                className="theme-topbar-btn justify-between px-3 py-2 text-sm disabled:cursor-not-allowed disabled:opacity-50"
                title={
                  !nniChipPresent
                    ? nniDetectionUnavailable
                      ? t("芯片检测暂时未完成，请刷新状态后重试。", "Chip detection is temporarily unavailable. Refresh the status and retry.")
                      : t("当前设备缺少芯片，不能执行该操作。", "This device has no chip, so this action cannot run.")
                    : undefined
                }
              >
                <span className="inline-flex items-center gap-2">
                  {nniActionLoading === action ? <Loader2 className="h-4 w-4 animate-spin" /> : <Cpu className="h-4 w-4" />}
                  {actionLabel(action)}
                </span>
                <span className="font-mono text-xs text-white/45">{action}</span>
              </button>
            ))}
          </div>
        </div>

        <div className="theme-panel-soft p-5">
          <div className="flex flex-wrap items-start justify-between gap-3">
            <div>
              <h4 className="text-lg font-semibold">{t("最近一次结果", "Latest result")}</h4>
              <p className="mt-2 text-sm text-white/60">
                {nniActionResult
                  ? actionLabel(nniActionResult.action)
                  : t("执行一个设备签名操作后，这里会显示返回值。", "Run a device signing action to show its result here.")}
              </p>
            </div>
            {nniPrimaryHex && nniPrimaryHex.label !== "pubkey" ? (
              <button type="button" onClick={() => copyPrimaryHex()} className="theme-secondary-btn px-3 py-2 text-xs">
                <Copy className="h-4 w-4" />
                {t("复制", "Copy")}
              </button>
            ) : null}
          </div>

          {nniActionMessage ? (
            <p className="mt-4 rounded-xl border border-emerald-500/25 bg-emerald-500/10 px-3 py-2 text-sm text-emerald-100">
              {nniActionMessage}
            </p>
          ) : null}
          {nniActionError ? (
            <p className="mt-4 rounded-xl border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-100">
              {nniActionError}
            </p>
          ) : null}

          {nniPrimaryHex?.label === "pubkey" && nniActionResult?.payload?.pubkey ? (
            <div className="mt-4 rounded-xl border border-white/10 bg-black/20 p-3">
              <p className="text-xs font-semibold text-white/75">pubkey</p>
              <NniPublicKeyDisplay
                value={nniActionResult.payload.pubkey}
                t={t}
                showByteSize
                onCopy={copyPrimaryHex}
                className="mt-3"
                valueClassName="text-xs leading-6 text-white/75"
              />
            </div>
          ) : nniPrimaryHex ? (
            <div className="mt-4 rounded-xl border border-white/10 bg-black/20 p-3">
              <div className="flex flex-wrap items-center justify-between gap-2">
                <p className="text-xs font-semibold text-white/75">{nniPrimaryHex.label}</p>
                {nniPrimaryHex.size != null ? (
                  <span className="rounded-full border border-white/10 bg-white/5 px-2 py-1 text-[11px] text-white/55">
                    {nniPrimaryHex.size} bytes
                  </span>
                ) : null}
              </div>
              <p className="mt-3 break-all font-mono text-xs leading-6 text-white/75">
                {shortenHex(nniPrimaryHex.value, 48, 48)}
              </p>
            </div>
          ) : null}

          {nniActionResult?.payload?.timestamp ? (
            <div className="mt-3 rounded-xl border border-white/10 bg-black/20 px-3 py-3">
              <p className="text-[11px] tracking-[0.14em] text-white/45">{t("签名时间", "Signed timestamp")}</p>
              <p className="mt-2 font-mono text-sm text-white/85">{nniActionResult.payload.timestamp}</p>
            </div>
          ) : null}

          {nniActionResult ? (
            <details className="mt-4 rounded-xl border border-white/10 bg-black/20 p-3">
              <summary className="cursor-pointer text-sm font-medium text-white/75">
                {t("查看原始 JSON", "View raw JSON")}
              </summary>
              <pre className="mt-3 max-h-72 overflow-auto whitespace-pre-wrap break-words rounded-lg bg-black/30 p-3 text-xs leading-5 text-white/65">
                {JSON.stringify(nniActionResult.payload ?? nniActionResult, null, 2)}
              </pre>
            </details>
          ) : null}
        </div>
        </section>
      ) : null}
    </div>
  );
}
