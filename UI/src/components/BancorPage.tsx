import {
  ArrowDownUp,
  BarChart3,
  ChevronLeft,
  ChevronRight,
  Minus,
  Plus,
  RefreshCw,
  ShieldCheck,
  TrendingUp,
  WalletCards,
  X,
} from "lucide-react";
import { useEffect, useRef, useState } from "react";
import type { KeyboardEvent as ReactKeyboardEvent, PointerEvent as ReactPointerEvent, WheelEvent as ReactWheelEvent } from "react";

import { calculateBancorInputFee, formatBancorApiError, validateBancorTradeInput } from "../hooks/useBancorRuntime";
import type { useBancorRuntime } from "../hooks/useBancorRuntime";
import type { NniBancorCandle, NniBancorQuoteResponse } from "../types/api";

type Translate = (zh: string, en: string) => string;
type BancorRuntime = ReturnType<typeof useBancorRuntime>;
type CandleColor = { stroke: string; fill: string; volumeFill: string };
export const BANCOR_CANDLE_AUTO_REFRESH_SECONDS = 15;
export const BANCOR_DEFAULT_VISIBLE_CANDLES = 100;
const BANCOR_MIN_VISIBLE_CANDLES = 6;
const BANCOR_DRAG_HISTORY_HEADROOM = 6;
export const BANCOR_CANDLE_INTERVALS = [
  { seconds: 60, zh: "1分", en: "1m" },
  { seconds: 300, zh: "5分", en: "5m" },
  { seconds: 900, zh: "15分", en: "15m" },
  { seconds: 3_600, zh: "1小时", en: "1h" },
  { seconds: 14_400, zh: "4小时", en: "4h" },
  { seconds: 86_400, zh: "1天", en: "1d" },
  { seconds: 604_800, zh: "1周", en: "1W" },
  { seconds: 31_536_000, zh: "1年", en: "1Y" },
] as const;

export function resolveBancorCandlePalette(t: Translate): { up: CandleColor; down: CandleColor } {
  const red = {
    stroke: "#f87171",
    fill: "rgba(248,113,113,0.30)",
    volumeFill: "rgba(248,113,113,0.25)",
  };
  const green = {
    stroke: "#34d399",
    fill: "rgba(52,211,153,0.30)",
    volumeFill: "rgba(52,211,153,0.25)",
  };
  return t("zh", "en") === "zh"
    ? { up: red, down: green }
    : { up: green, down: red };
}

export function calculateBancorPriceDomain(values: Array<{ high: number; low: number }>): {
  high: number;
  low: number;
} {
  const finiteValues = values.filter((value) => Number.isFinite(value.high) && Number.isFinite(value.low));
  if (finiteValues.length === 0) return { high: 1, low: 0 };
  const rawHigh = Math.max(...finiteValues.map((value) => value.high));
  const rawLow = Math.min(...finiteValues.map((value) => value.low));
  const rawSpan = Math.max(rawHigh - rawLow, 0);
  // Active markets often move by only a few basis points. Padding by a fixed
  // percentage of the absolute price flattens those candles, so use the
  // visible range and only fall back to an absolute-price pad for a flat bar.
  const padding = rawSpan > 0
    ? Math.max(rawSpan * 0.12, Number.EPSILON)
    : Math.max(Math.abs(rawHigh) * 0.0025, 0.0000000001);
  return {
    high: rawHigh + padding,
    low: Math.max(0, rawLow - padding),
  };
}

export function scaleBancorPriceDomain(
  domain: { high: number; low: number },
  requestedScale: number,
): { high: number; low: number } {
  const scale = Number.isFinite(requestedScale)
    ? Math.max(0.5, Math.min(64, requestedScale))
    : 1;
  const midpoint = (domain.high + domain.low) / 2;
  const halfSpan = Math.max((domain.high - domain.low) / (2 * scale), Number.EPSILON);
  let low = midpoint - halfSpan;
  let high = midpoint + halfSpan;
  if (low < 0) {
    high -= low;
    low = 0;
  }
  return { high, low };
}

export function calculateBancorVisibleWindow(total: number, visible: number, offsetFromLatest: number): {
  end: number;
  maxOffset: number;
  offset: number;
  start: number;
} {
  const safeTotal = Math.max(0, Math.floor(total));
  const safeVisible = Math.max(1, Math.min(safeTotal || 1, Math.floor(visible)));
  const maxOffset = Math.max(0, safeTotal - safeVisible);
  const offset = Math.max(0, Math.min(maxOffset, Math.floor(offsetFromLatest)));
  const start = Math.max(0, safeTotal - safeVisible - offset);
  return { start, end: Math.min(safeTotal, start + safeVisible), maxOffset, offset };
}

export function calculateBancorDefaultVisibleCount(total: number): number {
  const safeTotal = Math.max(0, Math.floor(total));
  if (safeTotal <= BANCOR_MIN_VISIBLE_CANDLES) return safeTotal;
  if (safeTotal <= BANCOR_DEFAULT_VISIBLE_CANDLES + BANCOR_DRAG_HISTORY_HEADROOM) {
    return Math.max(BANCOR_MIN_VISIBLE_CANDLES, safeTotal - BANCOR_DRAG_HISTORY_HEADROOM);
  }
  return BANCOR_DEFAULT_VISIBLE_CANDLES;
}

export function calculateBancorCandleBodyWidth(slotWidth: number): number {
  if (!Number.isFinite(slotWidth) || slotWidth <= 0) return 1;
  // A 100-bar viewport needs sub-4px bodies on narrow screens; always keep
  // the body inside its time slot so adjacent candles remain distinguishable.
  return Math.max(1, Math.min(72, slotWidth * 0.78));
}

export function calculateBancorChartGeometry(viewportWidth: number): {
  plotRight: number;
  priceAxisX: number;
  width: number;
} {
  const width = Number.isFinite(viewportWidth) ? Math.max(320, Math.round(viewportWidth)) : 900;
  return {
    width,
    plotRight: width - 124,
    priceAxisX: width - 110,
  };
}

