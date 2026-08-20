import { ArrowLeft, ArrowLeftRight, Calculator, RefreshCw } from "lucide-react";
import { useEffect, useMemo, useState } from "react";

import { calculateNniAprEstimate, parsePositiveNniDevicePrice } from "../lib/nni-apr";
import { appStorageKey } from "../lib/product-identity";
import type { NniBancorMarketResponse, NniRewardsResponse } from "../types/api";

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
  const estimate = useMemo(
    () => calculateNniAprEstimate({ devicePriceUsd, rewards, market }),
    [devicePriceUsd, market, rewards],
  );
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
              <span>{t("NNI 奖励年化", "NNI reward APR")}</span>
            </div>
            <p className="mt-3 max-w-3xl text-sm leading-6 text-white/60">
              {t(
                "用本设备最近一个已结算奖励周期和 Bancor 池当前边际价格，估算设备价格对应的简单年化收益率。",
                "Estimate simple annualized return from this device's latest settled reward period and the Bancor pool's current marginal price.",
              )}
            </p>
          </div>
          <div className="flex flex-wrap items-center justify-end gap-2">
            <button type="button" className="theme-secondary-btn" onClick={onBack}>
              <ArrowLeft className="h-4 w-4" />
              {t("返回 NNI", "Back to NNI")}
            </button>
            <button type="button" className="theme-secondary-btn" onClick={onOpenBancor}>
              <ArrowLeftRight className="h-4 w-4" />
              {t("查看市场", "Open market")}
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

        <div className="mt-5 grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
          <AprMetric
            label="APR"
            value={estimate ? `${formatDecimal(estimate.aprPercent, lang)}%` : "—"}
            emphasized
          />
          <AprMetric
            label={t("年化奖励估值", "Annual reward value")}
            value={estimate ? `${formatDecimal(estimate.annualRewardUsd, lang)} USD` : "—"}
          />
          <AprMetric
            label={t("最近周期奖励", "Latest period reward")}
            value={estimate ? `${formatDecimal(estimate.rewardAic, lang, 8)} AIC` : "—"}
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

        <p className="mt-5 text-xs leading-5 text-white/45">
          {t(
            "APR 是按最近周期静态外推的简单年化估算，不含复利、交易手续费和价格影响。设备数、奖励规则与 AIC 价格变化都会改变结果。",
            "APR is a simple annualized estimate extrapolated from the latest period. It excludes compounding, trading fees, and price impact. Device count, reward policy, and AIC price changes will alter the result.",
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
