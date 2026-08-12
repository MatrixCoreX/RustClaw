import { ChevronLeft, ChevronRight, Coins, Loader2, RefreshCw, WalletCards } from "lucide-react";

import { shortNniValue } from "../lib/nni-display";
import type { NniRewardsResponse } from "../types/api";
import { NniPublicKeyDisplay } from "./NniPublicKeyDisplay";

type Translate = (zh: string, en: string) => string;

export interface NniRewardsPanelProps {
  rewards: NniRewardsResponse | null;
  currentPointBalance: string | null;
  currentPointBalanceLoading: boolean;
  loading: boolean;
  error: string | null;
  pageSize: number;
  t: Translate;
  formatUnixDateTime: (ts: number | null | undefined) => string;
  onFetch: (page: number) => unknown | Promise<unknown>;
  onRefresh: (page: number) => unknown | Promise<unknown>;
}

export function NniRewardsPanel({
  rewards,
  currentPointBalance,
  currentPointBalanceLoading,
  loading,
  error,
  pageSize,
  t,
  formatUnixDateTime,
  onFetch,
  onRefresh,
}: NniRewardsPanelProps) {
  const page = rewards?.page ?? 1;
  const totalPages = Math.max(1, rewards?.total_pages ?? 1);
  const records = rewards?.records ?? [];
  const canPrev = page > 1;
  const canNext = page < totalPages;

  return (
    <div
      id="nni-history-rewards-panel"
      role="tabpanel"
      aria-labelledby="nni-history-rewards-tab"
      className="mt-5"
    >
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <p className="theme-kicker text-[10px] uppercase tracking-[0.28em]">
            {t("原生智能奖励", "Native intelligence rewards")}
          </p>
          <h4 className="mt-2 text-lg font-semibold">{t("本设备奖励账本", "This device's reward ledger")}</h4>
          <p className="mt-2 max-w-2xl text-sm leading-6 text-white/60">
            {t(
              "每次刷新都会由本机设备签署一次临时挑战，服务端只返回当前公钥自己的奖励。",
              "Each refresh signs a temporary challenge on this device. The server returns rewards for this public key only.",
            )}
          </p>
        </div>
        <button
          type="button"
          onClick={() => void onRefresh(page)}
          disabled={loading || currentPointBalanceLoading}
          className="theme-secondary-btn px-3 py-2 text-xs disabled:cursor-not-allowed disabled:opacity-50"
        >
          {loading || currentPointBalanceLoading ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <RefreshCw className="h-3.5 w-3.5" />}
          {t("刷新奖励", "Refresh rewards")}
        </button>
      </div>

      {error ? (
        <p className="mt-3 break-words rounded-xl border border-amber-300/20 bg-amber-300/10 px-3 py-2 text-xs leading-5 text-amber-100">
          {t("奖励暂时无法读取：", "Rewards could not be loaded: ")}
          {error}
        </p>
      ) : null}

      <div className="mt-4 grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
        <div className="rounded-2xl border border-emerald-300/15 bg-emerald-300/[0.06] p-4">
          <div className="flex items-center gap-2 text-emerald-100/75">
            <Coins className="h-4 w-4" />
            <span className="text-xs font-semibold">{t("累计奖励", "Total rewards")}</span>
          </div>
          <p className="mt-3 font-mono text-2xl font-semibold text-emerald-50">
            {rewards?.total_reward_points ?? "0.0000"}
          </p>
          <p className="mt-1 text-xs text-white/45">{t("点", "points")}</p>
        </div>
        <div className="rounded-2xl border border-sky-300/15 bg-sky-300/[0.06] p-4">
          <div className="flex items-center gap-2 text-sky-100/75">
            <WalletCards className="h-4 w-4" />
            <span className="text-xs font-semibold">{t("当前持有", "Current holdings")}</span>
          </div>
          <p className="mt-3 font-mono text-2xl font-semibold text-sky-50">
            {currentPointBalanceLoading ? "…" : currentPointBalance ?? "—"}
          </p>
          <p className="mt-1 text-xs text-white/45">POINT</p>
        </div>
        <div className="rounded-2xl border border-white/10 bg-black/20 p-4">
          <p className="text-xs font-semibold text-white/50">{t("获得奖励的时段", "Rewarded periods")}</p>
          <p className="mt-3 text-2xl font-semibold text-white/90">{rewards?.reward_grant_count ?? 0}</p>
          <p className="mt-1 text-xs text-white/45">{t("每个时段最多记录一次", "At most once per period")}</p>
        </div>
        <div className="rounded-2xl border border-white/10 bg-black/20 p-4">
          <p className="text-xs font-semibold text-white/50">{t("最近结算", "Latest settlement")}</p>
          <p className="mt-3 text-sm font-semibold text-white/90">
            {formatUnixDateTime(rewards?.latest_period_end_unix)}
          </p>
          <div className="mt-1 flex min-w-0 flex-wrap items-center gap-1 text-xs text-white/45">
            <span>{t("设备", "Device")}:</span>
            <NniPublicKeyDisplay
              value={rewards?.device_pubkey}
              t={t}
              shorten={{ head: 10, tail: 8 }}
              valueClassName="text-xs text-white/45"
            />
          </div>
        </div>
      </div>

      <div className="mt-5 flex flex-wrap items-end justify-between gap-2">
        <div>
          <h5 className="text-sm font-semibold text-white/85">{t("奖励明细", "Reward records")}</h5>
          <p className="mt-1 text-xs text-white/45">
            {t(
              `共 ${rewards?.total ?? 0} 条，每页 ${pageSize} 条。`,
              `${rewards?.total ?? 0} records total, ${pageSize} per page.`,
            )}
          </p>
        </div>
        {rewards?.node_url ? (
          <p className="font-mono text-xs text-white/40" title={rewards.node_url}>
            {shortNniValue(rewards.node_url)}
          </p>
        ) : null}
      </div>

      <div className="mt-3 overflow-hidden rounded-2xl border border-white/10 bg-black/20">
        {records.length === 0 ? (
          <p className="px-4 py-5 text-sm text-white/55">
            {loading
              ? t("正在验证设备并读取奖励...", "Verifying the device and loading rewards...")
              : t("当前设备还没有奖励记录。完成结算后的原生智能奖励会显示在这里。", "This device has no reward records yet. Settled native intelligence rewards will appear here.")}
          </p>
        ) : (
          records.map((record) => (
            <div key={record.id} className="border-t border-white/10 px-4 py-3 first:border-t-0">
              <div className="flex flex-wrap items-center justify-between gap-2">
                <span className="setup-status setup-status-done font-mono">+{record.reward_points}</span>
                <span className="text-xs text-white/50">{formatUnixDateTime(record.awarded_at_unix)}</span>
              </div>
              <div className="mt-3 grid gap-3 text-xs sm:grid-cols-2">
                <div>
                  <p className="font-semibold tracking-[0.12em] text-white/35">{t("奖励时段", "Reward period")}</p>
                  <p className="mt-1 text-white/75">
                    {formatUnixDateTime(record.period_start_unix)} – {formatUnixDateTime(record.period_end_unix)}
                  </p>
                </div>
                <div>
                  <p className="font-semibold tracking-[0.12em] text-white/35">{t("本时段心跳", "Heartbeats in period")}</p>
                  <p className="mt-1 text-white/75">
                    {t(
                      `${record.heartbeat_count_in_period} 次，按 1 台设备计奖`,
                      `${record.heartbeat_count_in_period} heartbeats, rewarded as 1 device`,
                    )}
                  </p>
                </div>
              </div>
            </div>
          ))
        )}
      </div>

      <div className="mt-4 flex flex-wrap items-center justify-between gap-3">
        <p className="text-xs text-white/50">
          {t(`第 ${page} / ${totalPages} 页`, `Page ${page} of ${totalPages}`)}
        </p>
        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={() => void onFetch(page - 1)}
            disabled={!canPrev || loading}
            className="theme-secondary-btn px-3 py-2 text-xs disabled:cursor-not-allowed disabled:opacity-50"
          >
            <ChevronLeft className="h-3.5 w-3.5" />
            {t("上一页", "Previous")}
          </button>
          <button
            type="button"
            onClick={() => void onFetch(page + 1)}
            disabled={!canNext || loading}
            className="theme-secondary-btn px-3 py-2 text-xs disabled:cursor-not-allowed disabled:opacity-50"
          >
            {t("下一页", "Next")}
            <ChevronRight className="h-3.5 w-3.5" />
          </button>
        </div>
      </div>
    </div>
  );
}