export function BancorPage({
  t,
  runtime,
  formatUnixDateTime,
  nniReady,
}: {
  t: Translate;
  runtime: BancorRuntime;
  formatUnixDateTime: (value?: number | null) => string;
  nniReady: boolean;
}) {
  const [side, setSide] = useState<"buy" | "sell">("sell");
  const [inputAmount, setInputAmount] = useState("");
  const {
    market,
    candles,
    account,
    marketTrades,
    quote,
    lastTrade,
    marketLoading,
    candlesLoading,
    candlesError,
    candleIntervalSeconds,
    accountLoading,
    marketTradesLoading,
    marketTradesError,
    quoteLoading,
    tradeLoading,
    error,
    message,
    fetchMarket,
    fetchCandles,
    changeCandleInterval,
    fetchAccount,
    fetchMarketTrades,
    preview,
    trade,
    clearQuote,
  } = runtime;
  const inputAsset = side === "buy" ? "USD" : "POINT";
  const outputAsset = side === "buy" ? "POINT" : "USD";
  const marketOpen = market?.status === "open";
  const tradingReady = marketOpen && nniReady;
  const inputErrorCode = inputAmount.trim()
    ? validateBancorTradeInput({ side, inputAmount, market, account })
    : null;
  const inputError = inputErrorCode
    ? formatBancorApiError(inputErrorCode, t, t("金额无法交易。", "This amount cannot be traded."))
    : null;
  const estimatedInputFee = market && inputAmount.trim()
    ? calculateBancorInputFee(inputAmount, market.fee_bps)
    : null;

  const changeSide = (next: "buy" | "sell") => {
    setSide(next);
    setInputAmount("");
    clearQuote();
  };
  const fillBalance = (next: "buy" | "sell", amount: string) => {
    setSide(next);
    setInputAmount(amount);
    clearQuote();
  };
  const confirmTrade = async () => {
    const result = await trade();
    if (result) setInputAmount("");
  };

  return (
    <div className="mx-auto grid w-full max-w-7xl gap-5 pb-10">
      <section className="theme-shadow-card p-5 sm:p-6">
        <div className="flex flex-wrap items-start justify-between gap-4">
          <div>
            <div className="flex items-center gap-2 text-xl font-semibold text-sky-200 sm:text-2xl">
              <TrendingUp className="h-5 w-5" />
              <span>{t("BANCOR储备曲线市场", "BANCOR reserve-curve market")}</span>
            </div>
            <p className="mt-3 max-w-3xl text-sm leading-6 text-white/60">
              {t(
                "强制流动性算法，每笔成交都会由本机设备单独签名，浏览器不会接触私钥。",
                "A forced-liquidity algorithm. Every trade is signed separately by this device; the browser never handles the private key.",
              )}
            </p>
          </div>
          <button
            type="button"
            className="theme-secondary-btn"
            disabled={marketLoading}
            onClick={() => void fetchMarket()}
          >
            <RefreshCw className={`h-4 w-4 ${marketLoading ? "animate-spin" : ""}`} />
            {t("刷新市场", "Refresh market")}
          </button>
        </div>
        <div className="mt-4 grid gap-2 sm:grid-cols-2 xl:grid-cols-4">
          <MetricCard
            label={t("市场储备", "Market reserves")}
            value={market ? `${market.point_reserve} POINT` : "—"}
            secondaryValue={market ? `${market.usd_reserve} USD` : undefined}
            detail={market ? undefined : t("等待读取", "Waiting to load")}
          />
          <MetricCard
            label={t("当前边际价格", "Current marginal price")}
            value={market ? `${market.marginal_price_usd_per_point} USD` : "—"}
            detail={t("每 1 POINT；实际成交价会随数量变化", "Per POINT; execution price changes with size")}
          />
          <MetricCard
            label={t("累计手续费", "Cumulative fees")}
            value={`${market?.fee_totals?.point_fee_amount ?? "0.0000"} POINT`}
            secondaryValue={`${market?.fee_totals?.usd_fee_amount ?? "0.0000"} USD`}
            detail={t("按支付资产分别累计", "Tracked separately by input asset")}
          />
          <MetricCard
            label={t("交易手续费", "Trading fee")}
            value={market ? `${(market.fee_bps / 100).toFixed(2)}%` : "—"}
            detail={t("买入从 USD 扣除，卖出从 POINT 扣除", "Charged in USD when buying and POINT when selling")}
          />
        </div>
      </section>

      {error ? <div className="rounded-xl border border-red-400/25 bg-red-500/10 px-4 py-3 text-sm text-red-100">{error}</div> : null}
      {message ? <div className="rounded-xl border border-emerald-400/25 bg-emerald-500/10 px-4 py-3 text-sm text-emerald-100">{message}</div> : null}
      {quote ? (
        <BancorQuoteDialog
          t={t}
          quote={quote}
          tradeLoading={tradeLoading}
          tradeError={error}
          onClose={clearQuote}
          onConfirm={() => void confirmTrade()}
        />
      ) : null}

      <section className="grid gap-5 lg:grid-cols-[minmax(0,2fr)_minmax(20rem,1fr)] lg:items-stretch">
        <section className="theme-shadow-card min-w-0 p-5 sm:p-6">
          <div className="flex flex-wrap items-center justify-between gap-3">
            <div className="flex min-w-0 items-center gap-2">
              <BarChart3 className="h-5 w-5 text-sky-300" />
              <h2 className="text-lg font-semibold text-white">{t("价格 K 线", "Price candlesticks")}</h2>
              <span className="whitespace-nowrap text-xs text-white/40">
                {t(
                  `每 ${BANCOR_CANDLE_AUTO_REFRESH_SECONDS} 秒自动刷新`,
                  `${BANCOR_CANDLE_AUTO_REFRESH_SECONDS}s auto-refresh`,
                )}
              </span>
            </div>
            <button
              type="button"
              className="theme-icon-btn"
              disabled={candlesLoading}
              onClick={() => void fetchCandles(candleIntervalSeconds)}
              title={t("立即刷新 K 线", "Refresh candlesticks now")}
            >
              <RefreshCw className={`h-4 w-4 ${candlesLoading ? "animate-spin" : ""}`} />
            </button>
          </div>
        <div className="mt-4 flex flex-wrap gap-2" aria-label={t("K 线周期", "Candlestick interval")}>
          {BANCOR_CANDLE_INTERVALS.map((interval) => (
            <button
              key={interval.seconds}
              type="button"
              aria-pressed={candleIntervalSeconds === interval.seconds}
              className={`rounded-lg border px-3 py-1.5 text-xs font-medium transition ${
                candleIntervalSeconds === interval.seconds
                  ? "border-sky-300/35 bg-sky-400/15 text-sky-100"
                  : "border-white/8 bg-white/[0.025] text-white/50 hover:text-white/75"
              }`}
              disabled={candlesLoading}
              onClick={() => void changeCandleInterval(interval.seconds)}
            >
              {t(interval.zh, interval.en)}
            </button>
          ))}
        </div>
        {candlesError ? (
          <div className="mt-4 rounded-xl border border-amber-400/20 bg-amber-400/5 px-4 py-3 text-sm text-amber-100">
            {candlesError}
          </div>
        ) : null}
        <div className="mt-4">
          {candlesLoading && !candles ? (
            <div className="flex min-h-64 items-center justify-center text-sm text-white/40">
              {t("正在读取成交数据...", "Loading trade data...")}
            </div>
          ) : candles?.candles.length ? (
            <CandleChart
              key={candleIntervalSeconds}
              candles={candles.candles}
              intervalSeconds={candles.interval_seconds}
              priceDecimalPlaces={candles.price_decimal_places}
              formatUnixDateTime={formatUnixDateTime}
              t={t}
            />
          ) : (
            <div className="flex min-h-64 flex-col items-center justify-center rounded-xl border border-dashed border-white/10 px-5 text-center">
              <BarChart3 className="h-8 w-8 text-white/35" />
              <p className="mt-3 text-sm text-white/55">{t("暂无成交，首笔成交后显示 K 线。", "No trades yet. Candlesticks will appear after the first trade.")}</p>
            </div>
          )}
        </div>
        </section>

        <div className="theme-shadow-card h-full p-5 sm:p-6">
          <div className="flex items-center gap-2">
            <ArrowDownUp className="h-5 w-5 text-sky-300" />
            <h2 className="text-lg font-semibold text-white">{t("交易", "Trade")}</h2>
          </div>
          <p className="mt-2 text-sm text-white/55">
            {t("先查看报价，确认数量和预计到账后再签名成交。", "Preview the quote first, then review the amounts before signing the trade.")}
          </p>

          <div className="mt-5 rounded-xl border border-white/8 bg-white/[0.025] p-4">
            <div className="flex items-center justify-between gap-3">
              <div className="flex items-center gap-2">
                <WalletCards className="h-4 w-4 text-sky-300" />
                <h3 className="text-sm font-medium text-white/80">{t("我的余额", "My balances")}</h3>
              </div>
              <button type="button" className="theme-icon-btn" disabled={accountLoading || !nniReady} onClick={() => void fetchAccount()} title={t("签名刷新余额", "Sign to refresh balances")}>
                <RefreshCw className={`h-4 w-4 ${accountLoading ? "animate-spin" : ""}`} />
              </button>
            </div>
            <div className="mt-3 grid gap-3 sm:grid-cols-2">
              <BalanceLine
                label="POINT"
                value={account?.point_balance ?? "—"}
                disabled={!account}
                actionLabel={t("点击填入全部 POINT 余额", "Use the full POINT balance")}
                onClick={() => account && fillBalance("sell", account.point_balance)}
              />
              <BalanceLine
                label="USD"
                value={account?.usd_balance ?? "—"}
                disabled={!account}
                actionLabel={t("点击填入全部 USD 余额", "Use the full USD balance")}
                onClick={() => account && fillBalance("buy", account.usd_balance)}
              />
            </div>
            <p className="mt-3 text-xs leading-5 text-white/40">
              {t("读取私人余额需要一次新的设备签名。", "Reading private balances requires a fresh device signature.")}
            </p>
            {account?.device_pubkey ? (
              <p className="mt-2 break-all text-xs text-white/35">
                {t("设备：", "Device: ")}{account.device_pubkey.slice(0, 12)}…{account.device_pubkey.slice(-8)}
              </p>
            ) : null}
          </div>

          <div className="mt-5 grid grid-cols-2 rounded-xl bg-white/5 p-1">
            {(["sell", "buy"] as const).map((value) => (
              <button
                key={value}
                type="button"
                className={`rounded-lg px-4 py-2.5 text-sm font-medium transition ${side === value ? "bg-sky-400/20 text-sky-100" : "text-white/55 hover:text-white/80"}`}
                onClick={() => changeSide(value)}
              >
                {value === "sell" ? t("卖出 POINT", "Sell POINT") : t("买入 POINT", "Buy POINT")}
              </button>
            ))}
          </div>

          <label className="mt-5 block text-sm text-white/70">
            {t("支付数量", "Amount to pay")} ({inputAsset})
            <div className="mt-2 flex items-center rounded-xl border border-white/10 bg-black/10 px-3 focus-within:border-sky-400/50">
              <input
                value={inputAmount}
                inputMode="decimal"
                placeholder="0.0000"
                className="min-w-0 flex-1 bg-transparent py-3 text-lg text-white outline-none placeholder:text-white/25"
                onChange={(event) => {
                  setInputAmount(event.target.value);
                  clearQuote();
                }}
              />
              <span className="text-sm font-semibold text-white/55">{inputAsset}</span>
            </div>
          </label>
          {inputError ? (
            <p className="mt-2 text-xs leading-5 text-red-200" role="alert">{inputError}</p>
          ) : null}
          {estimatedInputFee ? (
            <p className="mt-2 text-xs leading-5 text-white/50">
              {t("预计手续费", "Estimated fee")}：{estimatedInputFee} {inputAsset}
              {market ? ` · ${(market.fee_bps / 100).toFixed(2)}%` : ""}
            </p>
          ) : null}

          <button
            type="button"
            className="theme-primary-btn mt-4 w-full justify-center"
            disabled={!tradingReady || !inputAmount.trim() || Boolean(inputErrorCode) || quoteLoading || tradeLoading}
            onClick={() => void preview(side, inputAmount)}
          >
            {quoteLoading ? t("正在计算...", "Calculating...") : t("查看报价", "Preview quote")}
          </button>
          {!marketOpen ? (
            <p className="mt-3 text-xs leading-5 text-amber-200/80">
              {t("管理员尚未开启市场，因此现在只能查看储备和账户。", "The market is not enabled by the administrator, so only reserves and account data are available.")}
            </p>
          ) : null}
          {marketOpen && !nniReady ? (
            <p className="mt-3 text-xs leading-5 text-amber-200/80">
              {t(
                "请先在 NNI 页面加入网络，并确认本机签名设备可用；完成后才能获取报价和提交交易。",
                "Join the network on the NNI page and confirm that this device can sign before requesting quotes or trading.",
              )}
            </p>
          ) : null}

          {lastTrade ? (
            <div className="mt-4 rounded-xl border border-emerald-400/20 bg-emerald-400/5 p-4 text-sm text-emerald-50">
              {t("最近成交：", "Latest trade: ")}
              {lastTrade.trade.input_amount} {lastTrade.trade.input_asset} → {lastTrade.trade.output_amount} {lastTrade.trade.output_asset}
            </div>
          ) : null}
        </div>
      </section>

      <section className="grid gap-5 lg:grid-cols-2 lg:items-start">
        <article className="theme-shadow-card p-5 sm:p-6">
          <div className="flex items-center justify-between gap-3">
            <div>
              <h2 className="text-lg font-semibold text-white">{t("我的成交记录", "My trade history")}</h2>
              <p className="mt-1 text-sm text-white/50">{t("这里只显示当前设备公钥签署的交易。", "Only trades signed by this device key are shown.")}</p>
            </div>
            <span className="text-xs text-white/40">{account?.total ?? 0} {t("笔", "trades")}</span>
          </div>
          <div className="mt-4 grid gap-2">
            {account?.trades.length ? account.trades.map((record) => (
              <div key={record.trade_id} className="grid gap-2 rounded-xl border border-white/8 bg-white/[0.025] px-4 py-3 text-sm sm:grid-cols-[1fr_auto_auto] sm:items-center">
                <div>
                  <span className="font-medium text-white/85">{record.side === "buy" ? t("买入 POINT", "Buy POINT") : t("卖出 POINT", "Sell POINT")}</span>
                  <p className="mt-1 text-xs text-white/40">{formatUnixDateTime(record.created_at_unix)}</p>
                </div>
                <span className="text-white/55">{record.input_amount} {record.input_asset}</span>
                <span className="font-medium text-emerald-200">+ {record.output_amount} {record.output_asset}</span>
              </div>
            )) : (
              <div className="rounded-xl border border-dashed border-white/10 px-4 py-8 text-center text-sm text-white/40">
                {account ? t("还没有成交记录。", "No trades yet.") : t("点击“交易”卡片内的余额刷新按钮读取账户。", "Use the balance refresh button in the Trade card to load the account.")}
              </div>
            )}
          </div>
          {account && account.total_pages > 1 ? (
            <div className="mt-4 flex items-center justify-between gap-3">
              <button type="button" className="theme-secondary-btn" disabled={accountLoading || account.page <= 1} onClick={() => void fetchAccount(account.page - 1)}>
                {t("上一页", "Previous")}
              </button>
              <span className="text-xs text-white/45">{t("第", "Page")} {account.page} / {account.total_pages} {t("页", "")}</span>
              <button type="button" className="theme-secondary-btn" disabled={accountLoading || account.page >= account.total_pages} onClick={() => void fetchAccount(account.page + 1)}>
                {t("下一页", "Next")}
              </button>
            </div>
          ) : null}
        </article>

        <article className="theme-shadow-card p-5 sm:p-6">
          <div className="flex items-center justify-between gap-3">
            <div>
              <h2 className="text-lg font-semibold text-white">{t("市场成交记录", "Market trade history")}</h2>
              <p className="mt-1 text-sm text-white/50">{t("展示全市场成交，其他设备公钥已打码。", "Shows market-wide trades with device public keys masked.")}</p>
            </div>
            <div className="flex items-center gap-2">
              <span className="text-xs text-white/40">{marketTrades?.total ?? 0} {t("笔", "trades")}</span>
              <button
                type="button"
                className="theme-icon-btn"
                aria-label={t("刷新市场成交记录", "Refresh market trades")}
                disabled={marketTradesLoading}
                onClick={() => void fetchMarketTrades(marketTrades?.page ?? 1)}
              >
                <RefreshCw className={`h-4 w-4 ${marketTradesLoading ? "animate-spin" : ""}`} />
              </button>
            </div>
          </div>
          {marketTradesError ? <p className="mt-3 text-sm text-red-200" role="alert">{marketTradesError}</p> : null}
          <div className="mt-4 grid gap-2">
            {marketTrades?.trades.length ? marketTrades.trades.map((record) => (
              <div key={record.trade_id} className="grid gap-2 rounded-xl border border-white/8 bg-white/[0.025] px-4 py-3 text-sm sm:grid-cols-[minmax(0,1fr)_auto_auto] sm:items-center">
                <div className="min-w-0">
                  <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
                    <span className="font-medium text-white/85">{record.side === "buy" ? t("买入 POINT", "Buy POINT") : t("卖出 POINT", "Sell POINT")}</span>
                    <span className="max-w-full truncate font-mono text-[11px] text-white/35">{record.device_pubkey_masked}</span>
                  </div>
                  <p className="mt-1 text-xs text-white/40">{formatUnixDateTime(record.created_at_unix)}</p>
                </div>
                <span className="text-white/55">{record.input_amount} {record.input_asset}</span>
                <span className="font-medium text-emerald-200">+ {record.output_amount} {record.output_asset}</span>
              </div>
            )) : (
              <div className="rounded-xl border border-dashed border-white/10 px-4 py-8 text-center text-sm text-white/40">
                {marketTradesLoading ? t("正在读取市场成交记录…", "Loading market trades…") : t("市场暂时还没有成交记录。", "No market trades yet.")}
              </div>
            )}
          </div>
          {marketTrades && marketTrades.total_pages > 1 ? (
            <div className="mt-4 flex items-center justify-between gap-3">
              <button type="button" className="theme-secondary-btn" disabled={marketTradesLoading || marketTrades.page <= 1} onClick={() => void fetchMarketTrades(marketTrades.page - 1)}>
                {t("上一页", "Previous")}
              </button>
              <span className="text-xs text-white/45">{t("第", "Page")} {marketTrades.page} / {marketTrades.total_pages} {t("页", "")}</span>
              <button type="button" className="theme-secondary-btn" disabled={marketTradesLoading || marketTrades.page >= marketTrades.total_pages} onClick={() => void fetchMarketTrades(marketTrades.page + 1)}>
                {t("下一页", "Next")}
              </button>
            </div>
          ) : null}
        </article>
      </section>

      <BancorFormulaCard t={t} market={market} />
    </div>
  );
}

