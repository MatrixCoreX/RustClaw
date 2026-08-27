import {
  ArrowLeftRight,
  CircleDollarSign,
  Coins,
  RefreshCw,
  Settings2,
  WalletCards,
} from "lucide-react";

import type { NniBancorAccountResponse, NniBancorMarketResponse } from "../types/api";
import {
  formatBancorBalanceAmount,
  formatBancorBalanceHoverAmount,
  formatBancorTradeHistoryAmount,
} from "../lib/bancor-amount-display";
import { NniPublicKeyDisplay } from "./NniPublicKeyDisplay";

type Translate = (zh: string, en: string) => string;

interface FixedDecimal {
  units: bigint;
  scale: number;
}

export interface AssetPortfolioValues {
  aicValueUsd: string;
  totalValueUsd: string;
}

function parseUnsignedDecimal(value: string): FixedDecimal | null {
  const match = /^(\d+)(?:\.(\d+))?$/.exec(value.trim());
  if (!match) return null;
  const fraction = match[2] ?? "";
  return {
    units: BigInt(`${match[1]}${fraction}`),
    scale: fraction.length,
  };
}

function rescaleDecimal(value: FixedDecimal, targetScale: number): bigint {
  if (value.scale === targetScale) return value.units;
  if (value.scale < targetScale) {
    return value.units * (10n ** BigInt(targetScale - value.scale));
  }
  return value.units / (10n ** BigInt(value.scale - targetScale));
}

function formatDecimal(units: bigint, scale: number): string {
  if (scale === 0) return String(units);
  const divisor = 10n ** BigInt(scale);
  return `${units / divisor}.${String(units % divisor).padStart(scale, "0")}`;
}

export function calculateAssetPortfolioValues(
  account: NniBancorAccountResponse,
  market: NniBancorMarketResponse,
): AssetPortfolioValues | null {
  const aicBalance = parseUnsignedDecimal(account.aic_balance);
  const usdBalance = parseUnsignedDecimal(account.usd_balance);
  const aicPrice = parseUnsignedDecimal(market.marginal_price_usd_per_aic);
  if (!aicBalance || !usdBalance || !aicPrice) return null;

  const displayScale = 8;
  const aicValueUnits = rescaleDecimal({
    units: aicBalance.units * aicPrice.units,
    scale: aicBalance.scale + aicPrice.scale,
  }, displayScale);
  const totalValueUnits = aicValueUnits + rescaleDecimal(usdBalance, displayScale);
  return {
    aicValueUsd: formatDecimal(aicValueUnits, displayScale),
    totalValueUsd: formatDecimal(totalValueUnits, displayScale),
  };
}

function WalletAmount({ value, suffix }: { value: string; suffix: string }) {
  const visible = formatBancorBalanceAmount(value);
  const full = formatBancorBalanceHoverAmount(value);
  return (
    <span className="min-w-0 text-right" title={`${full} ${suffix}`} data-assets-full-value={full}>
      <span className="block truncate text-base font-semibold text-[var(--theme-text-strong)] sm:text-lg">
        {visible}
      </span>
      <span className="block text-[11px] font-medium text-[var(--theme-text-muted)]">{suffix}</span>
    </span>
  );
}

