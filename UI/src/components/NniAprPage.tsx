import { ArrowLeft, ArrowLeftRight, Calculator, RefreshCw } from "lucide-react";
import { useEffect, useMemo, useState } from "react";

import {
  calculateNniAprEstimate,
  calculateNniPeriodAprEstimate,
  calculateNniSimplePaybackDays,
  parsePositiveNniDevicePrice,
} from "../lib/nni-apr";
import { appStorageKey } from "../lib/product-identity";
import type {
  NniBancorMarketResponse,
  NniRewardsResponse,
  NniRewardWindowKey,
} from "../types/api";

type UiLanguage = "zh" | "en";
type Translate = (zh: string, en: string) => string;

const DEVICE_PRICE_STORAGE_KEY = appStorageKey("nni.apr.devicePriceUsd");

interface NniAprPriceStorage {
  getItem: (key: string) => string | null;
  setItem: (key: string, value: string) => void;
}

export function readNniAprDevicePrice(storage?: NniAprPriceStorage): string {
  if (!storage) return "";
  try {
    return storage.getItem(DEVICE_PRICE_STORAGE_KEY) ?? "";
  } catch {
    return "";
  }
}

export function persistNniAprDevicePrice(storage: NniAprPriceStorage | undefined, value: string): void {
  if (!storage) return;
  try {
    storage.setItem(DEVICE_PRICE_STORAGE_KEY, value);
  } catch {
    // Private browsing or a storage policy may make persistence unavailable.
  }
}

function formatDecimal(value: number, lang: UiLanguage, maximumFractionDigits = 2): string {
  return new Intl.NumberFormat(lang === "zh" ? "zh-CN" : "en-US", {
    maximumFractionDigits,
  }).format(value);
}

function formatPaybackTime(days: number | null, lang: UiLanguage, t: Translate): string {
  if (days === null) return "—";
  if (days >= 365) {
    return `${formatDecimal(days / 365, lang)} ${t("年", "years")}`;
  }
  if (days >= 1) {
    return `${formatDecimal(days, lang)} ${t("天", "days")}`;
  }
  const hours = days * 24;
  if (hours >= 1) {
    return `${formatDecimal(hours, lang)} ${t("小时", "hours")}`;
  }
  return `${formatDecimal(hours * 60, lang)} ${t("分钟", "minutes")}`;
}

export interface NniAprPageProps {
  lang: UiLanguage;
  t: Translate;
  joined: boolean;
  rewards: NniRewardsResponse | null;
  market: NniBancorMarketResponse | null;
  rewardsLoading: boolean;
  marketLoading: boolean;
  rewardsError: string | null;
  marketError: string | null;
  formatUnixDateTime: (ts: number | null | undefined) => string;
  onBack: () => void;
  onOpenBancor: () => void;
  onRefresh: () => unknown | Promise<unknown>;
}