function BancorFormulaCard({ t, market }: { t: Translate; market: BancorRuntime["market"] }) {
  return (
    <section className="theme-shadow-card p-5 sm:p-6" aria-label={t("BANCOR 储备曲线公式", "BANCOR reserve-curve formula")}>
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div>
          <p className="text-xs font-medium uppercase tracking-wide text-sky-300/75">BANCOR</p>
          <h2 className="mt-1 text-lg font-semibold text-white">{t("储备曲线交易公式", "Reserve-curve trading formula")}</h2>
        </div>
        <span className="rounded-md border border-white/8 bg-white/[0.035] px-2 py-1 text-xs text-white/45">
          {t("当前手续费", "Current fee")}：{market ? `${(market.fee_bps / 100).toFixed(2)}%` : "—"}
        </span>
      </div>
      <div className="mt-4 flex flex-col items-center gap-4 overflow-x-auto py-2 font-serif text-xl text-sky-100 sm:text-2xl" role="math">
        <div className="whitespace-nowrap" aria-label={t("有效支付量等于支付量减去手续费", "Effective input equals input amount minus fee")}>
          <var>x</var><sub className="text-xs not-italic sm:text-sm">e</sub>
          <span className="mx-2">=</span>
          <var>x</var>
          <span className="mx-2">−</span>
          <var>F</var>(<var>x</var>)
        </div>
        <div className="flex items-center whitespace-nowrap" aria-label={t("到账量等于有效支付量乘以输出储备，除以输入储备加有效支付量，然后向下取整", "Output equals effective input times output reserve, divided by input reserve plus effective input, rounded down")}>
          <var>y</var>
          <span className="mx-2">= ⌊</span>
          <span className="inline-grid text-center align-middle">
            <span className="border-b border-sky-100/55 px-3 pb-1">
              <var>x</var><sub className="text-xs not-italic sm:text-sm">e</sub>
              <span className="mx-2">×</span>
              <var>R</var><sub className="text-xs not-italic sm:text-sm">out</sub>
            </span>
            <span className="px-3 pt-1">
              <var>R</var><sub className="text-xs not-italic sm:text-sm">in</sub>
              <span className="mx-2">+</span>
              <var>x</var><sub className="text-xs not-italic sm:text-sm">e</sub>
            </span>
          </span>
          <span className="ml-2">⌋</span>
        </div>
      </div>
      <dl className="mt-4 grid gap-x-5 gap-y-1.5 border-t border-white/8 pt-3 text-xs leading-5 text-white/45 sm:grid-cols-2">
        <div><dt className="inline font-mono text-white/65">x</dt><dd className="inline"> — {t("支付量", "input amount")}</dd></div>
        <div><dt className="inline font-mono text-white/65">F(x)</dt><dd className="inline"> — {t("按当前费率收取的输入资产手续费", "input-asset fee charged at the current rate")}</dd></div>
        <div><dt className="inline font-mono text-white/65">xₑ</dt><dd className="inline"> — {t("扣除手续费后的有效支付量", "effective input after fees")}</dd></div>
        <div><dt className="inline font-mono text-white/65">Rᵢₙ</dt><dd className="inline"> — {t("输入资产的市场储备", "market reserve of the input asset")}</dd></div>
        <div><dt className="inline font-mono text-white/65">Rₒᵤₜ</dt><dd className="inline"> — {t("输出资产的市场储备", "market reserve of the output asset")}</dd></div>
        <div><dt className="inline font-mono text-white/65">y</dt><dd className="inline"> — {t("向下取整后的实际到账量", "actual output after rounding down")}</dd></div>
      </dl>
      <div className="mt-3 grid gap-2 text-xs leading-5 text-white/45 sm:grid-cols-2">
        <p>{t("买入 POINT：输入储备是 USD，输出储备是 POINT。", "Buy POINT: the input reserve is USD and the output reserve is POINT.")}</p>
        <p>{t("卖出 POINT：输入储备是 POINT，输出储备是 USD。", "Sell POINT: the input reserve is POINT and the output reserve is USD.")}</p>
      </div>
      <p className="mt-2 text-xs leading-5 text-white/40">
        {t(
          "⌊ ⌋ 表示按最小单位向下取整；本市场的 POINT 与 USD 均保留 4 位小数。交易数量越大，对储备比例和成交价格的影响越明显。",
          "⌊ ⌋ rounds down to the smallest unit. POINT and USD both use four decimal places. Larger trades have a greater effect on the reserve ratio and execution price.",
        )}
      </p>
    </section>
  );
}