export function AssetsPage({
  t,
  account,
  market,
  assetOwnerPubkey,
  signingDeviceReady,
  accountLoading,
  marketLoading,
  error,
  hardwareAccountAccessUnavailable,
  onRefresh,
  onOpenBancor,
  onOpenNni,
}: {
  t: Translate;
  account: NniBancorAccountResponse | null;
  market: NniBancorMarketResponse | null;
  assetOwnerPubkey?: string | null;
  signingDeviceReady: boolean;
  accountLoading: boolean;
  marketLoading: boolean;
  error: string | null;
  hardwareAccountAccessUnavailable: boolean;
  onRefresh: () => void | Promise<unknown>;
  onOpenBancor: () => void;
  onOpenNni: () => void;
}) {
  const portfolio = account && market ? calculateAssetPortfolioValues(account, market) : null;
  const loading = accountLoading || marketLoading;
  const accountAvailable = Boolean(assetOwnerPubkey && account);
  const statusMessage = !assetOwnerPubkey
    ? t("尚未绑定资产账户。请先完成资产账户设置。", "No asset account is bound yet. Set up an asset account first.")
    : !signingDeviceReady
      ? t("当前设备无法签名读取私有余额。", "This device cannot sign a private balance request.")
      : hardwareAccountAccessUnavailable
        ? t("当前硬件授权暂时无法读取这个资产账户。", "The current hardware authorization cannot read this asset account right now.")
        : error && !account
          ? error
          : t("资产余额暂不可用，请刷新后重试。", "Asset balances are unavailable. Refresh and try again.");

  return (
    <div className="space-y-5" data-assets-page="true">
      <header className="flex flex-col gap-4 sm:flex-row sm:items-start sm:justify-between">
        <div className="min-w-0">
          <div className="flex items-center gap-2 text-[var(--theme-icon-accent-color)]">
            <WalletCards className="h-5 w-5" />
            <span className="text-xs font-semibold uppercase tracking-wide">{t("钱包视图", "Wallet view")}</span>
          </div>
          <h2 className="mt-2 text-2xl font-semibold text-[var(--theme-text-strong)]">{t("资产", "Assets")}</h2>
          <p className="mt-1 max-w-2xl text-sm leading-6 text-[var(--theme-text-muted)]">
            {t("查看当前资产账户中的余额与按市场价格估算的价值。", "View balances in the current asset account and their estimated market value.")}
          </p>
        </div>
        <div className="flex flex-wrap gap-2">
          <button type="button" className="theme-secondary-btn px-3 py-2 text-sm" onClick={onOpenNni}>
            <Settings2 className="h-4 w-4" />
            {t("管理账户", "Manage account")}
          </button>
          <button type="button" className="theme-accent-btn px-3 py-2 text-sm" onClick={onOpenBancor}>
            <ArrowLeftRight className="h-4 w-4" />
            {t("交易", "Trade")}
          </button>
        </div>
      </header>

      <section className="theme-panel overflow-hidden p-5 sm:p-6" aria-labelledby="asset-overview-title">
        <div className="flex flex-col gap-5 lg:flex-row lg:items-end lg:justify-between">
          <div className="min-w-0">
            <p id="asset-overview-title" className="text-sm font-medium text-[var(--theme-text-muted)]">
              {t("总资产估值", "Estimated portfolio value")}
            </p>
            <div className="mt-2 min-h-12">
              {portfolio ? (
                <p
                  className="break-all text-3xl font-semibold text-[var(--theme-text-strong)] sm:text-4xl"
                  title={`${formatBancorBalanceHoverAmount(portfolio.totalValueUsd)} USD`}
                  data-assets-total-value={portfolio.totalValueUsd}
                >
                  {formatBancorBalanceAmount(portfolio.totalValueUsd)}
                  <span className="ml-2 text-base font-medium text-[var(--theme-text-muted)]">USD</span>
                </p>
              ) : (
                <p className="text-3xl font-semibold text-[var(--theme-text-faint)]">--</p>
              )}
            </div>
            <p className="mt-2 text-xs leading-5 text-[var(--theme-text-faint)]">
              {t("按当前 BANCOR 边际价格估算，不代表实际成交金额。", "Estimated using the current BANCOR marginal price; this is not an executable quote.")}
            </p>
          </div>
          <button
            type="button"
            className="theme-secondary-btn shrink-0 px-3 py-2 text-sm disabled:cursor-not-allowed disabled:opacity-50"
            disabled={loading || !assetOwnerPubkey || !signingDeviceReady}
            onClick={() => void onRefresh()}
          >
            <RefreshCw className={`h-4 w-4 ${loading ? "animate-spin" : ""}`} />
            {loading ? t("读取中", "Loading") : t("刷新资产", "Refresh assets")}
          </button>
        </div>

        <div className="mt-5 border-t border-[var(--theme-border)] pt-4">
          <p className="text-xs font-medium text-[var(--theme-text-muted)]">{t("资产账户", "Asset account")}</p>
          {assetOwnerPubkey ? (
            <NniPublicKeyDisplay
              value={assetOwnerPubkey}
              t={t}
              className="mt-2"
              valueClassName="text-xs text-[var(--theme-text-body)]"
              shorten={{ head: 12, tail: 10 }}
              allowFormatSwitch={false}
            />
          ) : (
            <p className="mt-2 text-sm text-[var(--theme-text-faint)]">{t("未绑定", "Not bound")}</p>
          )}
        </div>
      </section>

      <section className="theme-panel overflow-hidden" aria-labelledby="asset-list-title">
        <div className="flex items-center justify-between border-b border-[var(--theme-border)] px-5 py-4 sm:px-6">
          <div>
            <h3 id="asset-list-title" className="text-base font-semibold text-[var(--theme-text-strong)]">
              {t("资产列表", "Asset list")}
            </h3>
            <p className="mt-1 text-xs text-[var(--theme-text-muted)]">{t("当前账户支持的资产", "Assets supported by this account")}</p>
          </div>
          <span className="rounded-full border border-[var(--theme-border)] bg-[var(--theme-card-strong)] px-2.5 py-1 text-xs text-[var(--theme-text-muted)]">
            {accountAvailable ? t("已同步", "Synced") : t("待同步", "Not synced")}
          </span>
        </div>

        {accountAvailable && account ? (
          <div className="divide-y divide-[var(--theme-border)]" data-assets-list-ready="true">
            <div className="grid min-h-24 grid-cols-[minmax(0,1fr)_auto] items-center gap-4 px-5 py-4 sm:grid-cols-[minmax(180px,1fr)_minmax(150px,0.7fr)_auto] sm:px-6">
              <div className="flex min-w-0 items-center gap-3">
                <span className="flex h-10 w-10 shrink-0 items-center justify-center rounded-full border border-emerald-400/25 bg-emerald-500/10 text-emerald-300">
                  <Coins className="h-5 w-5" />
                </span>
                <div className="min-w-0">
                  <p className="font-semibold text-[var(--theme-text-strong)]">AIC</p>
                  <p className="truncate text-xs text-[var(--theme-text-muted)]">{t("网络原生资产", "Network asset")}</p>
                </div>
              </div>
              <div className="hidden min-w-0 text-right sm:block">
                <p className="text-xs text-[var(--theme-text-muted)]">{t("参考价格", "Reference price")}</p>
                <p className="mt-1 truncate text-sm text-[var(--theme-text-body)]">
                  {market ? `${formatBancorTradeHistoryAmount(market.marginal_price_usd_per_aic)} USD` : "--"}
                </p>
                <p className="mt-0.5 text-[11px] text-[var(--theme-text-faint)]">
                  {portfolio ? `≈ ${formatBancorBalanceAmount(portfolio.aicValueUsd)} USD` : ""}
                </p>
              </div>
              <WalletAmount value={account.aic_balance} suffix="AIC" />
            </div>

            <div className="grid min-h-24 grid-cols-[minmax(0,1fr)_auto] items-center gap-4 px-5 py-4 sm:grid-cols-[minmax(180px,1fr)_minmax(150px,0.7fr)_auto] sm:px-6">
              <div className="flex min-w-0 items-center gap-3">
                <span className="flex h-10 w-10 shrink-0 items-center justify-center rounded-full border border-amber-400/25 bg-amber-500/10 text-amber-300">
                  <CircleDollarSign className="h-5 w-5" />
                </span>
                <div className="min-w-0">
                  <p className="font-semibold text-[var(--theme-text-strong)]">USD</p>
                  <p className="truncate text-xs text-[var(--theme-text-muted)]">{t("美元记账资产", "USD ledger asset")}</p>
                </div>
              </div>
              <div className="hidden min-w-0 text-right sm:block">
                <p className="text-xs text-[var(--theme-text-muted)]">{t("参考价格", "Reference price")}</p>
                <p className="mt-1 text-sm text-[var(--theme-text-body)]">1 USD</p>
              </div>
              <WalletAmount value={account.usd_balance} suffix="USD" />
            </div>
          </div>
        ) : (
          <div className="px-5 py-10 text-center sm:px-6" data-assets-empty-state="true">
            <WalletCards className="mx-auto h-8 w-8 text-[var(--theme-text-faint)]" />
            <p className="mx-auto mt-3 max-w-lg text-sm leading-6 text-[var(--theme-text-muted)]">{statusMessage}</p>
            <div className="mt-4 flex flex-wrap justify-center gap-2">
              <button type="button" className="theme-secondary-btn px-3 py-2 text-sm" onClick={onOpenNni}>
                <Settings2 className="h-4 w-4" />
                {t("管理资产账户", "Manage asset account")}
              </button>
              {assetOwnerPubkey && signingDeviceReady ? (
                <button type="button" className="theme-secondary-btn px-3 py-2 text-sm" disabled={loading} onClick={() => void onRefresh()}>
                  <RefreshCw className={`h-4 w-4 ${loading ? "animate-spin" : ""}`} />
                  {t("重试", "Retry")}
                </button>
              ) : null}
            </div>
          </div>
        )}
      </section>
    </div>
  );
}
