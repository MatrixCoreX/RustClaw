import { Activity, CalendarClock, Coins, Globe2, Loader2, TimerReset, Users } from "lucide-react";

import type {
  NniNetworkDeviceStats as NniNetworkDeviceStatsValue,
  NniNetworkRewards,
  NniRewardPolicy,
} from "../types/api";

type Translate = (zh: string, en: string) => string;

export interface NniNetworkDeviceStatsProps {
  stats: NniNetworkDeviceStatsValue | null;
  networkRewards?: NniNetworkRewards | null;
  rewardPolicy?: NniRewardPolicy | null;
  loading: boolean;
  joined: boolean;
  t: Translate;
  formatUnixDateTime: (ts: number | null | undefined) => string;
}

export function NniNetworkDeviceStats({
  stats,
  networkRewards = null,
  rewardPolicy = null,
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
  const networkOutputValue = networkRewards?.total_distributed_reward_points ?? unavailableLabel;
  const rewardPoolValue = rewardPolicy?.current_reward_pool_points
    ? `${rewardPolicy.current_reward_pool_points} POINT`
    : unavailableLabel;
  const firstHeartbeatValue = stats?.first_heartbeat_unix != null
    ? formatUnixDateTime(stats.first_heartbeat_unix)
    : unavailableLabel;
  const nextHalvingValue = rewardPolicy?.rewards_ended
    ? t("奖励已结束", "Rewards ended")
    : rewardPolicy?.next_halving_at_unix != null
      ? formatUnixDateTime(rewardPolicy.next_halving_at_unix)
      : unavailableLabel;

  return (
    <div className="grid w-full gap-3 sm:grid-cols-2" aria-label={t("网络概览", "Network overview")}>
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

      <div className="rounded-2xl border border-white/10 bg-black/15 px-4 py-3">
        <div className="flex items-center justify-between gap-3">
          <div className="flex items-center gap-2 text-white/55">
            <Globe2 className="h-4 w-4" />
            <span className="text-xs font-semibold">{t("全网累计产出", "Network-wide output")}</span>
          </div>
          <p className={networkRewards ? "font-mono text-lg font-semibold text-white/90" : "text-sm font-semibold text-white/75"}>
            {loading && !networkRewards ? <Loader2 className="h-5 w-5 animate-spin" /> : networkOutputValue}
          </p>
        </div>
        <p className="mt-1 text-[11px] text-white/35">POINT</p>
      </div>

      <div className="rounded-2xl border border-white/10 bg-black/15 px-4 py-3">
        <div className="flex items-center justify-between gap-3">
          <div className="flex items-center gap-2 text-white/55">
            <Coins className="h-4 w-4" />
            <span className="text-xs font-semibold">{t("当前每 10 分钟总奖励", "Current total reward per 10 minutes")}</span>
          </div>
          <p className={rewardPolicy ? "font-mono text-lg font-semibold text-white/90" : "text-sm font-semibold text-white/75"}>
            {loading && !rewardPolicy ? <Loader2 className="h-5 w-5 animate-spin" /> : rewardPoolValue}
          </p>
        </div>
        <p className="mt-1 text-[11px] text-white/35">
          {t("由本周期有效心跳设备平分", "Shared equally by eligible devices in this period")}
        </p>
      </div>

      <div className="rounded-2xl border border-white/10 bg-black/15 px-4 py-3">
        <div className="flex items-center gap-2 text-white/55">
          <CalendarClock className="h-4 w-4" />
          <span className="text-xs font-semibold">{t("全网首次心跳时间", "First network heartbeat")}</span>
        </div>
        <p className={stats?.first_heartbeat_unix != null ? "mt-2 text-sm font-semibold text-white/90" : "mt-2 text-sm font-semibold text-white/75"}>
          {loading && !stats ? <Loader2 className="h-5 w-5 animate-spin" /> : firstHeartbeatValue}
        </p>
      </div>

      <div className="rounded-2xl border border-white/10 bg-black/15 px-4 py-3">
        <div className="flex items-center gap-2 text-white/55">
          <TimerReset className="h-4 w-4" />
          <span className="text-xs font-semibold">{t("下次减半时间", "Next halving")}</span>
        </div>
        <p className={rewardPolicy?.next_halving_at_unix != null ? "mt-2 text-sm font-semibold text-white/90" : "mt-2 text-sm font-semibold text-white/75"}>
          {loading && !rewardPolicy ? <Loader2 className="h-5 w-5 animate-spin" /> : nextHalvingValue}
        </p>
      </div>
    </div>
  );
}
