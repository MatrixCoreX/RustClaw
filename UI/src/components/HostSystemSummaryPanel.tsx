import {
  Box,
  Cpu,
  HardDrive,
  Loader2,
  MemoryStick,
  RefreshCw,
  Server,
} from "lucide-react";
import type { ReactNode } from "react";

import { formatBytes, formatDuration } from "../lib/display-format";
import {
  hostCapacityUsedPercent,
  hostSummaryIsPartial,
  hostSummaryIsStale,
  hostSystemTitle,
} from "../lib/host-system";
import type { HostCapacitySummary, HostSystemSummary } from "../types/api";

type Translate = (zh: string, en: string) => string;

interface HostSystemSummaryPanelProps {
  t: Translate;
  summary: HostSystemSummary | null;
  loading: boolean;
  errorCode: string | null;
  onRefresh: () => unknown | Promise<unknown>;
}

function ResourceProgress({
  t,
  label,
  capacity,
  icon,
}: {
  t: Translate;
  label: string;
  capacity: HostCapacitySummary;
  icon: ReactNode;
}) {
  const usedPercent = hostCapacityUsedPercent(capacity);
  return (
    <div className="min-w-0 py-3">
      <div className="flex items-center gap-2 text-white/55">
        {icon}
        <span className="text-xs font-medium">{label}</span>
      </div>
      <p className="mt-2 text-base font-semibold text-white/92">
        {formatBytes(capacity.available_bytes)}
        <span className="ml-1 text-xs font-normal text-white/45">
          {t("可用", "available")}
        </span>
      </p>
      <p className="mt-1 text-xs text-white/48">
        {t("总计", "Total")} {formatBytes(capacity.total_bytes)}
      </p>
      <div
        className="mt-3 h-1.5 overflow-hidden rounded-full bg-white/8"
        role={usedPercent == null ? undefined : "progressbar"}
        aria-label={label}
        aria-valuemin={usedPercent == null ? undefined : 0}
        aria-valuemax={usedPercent == null ? undefined : 100}
        aria-valuenow={usedPercent ?? undefined}
      >
        <div
          className="h-full rounded-full bg-emerald-400/70 transition-[width]"
          style={{ width: `${usedPercent ?? 0}%` }}
        />
      </div>
      <p className="mt-1.5 text-[11px] text-white/42">
        {usedPercent == null ? t("数据暂不可用", "Data unavailable") : t(`已使用 ${usedPercent}%`, `${usedPercent}% used`)}
      </p>
    </div>
  );
}

function hostErrorLabel(t: Translate, code: string): string {
  switch (code) {
    case "permission_denied":
      return t("当前账号无权查看系统信息。", "This account cannot view system information.");
    case "disconnected":
      return t("暂时无法连接 RustClaw。", "RustClaw is temporarily unreachable.");
    default:
      return t("系统信息暂不可用。", "System information is temporarily unavailable.");
  }
}

export function HostSystemSummaryPanel({
  t,
  summary,
  loading,
  errorCode,
  onRefresh,
}: HostSystemSummaryPanelProps) {
  const stale = hostSummaryIsStale(summary);
  const partial = hostSummaryIsPartial(summary);
  const deploymentLabel =
    summary?.deployment === "container"
      ? t("容器", "Container")
      : summary?.deployment === "host"
        ? t("主机", "Host")
        : summary?.deployment === "local_host"
          ? t("本地设备", "Local device")
          : null;

  return (
    <section className="theme-panel-soft px-4 py-4 sm:px-5">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <p className="theme-kicker text-[10px] uppercase tracking-[0.2em]">
            {t("运行设备", "Running Device")}
          </p>
          <h3 className="mt-1.5 text-base font-semibold text-white">
            {t("系统信息", "System information")}
          </h3>
        </div>
        <div className="flex items-center gap-2">
          {stale ? (
            <span className="text-xs text-amber-200">{t("数据需要刷新", "Refresh recommended")}</span>
          ) : partial ? (
            <span className="text-xs text-white/50">{t("部分数据不可用", "Some data unavailable")}</span>
          ) : null}
          <button
            type="button"
            onClick={() => void onRefresh()}
            disabled={loading}
            className="theme-icon-btn"
            title={t("刷新系统信息", "Refresh system information")}
            aria-label={t("刷新系统信息", "Refresh system information")}
          >
            {loading ? <Loader2 className="h-4 w-4 animate-spin" /> : <RefreshCw className="h-4 w-4" />}
          </button>
        </div>
      </div>

      {errorCode && !summary ? (
        <div className="mt-4 flex items-center gap-3 border-t border-white/8 py-5 text-sm text-white/60">
          <Server className="h-5 w-5 shrink-0" />
          <span>{hostErrorLabel(t, errorCode)}</span>
        </div>
      ) : loading && !summary ? (
        <div className="mt-4 grid animate-pulse gap-4 border-t border-white/8 pt-4 sm:grid-cols-2 lg:grid-cols-4">
          {[0, 1, 2, 3].map((item) => (
            <div key={item} className="h-20 rounded-md bg-white/5" />
          ))}
        </div>
      ) : summary ? (
        <div className="mt-4 grid gap-x-5 border-t border-white/8 sm:grid-cols-2 lg:grid-cols-4">
          <div className="min-w-0 py-3">
            <div className="flex items-center gap-2 text-white/55">
              <Cpu className="h-4 w-4" />
              <span className="text-xs font-medium">{t("系统", "System")}</span>
            </div>
            <p className="mt-2 break-words text-base font-semibold text-white/92">
              {hostSystemTitle(summary)}
            </p>
            <p className="mt-1 break-all text-xs text-white/48">
              {summary.architecture}
              {summary.os.kernel ? ` · ${summary.os.kernel}` : ""}
            </p>
          </div>

          <ResourceProgress
            t={t}
            label={t("内存", "Memory")}
            capacity={summary.memory}
            icon={<MemoryStick className="h-4 w-4" />}
          />
          <ResourceProgress
            t={t}
            label={t("RustClaw 存储", "RustClaw storage")}
            capacity={summary.storage}
            icon={<HardDrive className="h-4 w-4" />}
          />

          <div className="min-w-0 py-3">
            <div className="flex items-center gap-2 text-white/55">
              <Box className="h-4 w-4" />
              <span className="text-xs font-medium">{t("运行环境", "Environment")}</span>
            </div>
            <p className="mt-2 text-base font-semibold text-white/92">
              {deploymentLabel || t("未识别", "Not identified")}
            </p>
            <p className="mt-1 text-xs text-white/48">
              {t("已运行", "Uptime")} {formatDuration(summary.uptime_seconds ?? undefined)}
            </p>
          </div>
        </div>
      ) : null}

      {errorCode && summary ? (
        <p className="mt-2 text-xs text-amber-200/80">
          {t("刷新失败，当前显示上一次成功获取的数据。", "Refresh failed; showing the last successful data.")}
        </p>
      ) : null}
    </section>
  );
}