export function CandleChart({
  candles,
  intervalSeconds,
  priceDecimalPlaces,
  formatUnixDateTime,
  t,
}: {
  candles: NniBancorCandle[];
  intervalSeconds: number;
  priceDecimalPlaces: number;
  formatUnixDateTime: (value?: number | null) => string;
  t: Translate;
}) {
  const chartRef = useRef<HTMLDivElement>(null);
  const dragRef = useRef<{ pointerId: number; startOffset: number; startX: number } | null>(null);
  const previousCandleCountRef = useRef(candles.length);
  const [viewportWidth, setViewportWidth] = useState(900);
  const [visibleCountOverride, setVisibleCountOverride] = useState<number | null>(null);
  const [verticalZoom, setVerticalZoom] = useState(1);
  const [offsetFromLatest, setOffsetFromLatest] = useState(0);
  const [hoveredIndex, setHoveredIndex] = useState<number | null>(null);
  const [isDragging, setIsDragging] = useState(false);
  const geometry = calculateBancorChartGeometry(viewportWidth);
  const width = geometry.width;
  const height = 396;
  const plotLeft = 18;
  const plotRight = geometry.plotRight;
  const priceAxisX = geometry.priceAxisX;
  const priceTop = 18;
  const priceBottom = 274;
  const volumeTop = 296;
  const volumeBottom = 356;
  const timeAxisY = 382;
  const allValues = candles.map((candle) => ({
    candle,
    open: Number(candle.open),
    high: Number(candle.high),
    low: Number(candle.low),
    close: Number(candle.close),
    pointVolume: Number(candle.point_volume),
  }));

  useEffect(() => {
    const element = chartRef.current;
    if (!element || typeof ResizeObserver === "undefined") return;
    const updateWidth = () => setViewportWidth(Math.max(element.clientWidth, 320));
    updateWidth();
    const observer = new ResizeObserver(updateWidth);
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    const previousCount = previousCandleCountRef.current;
    if (candles.length > previousCount && offsetFromLatest > 0) {
      setOffsetFromLatest((current) => current + candles.length - previousCount);
    }
    previousCandleCountRef.current = candles.length;
  }, [candles.length, offsetFromLatest]);

  const defaultVisibleCount = calculateBancorDefaultVisibleCount(allValues.length);
  const requestedVisibleCount = visibleCountOverride ?? defaultVisibleCount;
  const visibleCount = Math.max(1, Math.min(allValues.length, requestedVisibleCount));
  const visibleWindow = calculateBancorVisibleWindow(allValues.length, visibleCount, offsetFromLatest);
  const values = allValues.slice(visibleWindow.start, visibleWindow.end);
  const autoPriceDomain = calculateBancorPriceDomain(values);
  const priceDomain = scaleBancorPriceDomain(autoPriceDomain, verticalZoom);
  const priceHigh = priceDomain.high;
  const priceLow = priceDomain.low;
  const priceSpan = Math.max(priceHigh - priceLow, Number.EPSILON);
  const maxVolume = Math.max(...values.map((value) => value.pointVolume), 1);
  const step = (plotRight - plotLeft) / Math.max(values.length, 1);
  const bodyWidth = calculateBancorCandleBodyWidth(step);
  const yForPrice = (price: number) => priceTop + ((priceHigh - price) / priceSpan) * (priceBottom - priceTop);
  const last = values.at(-1)!;
  const focused = hoveredIndex === null ? last : values[Math.min(hoveredIndex, values.length - 1)] ?? last;
  const tickIndexes = new Set([0, Math.floor((values.length - 1) / 2), values.length - 1]);
  const palette = resolveBancorCandlePalette(t);
  const latestColor = last.close >= last.open ? palette.up : palette.down;
  const showMinuteCloseLine = intervalSeconds === 60;
  const minuteCloseLinePoints = showMinuteCloseLine
    ? values
      .map((value, index) => `${plotLeft + step * (index + 0.5)},${yForPrice(value.close)}`)
      .join(" ")
    : "";
  const maxRequestedVisibleCount = Math.min(160, allValues.length);

  const clampOffset = (value: number) => Math.max(0, Math.min(visibleWindow.maxOffset, value));
  const panBy = (candlesToOlder: number) => {
    setOffsetFromLatest((current) => clampOffset(current + candlesToOlder));
    setHoveredIndex(null);
  };
  const zoomBy = (delta: number) => {
    const current = visibleCountOverride ?? defaultVisibleCount;
    const next = Math.max(Math.min(BANCOR_MIN_VISIBLE_CANDLES, allValues.length), Math.min(maxRequestedVisibleCount, current + delta));
    setVisibleCountOverride(next);
    setHoveredIndex(null);
  };
  const verticalZoomBy = (factor: number) => {
    setVerticalZoom((current) => Math.max(0.5, Math.min(64, current * factor)));
    setHoveredIndex(null);
  };
  const updateHoveredCandle = (event: ReactPointerEvent<HTMLDivElement>) => {
    const bounds = event.currentTarget.getBoundingClientRect();
    const svgX = ((event.clientX - bounds.left) / Math.max(bounds.width, 1)) * width;
    if (svgX < plotLeft || svgX > plotRight) {
      setHoveredIndex(null);
      return;
    }
    const index = Math.max(0, Math.min(values.length - 1, Math.floor((svgX - plotLeft) / step)));
    setHoveredIndex(index);
  };
  const handlePointerDown = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    event.preventDefault();
    event.currentTarget.setPointerCapture(event.pointerId);
    dragRef.current = { pointerId: event.pointerId, startOffset: visibleWindow.offset, startX: event.clientX };
    setIsDragging(true);
  };
  const handlePointerMove = (event: ReactPointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current;
    if (!drag) {
      updateHoveredCandle(event);
      return;
    }
    const pixelsPerCandle = Math.max((viewportWidth * 0.6) / Math.max(visibleCount, 1), 10);
    const candlesToOlder = Math.round((event.clientX - drag.startX) / pixelsPerCandle);
    setOffsetFromLatest(clampOffset(drag.startOffset + candlesToOlder));
    setHoveredIndex(null);
  };
  const finishPointerDrag = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (dragRef.current?.pointerId === event.pointerId) {
      dragRef.current = null;
      setIsDragging(false);
      if (event.currentTarget.hasPointerCapture(event.pointerId)) {
        event.currentTarget.releasePointerCapture(event.pointerId);
      }
    }
  };
  const handleWheel = (event: ReactWheelEvent<HTMLDivElement>) => {
    if (event.altKey) {
      event.preventDefault();
      verticalZoomBy(event.deltaY > 0 ? 1 / 1.35 : 1.35);
      return;
    }
    if (event.ctrlKey || event.metaKey) {
      event.preventDefault();
      zoomBy(event.deltaY > 0 ? 4 : -4);
      return;
    }
    if (Math.abs(event.deltaX) > Math.abs(event.deltaY) || event.shiftKey) {
      event.preventDefault();
      const horizontalDelta = Math.abs(event.deltaX) > 0 ? event.deltaX : event.deltaY;
      panBy(horizontalDelta > 0 ? -2 : 2);
    }
  };
  const handleKeyDown = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    if (event.key === "ArrowLeft") {
      event.preventDefault();
      panBy(1);
    } else if (event.key === "ArrowRight") {
      event.preventDefault();
      panBy(-1);
    } else if (event.key === "Home") {
      event.preventDefault();
      setOffsetFromLatest(visibleWindow.maxOffset);
    } else if (event.key === "End") {
      event.preventDefault();
      setOffsetFromLatest(0);
    }
  };
  const hoveredX = hoveredIndex === null ? null : plotLeft + step * (hoveredIndex + 0.5);
  const hoveredY = hoveredIndex === null ? null : yForPrice(focused.close);

  return (
    <div>
      <div className="mb-2 flex flex-wrap items-center justify-between gap-2 text-xs text-white/45">
        <div className="min-w-0">
          <span className="text-white/65">{formatUnixDateTime(focused.candle.bucket_start_unix)}</span>
          <span className="ml-3">O {focused.candle.open} · H {focused.candle.high} · L {focused.candle.low} · C {focused.candle.close}</span>
          <span className="ml-3">VOL {focused.candle.point_volume} POINT</span>
        </div>
        <span>
          {visibleWindow.maxOffset > 0
            ? t("左右拖动查看历史；Ctrl+滚轮横向缩放，Alt+滚轮纵向缩放", "Drag for history; Ctrl+wheel zooms time, Alt+wheel zooms price")
            : t("全部真实 K 线已显示，暂无更多历史", "All real-trade candles are visible; no older history is available")}
        </span>
      </div>
      <div
        ref={chartRef}
        className={`relative overflow-hidden rounded-xl border border-white/8 bg-black/10 outline-none transition ${isDragging ? "cursor-grabbing ring-1 ring-sky-400/25" : "cursor-grab focus:ring-1 focus:ring-sky-400/35"}`}
        style={{ touchAction: "pan-y" }}
        role="group"
        tabIndex={0}
        aria-label={t("可横向与纵向缩放的 POINT 对 USD 真实成交 K 线图", "Horizontally and vertically zoomable real-trade POINT to USD candlestick chart")}
        onKeyDown={handleKeyDown}
        onPointerDown={handlePointerDown}
        onPointerMove={handlePointerMove}
        onPointerUp={finishPointerDrag}
        onPointerCancel={finishPointerDrag}
        onPointerLeave={() => {
          if (!dragRef.current) setHoveredIndex(null);
        }}
        onWheel={handleWheel}
      >
        <svg
          viewBox={`0 0 ${width} ${height}`}
          className="block h-auto min-h-72 w-full select-none"
          role="img"
          aria-label={t("POINT 对 USD 的真实成交 K 线图", "Real-trade POINT to USD candlestick chart")}
        >
          {[0, 0.25, 0.5, 0.75, 1].map((ratio) => {
            const y = priceTop + ratio * (priceBottom - priceTop);
            const label = (priceHigh - ratio * priceSpan).toFixed(priceDecimalPlaces);
            return (
              <g key={ratio}>
                <line x1={plotLeft} y1={y} x2={plotRight} y2={y} stroke="rgba(255,255,255,0.075)" strokeDasharray="3 6" />
                <text x={priceAxisX} y={y + 4} fill="rgba(255,255,255,0.42)" fontSize="11">{label}</text>
              </g>
            );
          })}
          {[0.25, 0.5, 0.75].map((ratio) => {
            const x = plotLeft + ratio * (plotRight - plotLeft);
            return <line key={ratio} x1={x} y1={priceTop} x2={x} y2={volumeBottom} stroke="rgba(255,255,255,0.05)" strokeDasharray="3 7" />;
          })}
          <line x1={plotLeft} y1={volumeTop - 10} x2={plotRight} y2={volumeTop - 10} stroke="rgba(255,255,255,0.08)" />
          <line x1={plotLeft} y1={yForPrice(last.close)} x2={plotRight} y2={yForPrice(last.close)} stroke={latestColor.stroke} strokeOpacity="0.45" strokeDasharray="5 5" />
          {showMinuteCloseLine && values.length > 1 ? (
            <polyline
              data-bancor-chart-layer="one-minute-close-line"
              points={minuteCloseLinePoints}
              fill="none"
              stroke="#7dd3fc"
              strokeOpacity="0.88"
              strokeWidth="1.75"
              strokeLinecap="round"
              strokeLinejoin="round"
              vectorEffect="non-scaling-stroke"
              pointerEvents="none"
            />
          ) : null}
          {values.map((value, index) => {
            const x = plotLeft + step * (index + 0.5);
            const up = value.close >= value.open;
            const color = up ? palette.up : palette.down;
            const hasTrades = value.candle.has_trades ?? (value.candle.trade_count > 0);
            const showCandleBody = !showMinuteCloseLine || hasTrades;
            const openY = yForPrice(value.open);
            const closeY = yForPrice(value.close);
            const bodyTop = Math.min(openY, closeY);
            const bodyHeight = Math.max(Math.abs(openY - closeY), 2);
            const bodyBottom = bodyTop + bodyHeight;
            const highY = yForPrice(value.high);
            const lowY = yForPrice(value.low);
            const volumeHeight = (value.pointVolume / maxVolume) * (volumeBottom - volumeTop);
            return (
              <g key={`${value.candle.bucket_start_unix}-${index}`}>
                <title>{`${formatUnixDateTime(value.candle.bucket_start_unix)} · O ${value.candle.open} · H ${value.candle.high} · L ${value.candle.low} · C ${value.candle.close} · ${value.candle.point_volume} POINT · ${value.candle.trade_count} ${t("笔", "trades")}`}</title>
                {showCandleBody && highY < bodyTop ? <line x1={x} y1={highY} x2={x} y2={bodyTop} stroke={color.stroke} strokeWidth="1.5" /> : null}
                {showCandleBody && bodyBottom < lowY ? <line x1={x} y1={bodyBottom} x2={x} y2={lowY} stroke={color.stroke} strokeWidth="1.5" /> : null}
                {showCandleBody ? <rect data-bancor-candle-body="true" x={x - bodyWidth / 2} y={bodyTop} width={bodyWidth} height={bodyHeight} rx="1" fill={color.stroke} stroke={color.stroke} strokeWidth="1.2" /> : null}
                <rect x={x - bodyWidth / 2} y={volumeBottom - volumeHeight} width={bodyWidth} height={volumeHeight} rx="1" fill={color.volumeFill} />
                {tickIndexes.has(index) ? (
                  <text x={x} y={timeAxisY} textAnchor="middle" fill="rgba(255,255,255,0.38)" fontSize="10">
                    {formatUnixDateTime(value.candle.bucket_start_unix)}
                  </text>
                ) : null}
              </g>
            );
          })}
          {hoveredX !== null && hoveredY !== null ? (
            <g pointerEvents="none">
              <line x1={hoveredX} y1={priceTop} x2={hoveredX} y2={volumeBottom} stroke="rgba(255,255,255,0.38)" strokeDasharray="4 5" />
              <line x1={plotLeft} y1={hoveredY} x2={plotRight} y2={hoveredY} stroke="rgba(255,255,255,0.38)" strokeDasharray="4 5" />
              <rect x={priceAxisX - 4} y={hoveredY - 10} width="96" height="20" rx="3" fill="rgba(15,23,42,0.96)" />
              <text x={priceAxisX + 3} y={hoveredY + 4} fill="rgba(255,255,255,0.88)" fontSize="11">{focused.candle.close}</text>
            </g>
          ) : null}
          <rect x={priceAxisX - 4} y={yForPrice(last.close) - 10} width="96" height="20" rx="3" fill={latestColor.fill} stroke={latestColor.stroke} strokeWidth="1" />
          <text x={priceAxisX + 3} y={yForPrice(last.close) + 4} fill={latestColor.stroke} fontSize="11">{last.candle.close}</text>
          <text x={plotLeft} y={volumeTop + 5} fill="rgba(255,255,255,0.38)" fontSize="10">VOL · POINT</text>
        </svg>
      </div>
      <div className="mt-2 flex flex-wrap items-center justify-between gap-2 text-xs text-white/45">
        <span>
          {t("当前显示", "Showing")} {visibleWindow.start + 1}–{visibleWindow.end} / {allValues.length}
        </span>
        <div className="flex flex-wrap items-center justify-end gap-1">
          <button type="button" className="theme-icon-btn h-8 w-8" disabled={visibleWindow.offset >= visibleWindow.maxOffset} onClick={() => panBy(Math.max(1, Math.floor(visibleCount / 2)))} title={t("查看更早 K 线", "View older candles")}>
            <ChevronLeft className="h-4 w-4" />
          </button>
          <span className="ml-1 text-[11px] text-white/35">{t("横向", "Time")}</span>
          <button type="button" className="theme-icon-btn h-8 w-8" disabled={visibleCount >= maxRequestedVisibleCount} onClick={() => zoomBy(6)} title={t("横向缩小，显示更多 K 线", "Zoom time out to show more candles")}>
            <Minus className="h-4 w-4" />
          </button>
          <button type="button" className="theme-icon-btn h-8 w-8" disabled={visibleCount <= Math.min(BANCOR_MIN_VISIBLE_CANDLES, allValues.length)} onClick={() => zoomBy(-6)} title={t("横向放大，显示更少 K 线", "Zoom time in to show fewer candles")}>
            <Plus className="h-4 w-4" />
          </button>
          <span className="ml-1 text-[11px] text-white/35">{t("纵向", "Price")}</span>
          <button type="button" className="theme-icon-btn h-8 w-8" disabled={verticalZoom <= 0.5} onClick={() => verticalZoomBy(1 / 1.5)} title={t("缩小价格波动", "Zoom price out")}>
            <Minus className="h-4 w-4" />
          </button>
          <button type="button" className="theme-icon-btn h-8 w-8" disabled={verticalZoom >= 64} onClick={() => verticalZoomBy(1.5)} title={t("放大价格波动", "Zoom price in")}>
            <Plus className="h-4 w-4" />
          </button>
          <button type="button" className="theme-secondary-btn min-h-8 px-2.5 py-1 text-xs" disabled={Math.abs(verticalZoom - 1) < 0.0001} onClick={() => setVerticalZoom(1)}>
            {t("自动纵轴", "Auto price")}
          </button>
          <button type="button" className="theme-secondary-btn min-h-8 px-2.5 py-1 text-xs" disabled={visibleWindow.offset === 0} onClick={() => setOffsetFromLatest(0)}>
            {t("回到最新", "Latest")}
          </button>
          <button type="button" className="theme-icon-btn h-8 w-8" disabled={visibleWindow.offset === 0} onClick={() => panBy(-Math.max(1, Math.floor(visibleCount / 2)))} title={t("查看更新 K 线", "View newer candles")}>
            <ChevronRight className="h-4 w-4" />
          </button>
        </div>
      </div>
    </div>
  );
}