export function NniAprPage({
  lang,
  t,
  joined,
  rewards,
  market,
  rewardsLoading,
  marketLoading,
  rewardsError,
  marketError,
  formatUnixDateTime,
  onBack,
  onOpenBancor,
  onRefresh,
}: NniAprPageProps) {
  const [devicePriceUsd, setDevicePriceUsd] = useState(() =>
    readNniAprDevicePrice(typeof window === "undefined" ? undefined : window.localStorage),
  );
  const [periodWindow, setPeriodWindow] = useState<NniRewardWindowKey>("week");
  const estimate = useMemo(
    () => calculateNniAprEstimate({ devicePriceUsd, rewards, market }),
    [devicePriceUsd, market, rewards],
  );
  const periodEstimate = useMemo(
    () => calculateNniPeriodAprEstimate({
      devicePriceUsd,
      rewards,
      market,
      windowKey: periodWindow,
    }),
    [devicePriceUsd, market, periodWindow, rewards],
  );
  const periodPaybackDays = calculateNniSimplePaybackDays(periodEstimate?.aprPercent ?? 0);
  const priceInvalid = devicePriceUsd.trim() !== "" && parsePositiveNniDevicePrice(devicePriceUsd) === null;
  const loading = rewardsLoading || marketLoading;

  useEffect(() => {
    persistNniAprDevicePrice(
      typeof window === "undefined" ? undefined : window.localStorage,
      devicePriceUsd,
    );
  }, [devicePriceUsd]);

  return (
    <div className="mx-auto grid w-full max-w-5xl gap-5 pb-10">
      <section className="theme-shadow-card p-5 sm:p-6">
        <div className="flex flex-wrap items-start justify-between gap-4">
          <div>
            <div className="flex items-center gap-2 text-xl font-semibold text-emerald-100 sm:text-2xl">
              <Calculator className="h-5 w-5" />
              <span>{t("NNI 奖励 APR", "NNI reward APR")}</span>
            </div>
            <p className="mt-3 max-w-3xl text-sm leading-6 text-white/60">
              {t(
                "根据本设备奖励、设备价格和 Bancor 池当前边际价格计算 APR；可查看最近结算窗口或选定历史周期。",
                "Calculate APR from this device's rewards, device price, and the Bancor pool's current marginal price, using either the latest settlement or a selected history window.",
              )}
            </p>
          </div>
          <div className="flex flex-wrap items-center justify-end gap-2">
            <button type="button" className="theme-secondary-btn" onClick={onBack}>
              <ArrowLeft className="h-4 w-4" />
              {t("返回 NNI", "Back to NNI")}
            </button>
            <button
              type="button"
              className="theme-secondary-btn"
              data-nni-apr-back-to-bancor="true"
              onClick={onOpenBancor}
            >
              <ArrowLeftRight className="h-4 w-4" />
              {t("返回 Bancor", "Back to Bancor")}
            </button>
            <button
              type="button"
              className="theme-secondary-btn"
              disabled={loading}
              onClick={() => void onRefresh()}
            >
              <RefreshCw className={`h-4 w-4 ${loading ? "animate-spin" : ""}`} />
              {t("刷新", "Refresh")}
            </button>
          </div>
        </div>
      </section>

      <section className="theme-shadow-card p-5 sm:p-6">
        <label className="block max-w-md">
          <span className="text-sm font-semibold text-white/85">{t("设备价格（USD）", "Device price (USD)")}</span>
          <input
            type="text"
            inputMode="decimal"
            value={devicePriceUsd}
            onChange={(event) => setDevicePriceUsd(event.target.value)}
            placeholder={t("例如 299", "For example, 299")}
            className="theme-input mt-2 w-full"
            aria-invalid={priceInvalid}
          />
          <span className={`mt-2 block text-xs leading-5 ${priceInvalid ? "text-red-200" : "text-white/45"}`}>
            {priceInvalid
              ? t("请输入大于 0 的设备价格。", "Enter a device price greater than zero.")
              : t("价格只保存在当前浏览器中，下次打开仍会保留。", "The price is saved only in this browser and remains available next time.")}
          </span>
        </label>

        {!joined ? (
          <p className="mt-4 rounded-lg border border-amber-300/20 bg-amber-300/10 px-4 py-3 text-sm text-amber-100">
            {t("请先返回 NNI 页面加入网络，才能读取本设备奖励。", "Return to NNI and join the network before loading this device's rewards.")}
          </p>
        ) : null}
        {rewardsError ? (
          <p className="mt-4 break-words rounded-lg border border-amber-300/20 bg-amber-300/10 px-4 py-3 text-sm text-amber-100">
            {t("奖励读取失败：", "Reward data could not be loaded: ")}{rewardsError}
          </p>
        ) : null}
        {marketError ? (
          <p className="mt-4 break-words rounded-lg border border-amber-300/20 bg-amber-300/10 px-4 py-3 text-sm text-amber-100">
            {t("市场价格读取失败：", "Market price could not be loaded: ")}{marketError}
          </p>
        ) : null}

        <div className="mt-6">
          <h2 className="text-base font-semibold text-white/90">{t("实时 APR", "Live APR")}</h2>
          <p className="mt-1 text-xs leading-5 text-white/45">
            {t("使用本设备最近一个已结算奖励窗口。", "Uses this device's latest settled reward window.")}
          </p>
        </div>

        <div className="mt-4 grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
          <AprMetric
            label={t("实时 APR", "Live APR")}
            value={estimate ? `${formatDecimal(estimate.aprPercent, lang)}%` : "—"}
            emphasized
          />
          <AprMetric
            label={t("最近周期奖励", "Latest period reward")}
            value={estimate ? `${formatDecimal(estimate.rewardAic, lang, 8)} AIC` : "—"}
          />
          <AprMetric
            label={t("最近奖励估值", "Latest reward value")}
            value={estimate ? `${formatDecimal(estimate.periodValueUsd, lang, 8)} USD` : "—"}
          />
          <AprMetric
            label={t("AIC 当前价格", "Current AIC price")}
            value={estimate ? `${formatDecimal(estimate.aicPriceUsd, lang, 12)} USD` : "—"}
          />
        </div>

        <div className="mt-5 grid gap-3 text-sm sm:grid-cols-2">
          <div className="rounded-lg border border-white/10 bg-black/15 p-4">
            <p className="text-xs font-semibold text-white/45">{t("采用的奖励周期", "Reward period used")}</p>
            <p className="mt-2 text-white/80">
              {estimate
                ? `${formatUnixDateTime(estimate.record.period_start_unix)} – ${formatUnixDateTime(estimate.record.period_end_unix)}`
                : t("等待读取最近结算记录", "Waiting for the latest settlement")}
            </p>
            <p className="mt-1 text-xs text-white/45">
              {estimate
                ? t(`周期 ${estimate.periodSeconds} 秒`, `${estimate.periodSeconds}-second period`)
                : t("通常每 10 分钟结算一次", "Usually settled every 10 minutes")}
            </p>
          </div>
          <div className="rounded-lg border border-white/10 bg-black/15 p-4">
            <p className="text-xs font-semibold text-white/45">{t("周期奖励估值", "Period reward value")}</p>
            <p className="mt-2 text-white/80">
              {estimate ? `${formatDecimal(estimate.periodValueUsd, lang, 8)} USD` : "—"}
            </p>
            <p className="mt-1 text-xs text-white/45">
              {t("页面停留期间每 10 分钟自动刷新奖励和池价格。", "Rewards and pool price refresh every 10 minutes while this page remains open.")}
            </p>
          </div>
        </div>

        <div className="mt-7 border-t border-white/10 pt-6">
          <div className="flex flex-wrap items-end justify-between gap-3">
            <div>
              <h2 className="text-base font-semibold text-white/90">{t("周期 APR", "Period APR")}</h2>
              <p className="mt-1 text-xs leading-5 text-white/45">
                {t(
                  "按所选窗口的累计奖励计算，假定这些 AIC 没有卖出，并统一按当前池价格估值。",
                  "Uses total rewards in the selected window, assumes the rewarded AIC was not sold, and values all of it at the current pool price.",
                )}
              </p>
            </div>
            <label className="min-w-40">
              <span className="sr-only">{t("选择计算周期", "Select calculation period")}</span>
              <select
                className="theme-input w-full"
                value={periodWindow}
                onChange={(event) => setPeriodWindow(event.target.value as NniRewardWindowKey)}
                aria-label={t("周期 APR 窗口", "Period APR window")}
              >
                <option value="week">{t("周（最近 7 天）", "Week (last 7 days)")}</option>
                <option value="month">{t("月（最近 30 天）", "Month (last 30 days)")}</option>
                <option value="year">{t("年（最近 365 天）", "Year (last 365 days)")}</option>
              </select>
            </label>
          </div>

          <div className="mt-4 grid gap-3 sm:grid-cols-2 xl:grid-cols-5">
            <AprMetric
              label={t("周期 APR", "Period APR")}
              value={periodEstimate ? `${formatDecimal(periodEstimate.aprPercent, lang)}%` : "—"}
              emphasized
            />
            <AprMetric
              label={t("预计回本时间", "Estimated payback time")}
              value={formatPaybackTime(periodPaybackDays, lang, t)}
              emphasized
            />
            <AprMetric
              label={t("本设备窗口累计奖励", "This device's window rewards")}
              value={periodEstimate ? `${formatDecimal(periodEstimate.rewardAic, lang, 8)} AIC` : "—"}
            />
            <AprMetric
              label={t("窗口奖励估值", "Window reward value")}
              value={periodEstimate ? `${formatDecimal(periodEstimate.windowValueUsd, lang, 8)} USD` : "—"}
            />
            <AprMetric
              label={t("结算记录", "Settlements")}
              value={periodEstimate ? formatDecimal(periodEstimate.window.reward_grant_count, lang, 0) : "—"}
            />
          </div>

          <div className="mt-3 rounded-lg border border-white/10 bg-black/15 p-4 text-sm">
            <p className="text-xs font-semibold text-white/45">{t("实际奖励覆盖", "Actual reward coverage")}</p>
            <p className="mt-2 text-white/80">
              {periodEstimate
                ? `${formatUnixDateTime(periodEstimate.coverageStartUnix)} – ${formatUnixDateTime(periodEstimate.coverageEndUnix)}`
                : rewards?.reward_windows
                  ? t("当前窗口暂无可用数据", "No data is available for this window")
                  : t("所选 NNI 节点暂未提供周期奖励汇总", "The selected NNI node does not provide period reward summaries yet")}
            </p>
            {periodEstimate ? (
              <p className="mt-1 text-xs leading-5 text-white/45">
                {periodEstimate.coverageSeconds < periodEstimate.window.window_seconds
                  ? t(
                    "设备奖励记录尚未覆盖完整所选窗口，APR 按已有记录的实际时长换算。",
                    "The device does not yet have a complete selected window, so APR uses the actual duration covered by its reward records.",
                  )
                  : t(
                    "设备奖励记录已覆盖完整所选窗口。",
                    "The device reward records cover the complete selected window.",
                  )}
              </p>
            ) : null}
          </div>
        </div>

        <p className="mt-5 text-xs leading-5 text-white/45">
          {t(
            "APR 和回本时间均为估算值；回本时间按所选周期 APR 保持不变并采用单利外推，不含复利、交易手续费和价格影响。设备数、奖励规则与 AIC 价格变化都会改变结果。",
            "APR and payback time are estimates. Payback assumes the selected period APR remains constant and uses a simple-return projection, excluding compounding, trading fees, and price impact. Device count, reward policy, and AIC price changes will alter the result.",
          )}
        </p>
      </section>
    </div>
  );
}

function AprMetric({
  label,
  value,
  emphasized = false,
}: {
  label: string;
  value: string;
  emphasized?: boolean;
}) {
  return (
    <div className={`min-w-0 rounded-lg border p-4 ${emphasized ? "border-emerald-300/20 bg-emerald-300/[0.07]" : "border-white/10 bg-black/15"}`}>
      <p className="text-xs font-semibold text-white/50">{label}</p>
      <p className={`mt-3 break-words font-mono text-xl font-semibold ${emphasized ? "text-emerald-100" : "text-white/90"}`}>
        {value}
      </p>
    </div>
  );
}
