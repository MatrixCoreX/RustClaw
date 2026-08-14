import { Activity, Loader2, Users } from "lucide-react";

import type { NniNetworkDeviceStats as NniNetworkDeviceStatsValue } from "../types/api";

type Translate = (zh: string, en: string) => string;

export interface NniNetworkDeviceStatsProps {
  stats: NniNetworkDeviceStatsValue | null;
  loading: boolean;
  joined: boolean;
  t: Translate;
  formatUnixDateTime: (ts: number | null | undefined) => string;
}

export function NniNetworkDeviceStats({
  stats,
  loading,
  joined,
  t,
  formatUnixDateTime,
}: NniNetworkDeviceStatsProps) {
  const unavailableLabel = joined
    ? t("暂不可用", "Unavailable")
    : t("未加入", "Not joined");
  const activePeriod = !joined
    ? t("加入网络后可查看", "Join the network to view")
    : stats?.active_period_start_unix != null && stats.active_period_end_unix != null
      ? `${formatUnixDateTime(stats.active_period_start_unix)} – ${formatUnixDateTime(stats.active_period_end_unix)}`
      : stats
        ? t("等待首个窗口结算", "Waiting for the first settled window")
        : t("刷新状态后重试", "Refresh status to retry");
  const registeredValue = stats?.registered_device_count ?? unavailableLabel;
  const activeValue = stats?.active_device_count ?? unavailableLabel;

  return (
    <div className="grid w-full gap-3 sm:grid-cols-2" aria-label={t("网络设备概览", "Network device overview")}>
      <div className="rounded-2xl border border-white/10 bg-black/15 px-4 py-3">
        <div className="flex items-center justify-between gap-3">
          <div className="flex items-center gap-2 text-white/55">
            <Users className="h-4 w-4" />
            <span className="text-xs font-semibold">{t("注册设备", "Registered devices")}</span>
          </div>
          <p className={stats ? "text-2xl font-semibold text-white/90" : "text-sm font-semibold text-white/75"}>
            {loading && !stats ? <Loader2 className="h-5 w-5 animate-spin" /> : registeredValue}
          </p>
        </div>
      </div>

      <div className="rounded-2xl border border-white/10 bg-black/15 px-4 py-3">
        <div className="flex items-center justify-between gap-3">
          <div className="flex items-center gap-2 text-white/55">
            <Activity className="h-4 w-4" />
            <span className="text-xs font-semibold">{t("活跃设备", "Active devices")}</span>
          </div>
          <p className={stats ? "text-2xl font-semibold text-white/90" : "text-sm font-semibold text-white/75"}>
            {loading && !stats ? <Loader2 className="h-5 w-5 animate-spin" /> : activeValue}
          </p>
        </div>
        <p className="mt-1 text-[11px] text-white/35">{activePeriod}</p>
      </div>
    </div>
  );
}
