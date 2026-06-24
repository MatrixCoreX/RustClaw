import {
  ChevronLeft,
  ChevronRight,
  Copy,
  Cpu,
  Fingerprint,
  KeyRound,
  Loader2,
  Network,
  RefreshCw,
  ShieldAlert,
  ShieldCheck,
  Trash2,
} from "lucide-react";
import { useEffect, useRef, useState } from "react";

import { writeTextToClipboard } from "../lib/auth-keys";
import {
  NNI_RUNTIME_TILES,
  nniActionLabel,
  nniPayloadHexField,
  shortenHex,
  shortNniValue,
} from "../lib/nni-display";
import type {
  NniDeviceActionResponse,
  NniDeviceStatusResponse,
  NniHeartbeatErrorRecord,
  NniHeartbeatRecord,
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
  nniJoined: boolean;
  nniRemoteNodes: string;
  nniRemoteNodeCount: number;
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
  onFetchHeartbeatRecords: (page: number) => unknown | Promise<unknown>;
  onClearHeartbeatRecords: () => unknown | Promise<unknown>;
  onFetchHeartbeatErrors: (page: number) => unknown | Promise<unknown>;
  onClearHeartbeatErrors: () => unknown | Promise<unknown>;
  onRunDeviceAction: (action: string) => unknown | Promise<unknown>;
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
  nniJoined,
  nniRemoteNodes,
  nniRemoteNodeCount,
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
  onFetchHeartbeatRecords,
  onClearHeartbeatRecords,
  onFetchHeartbeatErrors,
  onClearHeartbeatErrors,
  onRunDeviceAction,
  onActionMessageChange,
  onActionErrorChange,
}: NniPageProps) {
  const [nniTestJoinPulse, setNniTestJoinPulse] = useState(false);
  const nniTestJoinPulseTimer = useRef<number | null>(null);
  const nniChipPresent = nniStatus?.signature_chip_present === true;
  const nniChipMissing = nniStatus?.signature_chip_present === false;
  const nniPrimaryHex = nniPayloadHexField(nniActionResult?.payload);
  const nniHeartbeatRecordsCanPrev = nniHeartbeatRecordsPage > 1;
  const nniHeartbeatRecordsCanNext = nniHeartbeatRecordsPage < nniHeartbeatRecordsTotalPages;
  const nniHeartbeatErrorsCanPrev = nniHeartbeatErrorsPage > 1;
  const nniHeartbeatErrorsCanNext = nniHeartbeatErrorsPage < nniHeartbeatErrorsTotalPages;
  const actionLabel = (action: string) => nniActionLabel(action, lang);
  const nniRuntimeActivity =
    nniJoined || nniTestJoinPulse || ["join_nni", "sign_challenge", "sign_timestamp"].includes(nniActionLoading || "");

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
    try {
      await Promise.resolve(onTestJoin());
    } finally {
      nniTestJoinPulseTimer.current = window.setTimeout(() => {
        setNniTestJoinPulse(false);
        nniTestJoinPulseTimer.current = null;
      }, NNI_TEST_JOIN_ACTIVITY_MS);
    }
  };

  const copyPrimaryHex = () => {
    if (!nniPrimaryHex) return;
    void writeTextToClipboard(nniPrimaryHex.value)
      .then(() => onActionMessageChange(t("已复制结果。", "Result copied.")))
      .catch((err) => onActionErrorChange(err instanceof Error ? err.message : t("复制失败", "Copy failed")));
  };

  return (
    <div className="space-y-4">
      <section className="theme-panel p-5 sm:p-6">
        <div className="flex flex-col gap-5 xl:flex-row xl:items-start xl:justify-between">
          <div className="max-w-3xl">
            <p className="theme-kicker text-[10px] uppercase tracking-[0.35em]">Network Native Intelligence</p>
            <h3 className="mt-2 flex items-center gap-2 text-xl font-semibold tracking-tight sm:text-2xl">
              <Network className="h-6 w-6 theme-icon-accent" />
              <span>{t("NNI 网络原生智能", "NNI Network-Native Intelligence")}</span>
            </h3>
            <p className="mt-3 text-sm leading-7 text-white/70">
              {t(
                "这里管理 Pi App 里的 NNI 入口和设备签名能力。普通设备可以只查看状态；带安全芯片的设备可以读取公钥、生成时间戳签名，并查看 TNG 证书链。加入时，本机公钥必须是白名单合规公钥。",
                "This page manages the NNI entry from the Pi App and device signing. Regular devices can simply check status; devices with a secure chip can read the public key, create timestamp signatures, and inspect the TNG certificate chain. To join, the local public key must be compliant with the whitelist.",
              )}
            </p>
          </div>

          <div className="flex flex-wrap gap-2">
            <button
              type="button"
              onClick={() => void onFetchDeviceStatus()}
              disabled={nniStatusLoading}
              className="theme-secondary-btn px-3 py-2 text-sm"
            >
              {nniStatusLoading ? <Loader2 className="h-4 w-4 animate-spin" /> : <RefreshCw className="h-4 w-4" />}
              {t("刷新状态", "Refresh status")}
            </button>
            <button
              type="button"
              onClick={() => (nniJoined ? void onSetJoinedPersisted(false) : void onJoin())}
              disabled={Boolean(nniActionLoading) || nniStatusLoading || nniChipMissing || (!nniJoined && nniRemoteNodeCount === 0)}
              className={nniJoined ? "theme-secondary-btn px-3 py-2 text-sm" : "theme-accent-btn px-3 py-2 text-sm"}
              title={
                nniChipMissing
                  ? t("当前设备缺少签名芯片，不能加入需要设备签名的 NNI。", "This device has no signature chip, so it cannot join signed NNI.")
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
                        "上次检测未找到签名芯片；测试加入会重新尝试本机时间戳签名，不请求远程 NNI 服务端。",
                        "The last check did not find a signature chip. Test Join retries a local timestamp signature and does not contact the remote NNI server.",
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
      </section>

      {nniStatusError ? (
        <p className="rounded-2xl border border-red-500/30 bg-red-500/10 px-4 py-3 text-sm text-red-100">
          {nniStatusError}
        </p>
      ) : null}

      <section className="grid gap-4 xl:grid-cols-[0.95fr_1.05fr]">
        <div className="theme-panel-soft p-5">
          <div className="flex items-start justify-between gap-3">
            <div>
              <p className="theme-kicker text-[10px] uppercase tracking-[0.28em]">{t("设备状态", "Device status")}</p>
              <h4 className="mt-2 text-lg font-semibold">{t("设备签名芯片", "Device signature chip")}</h4>
            </div>
            <span
              className={
                nniStatusLoading
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
              ) : nniChipPresent ? (
                <>
                  <ShieldCheck className="h-3.5 w-3.5" />
                  {t("可用", "Ready")}
                </>
              ) : nniStatus == null ? (
                t("未检测", "Not checked")
              ) : (
                <>
                  <ShieldAlert className="h-3.5 w-3.5" />
                  {t("缺失签名芯片", "Signature chip missing")}
                </>
              )}
            </span>
          </div>

          <div
            className={
              nniChipPresent
                ? "mt-4 rounded-xl border border-emerald-500/25 bg-emerald-500/10 px-3 py-3 text-sm text-emerald-100"
                : "mt-4 rounded-xl border border-amber-500/30 bg-amber-500/10 px-3 py-3 text-sm text-amber-100"
            }
          >
            <p className="font-medium">
              {nniStatus?.message ||
                (nniStatusLoading
                  ? t("正在读取签名芯片状态。", "Reading signature chip status.")
                  : t("还没有读取状态。点击刷新状态开始检测。", "Status has not been loaded yet. Click Refresh status to check."))}
            </p>
            {nniStatus?.next_step ? <p className="mt-1 text-sm opacity-80">{nniStatus.next_step}</p> : null}
            {nniStatus?.error ? <p className="mt-2 break-words font-mono text-xs opacity-75">{nniStatus.error}</p> : null}
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
                "例如：https://nni-node.example.com\n多个节点可以一行一个，系统会按顺序尝试。",
                "Example: https://nni-node.example.com\nUse one node per line. The system will try them in order.",
              )}
              value={nniRemoteNodes}
              onChange={(event) => onRemoteNodesChange(event.target.value)}
            />
            <p className="mt-2 text-xs leading-5 text-white/50">
              {t(
                "远程节点会保存到 configs/config.toml；加入成功后运行状态也会保存，clawd 重启或页面重开后会自动载入。远程节点负责下发 challenge、验签并记录合规请求。本机公钥必须是白名单合规公钥。",
                "Remote nodes are saved to configs/config.toml. After Join succeeds, the runtime state is saved too and loads automatically after clawd restarts or the page reopens. Remote nodes issue challenges, verify signatures, and record compliant requests. The local public key must be compliant with the whitelist.",
              )}
            </p>
            {nniConfigMessage ? <p className="mt-2 text-xs text-emerald-200">{nniConfigMessage}</p> : null}
            {nniConfigError ? <p className="mt-2 break-words text-xs text-red-200">{nniConfigError}</p> : null}
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
            {nniChipMissing
              ? t(
                  "当前设备缺少签名芯片，因此不会显示为已加入。你仍可以继续使用 RustClaw 的其它功能。",
                  "This device has no signature chip, so it will not be marked as joined. Other RustClaw features remain available.",
                )
              : nniJoined
                ? t(
                    "服务端已验证设备签名，NNI 运行入口已开启。clawd 会每 15 分钟向服务器发送一次硬件签名心跳。",
                    "The server verified the device signature, and the NNI runtime entry is active. clawd will send a hardware-signed heartbeat to the server every 15 minutes.",
                  )
                : t(
                    "点击加入会向远程服务端请求一次随机挑战。本机公钥必须是白名单合规公钥，验签通过后开启运行入口；测试加入只做本机时间戳签名，不请求远程服务端。",
                    "Click Join to request a random challenge from the remote server. The local public key must be compliant with the whitelist, and the runtime is enabled after verification. Test Join only signs a local timestamp and does not contact the remote server.",
                  )}
          </p>
        </div>
      </section>

      <section className="theme-panel-soft p-5">
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
      </section>

      <section className="theme-panel-soft p-5">
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
                      <p className="mt-1 break-all font-mono text-white/75" title={record.device_pubkey || ""}>
                        {shortNniValue(record.device_pubkey)}
                      </p>
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
      </section>

      <section className="grid gap-4 xl:grid-cols-[0.9fr_1.1fr]">
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
                disabled={Boolean(nniActionLoading) || nniStatusLoading || nniChipMissing}
                className="theme-topbar-btn justify-between px-3 py-2 text-sm disabled:cursor-not-allowed disabled:opacity-50"
                title={
                  nniChipMissing
                    ? t("当前设备缺少签名芯片，不能执行该操作。", "This device has no signature chip, so this action cannot run.")
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
            {nniPrimaryHex ? (
              <button type="button" onClick={copyPrimaryHex} className="theme-secondary-btn px-3 py-2 text-xs">
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

          {nniPrimaryHex ? (
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
    </div>
  );
}