export function BancorQuoteDialog({
  t,
  quote,
  tradeLoading,
  tradeError,
  onClose,
  onConfirm,
}: {
  t: Translate;
  quote: NniBancorQuoteResponse;
  tradeLoading: boolean;
  tradeError: string | null;
  onClose: () => void;
  onConfirm: () => void;
}) {
  const confirmButtonRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    const frame = window.requestAnimationFrame(() => confirmButtonRef.current?.focus());
    const previousOverflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    const onKeyDown = (event: globalThis.KeyboardEvent) => {
      if (event.key === "Escape" && !tradeLoading) onClose();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.cancelAnimationFrame(frame);
      window.removeEventListener("keydown", onKeyDown);
      document.body.style.overflow = previousOverflow;
    };
  }, [onClose, tradeLoading]);

  return (
    <div
      className="fixed inset-0 z-[120] flex items-center justify-center bg-black/55 p-4 backdrop-blur-sm"
      onMouseDown={(event) => {
        if (!tradeLoading && event.target === event.currentTarget) onClose();
      }}
    >
      <section
        role="dialog"
        aria-modal="true"
        aria-labelledby="bancor-quote-dialog-title"
        aria-describedby="bancor-quote-dialog-description"
        className="theme-card w-full max-w-lg border p-0 shadow-2xl"
      >
        <header className="flex items-start gap-3 border-b border-white/10 px-5 py-4">
          <span className="mt-0.5 text-sky-300"><ShieldCheck className="h-5 w-5" /></span>
          <div className="min-w-0 flex-1">
            <h2 id="bancor-quote-dialog-title" className="text-base font-semibold text-white">
              {t("查看报价并确认交易", "Review quote and confirm trade")}
            </h2>
            <p id="bancor-quote-dialog-description" className="mt-1 text-sm leading-6 text-white/55">
              {t("请核对支付、到账和手续费，再使用本机设备签名。", "Check the payment, output, and fee before signing with this device.")}
            </p>
          </div>
          <button
            type="button"
            className="theme-icon-btn h-8 w-8 shrink-0"
            disabled={tradeLoading}
            onClick={onClose}
            title={t("关闭报价", "Close quote")}
          >
            <X className="h-4 w-4" />
          </button>
        </header>

        <div className="grid gap-4 px-5 py-5 text-sm sm:grid-cols-2">
          <QuoteLine label={t("支付", "Pay")} value={`${quote.input_amount} ${quote.input_asset}`} />
          <QuoteLine label={t("预计收到", "Expected output")} value={`${quote.output_amount} ${quote.output_asset}`} strong />
          <QuoteLine label={t("最低收到", "Minimum output")} value={`${quote.min_output_amount} ${quote.output_asset}`} />
          <QuoteLine label={t("手续费", "Fee")} value={`${quote.fee_amount} ${quote.fee_asset}`} />
          <QuoteLine label={t("价格影响", "Price impact")} value={`${(quote.price_impact_bps / 100).toFixed(2)}%`} />
        </div>

        {tradeError ? (
          <p className="mx-5 rounded-xl border border-red-400/25 bg-red-500/10 px-4 py-3 text-sm leading-6 text-red-100" role="alert">
            {tradeError}
          </p>
        ) : null}

        <footer className="flex flex-wrap justify-end gap-2 px-5 py-4">
          <button type="button" className="theme-secondary-btn px-4 py-2 text-sm" disabled={tradeLoading} onClick={onClose}>
            {t("返回修改", "Back to edit")}
          </button>
          <button
            ref={confirmButtonRef}
            type="button"
            className="bancor-sign-trade-btn"
            disabled={tradeLoading}
            onClick={onConfirm}
          >
            <ShieldCheck className="h-4 w-4" />
            {tradeLoading ? t("正在签名并提交...", "Signing and submitting...") : t("确认签名交易", "Confirm signed trade")}
          </button>
        </footer>
      </section>
    </div>
  );
}

