import { Activity, CalendarClock, Coins, Globe2, Loader2, TimerReset, Users } from "lucide-react";

import type {
  NniNetworkDeviceStats as NniNetworkDeviceStatsValue,
  NniNetworkRewards,
  NniRewardPolicy,
} from "../types/api";
import { NniDecimalAmount } from "./NniDecimalAmount";

type Translate = (zh: string, en: string) => string;

export function formatNniRewardMetric(value: string | number): string {
  const text = String(value).trim();
  return text.replace(/\.0+$/, "");
}

export interface NniNetworkDeviceStatsProps {
  stats: NniNetworkDeviceStatsValue | null;
  networkRewards?: NniNetworkRewards | null;
  rewardPolicy?: NniRewardPolicy | null;
  localPreviousRewardAic?: string | null;
  localRewardLoading?: boolean;
  loading: boolean;
  t: Translate;
  formatUnixDateTime: (ts: number | null | undefined) => string;
}

export function NniNetworkDeviceStats({
  stats,
  networkRewards = null,
  rewardPolicy = null,
  localPreviousRewardAic = null,
  localRewardLoading = false,
  loading,
  t,
  formatUnixDateTime,
}: NniNetworkDeviceStatsProps) {
  const unavailableLabel = t("暂不可用", "Unavailable");
  const registeredValue = stats?.registered_device_count ?? unavailableLabel;
  const activeValue = stats?.active_device_count ?? unavailableLabel;
  const networkOutputValue = networkRewards?.total_distributed_reward_aic
    ? formatNniRewardMetric(networkRewards.total_distributed_reward_aic)
    : unavailableLabel;
  const rewardPoolValue = rewardPolicy?.current_reward_pool_aic
    ? formatNniRewardMetric(rewardPolicy.current_reward_pool_aic)
    : unavailableLabel;
  const localPreviousRewardValue = localPreviousRewardAic
    ? formatNniRewardMetric(localPreviousRewardAic)
    : unavailableLabel;
  const firstHeartbeatValue = stats?.first_heartbeat_unix != null
    ? formatUnixDateTime(stats.first_heartbeat_unix)
    : stats
      ? t("等待首跳", "Waiting for first heartbeat")
      : unavailableLabel;
  const nextHalvingValue = rewardPolicy?.rewards_ended
    ? t("奖励已结束", "Rewards ended")
    : rewardPolicy?.next_halving_at_unix != null
      ? formatUnixDateTime(rewardPolicy.next_halving_at_unix)
      : rewardPolicy && rewardPolicy.halving_epoch_unix == null
        ? t("首跳后计算", "Calculated after first heartbeat")
        : unavailableLabel;

  return (
    <div className="grid w-full gap-2.5 sm:grid-cols-2 md:grid-cols-3 xl:grid-cols-6" aria-label={t("网络概览", "Network overview")}>
      <div className="min-w-0 rounded-lg border border-white/10 bg-black/15 px-3 py-2.5">
        <div className="flex items-center justify-between gap-2">
          <div className="flex min-w-0 items-center gap-2 text-white/55">
            <Users className="h-4 w-4" />
            <span className="text-xs font-semibold">{t("注册设备", "Registered devices")}</span>
          </div>
          <p className={stats ? "shrink-0 text-xl font-semibold text-white/90" : "text-sm font-semibold text-white/75"}>
            {loading && !stats ? <Loader2 className="h-5 w-5 animate-spin" /> : registeredValue}
          </p>
        </div>
      </div>

      <div className="min-w-0 rounded-lg border border-white/10 bg-black/15 px-3 py-2.5">
        <div className="flex items-center justify-between gap-2">
          <div className="flex min-w-0 items-center gap-2 text-white/55">
            <Activity className="h-4 w-4" />
            <span className="text-xs font-semibold">{t("活跃设备", "Active devices")}</span>
          </div>
          <p className={stats ? "shrink-0 text-xl font-semibold text-white/90" : "text-sm font-semibold text-white/75"}>
            {loading && !stats ? <Loader2 className="h-5 w-5 animate-spin" /> : activeValue}
          </p>
        </div>
      </div>

      <div className="min-w-0 rounded-lg border border-white/10 bg-black/15 px-3 py-2.5">
        <div className="flex items-start justify-between gap-2">
          <div className="flex min-w-0 items-center gap-2 text-white/55">
            <Globe2 className="h-4 w-4" />
            <span className="text-xs font-semibold">{t("累计产出", "Total output")}</span>
          </div>
          <p className={networkRewards ? "min-w-0 break-all text-right font-mono text-base font-semibold text-white/90" : "text-sm font-semibold text-white/75"}>
            {loading && !networkRewards ? <Loader2 className="h-5 w-5 animate-spin" /> : <NniDecimalAmount value={String(networkOutputValue)} />}
          </p>
        </div>
      </div>

      <div className="min-w-0 rounded-lg border border-white/10 bg-black/15 px-3 py-2.5">
        <div className="flex items-center gap-2 text-white/55">
          <Coins className="h-4 w-4" />
          <span className="text-xs font-semibold">{t("窗口奖励", "Window reward")}</span>
        </div>
        <div className="mt-1.5 grid min-w-0 grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)] items-end gap-1">
          <div className="min-w-0">
            <p className="text-[9px] leading-3 text-white/40">{t("总奖励", "Total")}</p>
            <p className={rewardPolicy ? "min-w-0 break-all font-mono text-sm font-semibold text-white/90" : "text-xs font-semibold text-white/75"}>
              {loading && !rewardPolicy ? <Loader2 className="h-4 w-4 animate-spin" /> : <NniDecimalAmount value={String(rewardPoolValue)} shrinkFraction={false} />}
            </p>
          </div>
          <span className="pb-0.5 text-xs text-white/25">/</span>
          <div className="min-w-0 text-right">
            <p className="text-[9px] leading-3 text-white/40">{t("本机上个窗口", "Local previous")}</p>
            <p className={localPreviousRewardAic ? "min-w-0 break-all font-mono text-sm font-semibold text-white/90" : "text-xs font-semibold text-white/75"}>
              {localRewardLoading && !localPreviousRewardAic ? <Loader2 className="ml-auto h-4 w-4 animate-spin" /> : <NniDecimalAmount value={String(localPreviousRewardValue)} shrinkFraction={false} />}
            </p>
          </div>
        </div>
      </div>

      <div className="min-w-0 rounded-lg border border-white/10 bg-black/15 px-3 py-2.5">
        <div className="flex items-center gap-2 text-white/55">
          <CalendarClock className="h-4 w-4" />
          <span className="text-xs font-semibold">{t("首跳", "First heartbeat")}</span>
        </div>
        <p className={stats?.first_heartbeat_unix != null ? "mt-1.5 whitespace-nowrap text-xs font-medium text-white/90" : "mt-1.5 text-xs font-medium text-white/75"}>
          {loading && !stats ? <Loader2 className="h-5 w-5 animate-spin" /> : firstHeartbeatValue}
        </p>
      </div>

      <div className="min-w-0 rounded-lg border border-white/10 bg-black/15 px-3 py-2.5">
        <div className="flex items-center gap-2 text-white/55">
          <TimerReset className="h-4 w-4" />
          <span className="text-xs font-semibold">{t("减半", "Halving")}</span>
        </div>
        <p className={rewardPolicy?.next_halving_at_unix != null ? "mt-1.5 whitespace-nowrap text-xs font-medium text-white/90" : "mt-1.5 text-xs font-medium text-white/75"}>
          {loading && !rewardPolicy ? <Loader2 className="h-5 w-5 animate-spin" /> : nextHalvingValue}
        </p>
      </div>
    </div>
  );
}