function MetricCard({
  label,
  value,
  secondaryValue,
  detail,
}: {
  label: string;
  value: string;
  secondaryValue?: string;
  detail?: string;
}) {
  const valueClassName = "mt-1 break-all text-sm font-semibold text-white sm:text-base";
  return (
    <div className="rounded-xl border border-white/8 bg-white/[0.025] px-3 py-2.5">
      <p className="text-[11px] uppercase tracking-wide text-white/40">{label}</p>
      <p className={valueClassName}>{value}</p>
      {secondaryValue ? <p className={valueClassName}>{secondaryValue}</p> : null}
      {detail ? <p className="mt-0.5 text-[11px] leading-4 text-white/45">{detail}</p> : null}
    </div>
  );
}

function QuoteLine({ label, value, strong = false }: { label: string; value: string; strong?: boolean }) {
  return (
    <div>
      <p className="text-xs text-white/45">{label}</p>
      <p className={`mt-1 ${strong ? "font-semibold text-emerald-200" : "text-white/80"}`}>{value}</p>
    </div>
  );
}

function BalanceLine({
  label,
  value,
  actionLabel,
  disabled,
  onClick,
}: {
  label: string;
  value: string;
  actionLabel: string;
  disabled: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      className="group rounded-xl border border-white/8 bg-white/[0.025] px-4 py-3 text-left transition enabled:hover:border-sky-300/30 enabled:hover:bg-sky-400/[0.07] disabled:cursor-default"
      disabled={disabled}
      onClick={onClick}
      title={actionLabel}
      aria-label={`${actionLabel}: ${value}`}
    >
      <span className="text-xs text-white/45">{label}</span>
      <span className="mt-1 block text-xl font-semibold text-white">{value}</span>
      {!disabled ? <span className="mt-1 block text-[11px] text-sky-200/55 transition group-hover:text-sky-100/80">{actionLabel}</span> : null}
    </button>
  );
}
