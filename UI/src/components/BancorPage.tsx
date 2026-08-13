import {
  ArrowDownUp,
  BarChart3,
  ChevronLeft,
  ChevronRight,
  Maximize2,
  Minimize2,
  Minus,
  Plus,
  RefreshCw,
  ShieldCheck,
  TrendingUp,
  WalletCards,
  X,
} from "lucide-react";
import { useEffect, useId, useRef, useState } from "react";
import type { KeyboardEvent as ReactKeyboardEvent, PointerEvent as ReactPointerEvent } from "react";

import {
  BANCOR_DEFAULT_SLIPPAGE_BPS,
  BANCOR_TRADE_PAGE_SIZE,
  type BancorAmountAdjustment,
  adjustBancorInputAmount,
  calculateBancorEstimatedOutput,
  calculateBancorInputFee,
  formatBancorApiError,
  parseBancorSlippagePercent,
  validateBancorTradeInput,
} from "../hooks/useBancorRuntime";
import type { useBancorRuntime } from "../hooks/useBancorRuntime";
import type { NniBancorCandle, NniBancorQuoteResponse } from "../types/api";
import { NniPublicKeyDisplay } from "./NniPublicKeyDisplay";

type Translate = (zh: string, en: string) => string;
type BancorRuntime = ReturnType<typeof useBancorRuntime>;
type CandleColor = { stroke: string; fill: string; volumeFill: string };
export type BancorCandleVisualState = "up" | "down" | "flat" | "gap";
type BancorWheelTarget = {
  addEventListener: (type: "wheel", listener: EventListener, options?: AddEventListenerOptions) => void;
  removeEventListener: (type: "wheel", listener: EventListener) => void;
};
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

export function resolveBancorCandlePalette(t: Translate): {
  up: CandleColor;
  down: CandleColor;
  flat: CandleColor;
  gap: CandleColor;
} {
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
  const flat = {
    stroke: "var(--theme-chart-neutral)",
    fill: "var(--theme-chart-neutral-fill)",
    volumeFill: "var(--theme-chart-neutral-volume)",
  };
  const gap = {
    stroke: "var(--theme-chart-gap)",
    fill: "var(--theme-chart-surface)",
    volumeFill: "transparent",
  };
  return t("zh", "en") === "zh"
    ? { up: red, down: green, flat, gap }
    : { up: green, down: red, flat, gap };
}

export function resolveBancorCandleVisualState(candle: NniBancorCandle): BancorCandleVisualState {
  if (!candle.has_trades) return "gap";
  const open = Number(candle.open);
  const close = Number(candle.close);
  if (!Number.isFinite(open) || !Number.isFinite(close) || close === open) return "flat";
  return close > open ? "up" : "down";
}

export function resolveBancorTradeColor(side: "buy" | "sell", t: Translate): string {
  const palette = resolveBancorCandlePalette(t);
  return side === "buy" ? palette.up.stroke : palette.down.stroke;
}

export function isBancorCandleOpen(candle: NniBancorCandle, nowUnix = Date.now() / 1_000): boolean {
  return Number.isFinite(nowUnix)
    && candle.bucket_start_unix <= nowUnix
    && nowUnix < candle.bucket_end_unix;
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

export function calculateBancorZoomViewport({
  total,
  visible,
  offsetFromLatest,
  nextVisible,
  anchorRatio,
}: {
  total: number;
  visible: number;
  offsetFromLatest: number;
  nextVisible: number;
  anchorRatio: number;
}): { visible: number; offsetFromLatest: number } {
  const safeTotal = Math.max(0, Math.floor(total));
  if (safeTotal === 0) return { visible: 0, offsetFromLatest: 0 };

  const currentWindow = calculateBancorVisibleWindow(safeTotal, visible, offsetFromLatest);
  const safeNextVisible = Math.max(1, Math.min(safeTotal, Math.floor(nextVisible)));
  const safeAnchorRatio = Number.isFinite(anchorRatio)
    ? Math.max(0, Math.min(1, anchorRatio))
    : 0.5;
  const currentVisible = currentWindow.end - currentWindow.start;
  const anchoredPosition = currentWindow.start + safeAnchorRatio * currentVisible;
  const desiredStart = anchoredPosition - safeAnchorRatio * safeNextVisible;
  const desiredOffset = Math.round(safeTotal - safeNextVisible - desiredStart);
  const nextWindow = calculateBancorVisibleWindow(safeTotal, safeNextVisible, desiredOffset);
  return { visible: safeNextVisible, offsetFromLatest: nextWindow.offset };
}

export function calculateBancorDefaultVisibleCount(total: number, viewportWidth = 900): number {
  const safeTotal = Math.max(0, Math.floor(total));
  if (safeTotal <= BANCOR_MIN_VISIBLE_CANDLES) return safeTotal;
  const geometry = calculateBancorChartGeometry(viewportWidth);
  const responsiveCapacity = Math.max(
    BANCOR_MIN_VISIBLE_CANDLES,
    Math.floor((geometry.plotRight - 18) / 6),
  );
  if (safeTotal <= BANCOR_DEFAULT_VISIBLE_CANDLES + BANCOR_DRAG_HISTORY_HEADROOM) {
    return Math.min(
      responsiveCapacity,
      Math.max(BANCOR_MIN_VISIBLE_CANDLES, safeTotal - BANCOR_DRAG_HISTORY_HEADROOM),
    );
  }
  return Math.min(BANCOR_DEFAULT_VISIBLE_CANDLES, responsiveCapacity);
}

export function calculateBancorPointerCandleIndex({
  pointerX,
  plotLeft,
  plotRight,
  candleCount,
}: {
  pointerX: number;
  plotLeft: number;
  plotRight: number;
  candleCount: number;
}): number | null {
  const safeCount = Math.max(0, Math.floor(candleCount));
  if (safeCount === 0 || !Number.isFinite(pointerX) || pointerX < plotLeft || pointerX > plotRight) {
    return null;
  }
  const step = (plotRight - plotLeft) / safeCount;
  if (!Number.isFinite(step) || step <= 0) return null;
  return Math.max(0, Math.min(safeCount - 1, Math.floor((pointerX - plotLeft) / step)));
}

export function bindBancorWheelZoom(
  target: BancorWheelTarget,
  handler: (event: globalThis.WheelEvent) => void,
): () => void {
  const listener: EventListener = (event) => handler(event as globalThis.WheelEvent);
  target.addEventListener("wheel", listener, { passive: false });
  return () => target.removeEventListener("wheel", listener);
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

export function paginateBancorTrades<T>(items: readonly T[], requestedPage: number): {
  items: T[];
  page: number;
  totalPages: number;
} {
  const totalPages = Math.max(1, Math.ceil(items.length / BANCOR_TRADE_PAGE_SIZE));
  const normalizedPage = Number.isFinite(requestedPage) ? Math.floor(requestedPage) : 1;
  const page = Math.max(1, Math.min(totalPages, normalizedPage));
  const start = (page - 1) * BANCOR_TRADE_PAGE_SIZE;
  return {
    items: items.slice(start, start + BANCOR_TRADE_PAGE_SIZE),
    page,
    totalPages,
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
  const tradePanelRef = useRef<HTMLDivElement>(null);
  const [side, setSide] = useState<"buy" | "sell">("sell");
  const [tradeLayout, setTradeLayout] = useState<"standard" | "swap">("standard");
  const [inputAmount, setInputAmount] = useState("");
  const [slippagePercent, setSlippagePercent] = useState((BANCOR_DEFAULT_SLIPPAGE_BPS / 100).toFixed(2));
  const [marketTradesPage, setMarketTradesPage] = useState(1);
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
  const slippageBps = parseBancorSlippagePercent(slippagePercent);
  const slippageError = slippageBps === null
    ? t("滑点必须在 0% 到 50% 之间，最多保留两位小数。", "Slippage must be between 0% and 50%, with at most two decimal places.")
    : null;
  const estimatedInputFee = market && inputAmount.trim()
    ? calculateBancorInputFee(inputAmount, market.fee_bps)
    : null;
  const inputBalance = side === "buy" ? account?.usd_balance : account?.point_balance;
  const estimatedSwapOutput = market && inputAmount.trim()
    ? calculateBancorEstimatedOutput({ side, inputAmount, market })
    : null;
  const quotedOutput = quote?.side === side && quote.input_amount === inputAmount
    ? quote.output_amount
    : estimatedSwapOutput;
  const paginatedMarketTrades = paginateBancorTrades(marketTrades?.trades ?? [], marketTradesPage);
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
  const changeTradeLayout = (next: "standard" | "swap") => {
    setTradeLayout(next);
    clearQuote();
  };
  const changeSlippage = (value: string) => {
    setSlippagePercent(value);
    clearQuote();
  };
  const confirmTrade = async () => {
    const result = await trade();
    if (result) setInputAmount("");
  };
  const openTradePanel = () => {
    const panel = tradePanelRef.current;
    if (!panel) return;
    panel.scrollIntoView({ behavior: "smooth", block: "start" });
    panel.querySelector<HTMLInputElement>('input[inputmode="decimal"]')?.focus({ preventScroll: true });
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
        <div className="mt-4 grid gap-2 sm:grid-cols-2 xl:grid-cols-3">
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
            <div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1">
              <BarChart3 className="h-5 w-5 text-sky-300" />
              <h2 className="text-lg font-semibold text-white">{t("实际成交均价 K 线", "Average execution-price candlesticks")}</h2>
              <div
                className="flex min-w-0 items-baseline gap-1.5 rounded-md border border-sky-300/15 bg-sky-400/[0.06] px-2 py-1"
                aria-label={t("池内即时边际价", "Live pool marginal price")}
              >
                <span className="shrink-0 text-[11px] text-white/45">{t("池内即时边际价", "Live pool marginal price")}</span>
                <span className="min-w-0 break-all font-mono text-xs font-semibold leading-4 text-sky-200 sm:text-sm">
                  {market ? `${market.marginal_price_usd_per_point} USD` : "—"}
                </span>
              </div>
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
            <>
              <CandleChart
                key={candleIntervalSeconds}
                candles={candles.candles}
                intervalSeconds={candles.interval_seconds}
                priceDecimalPlaces={candles.price_decimal_places}
                livePrice={market?.marginal_price_usd_per_point}
                formatUnixDateTime={formatUnixDateTime}
                onTrade={openTradePanel}
                t={t}
              />
            </>
          ) : (
            <div className="flex min-h-64 flex-col items-center justify-center rounded-xl border border-dashed border-white/10 px-5 text-center">
              <BarChart3 className="h-8 w-8 text-white/35" />
              <p className="mt-3 text-sm text-white/55">{t("暂无成交，首笔成交后显示 K 线。", "No trades yet. Candlesticks will appear after the first trade.")}</p>
            </div>
          )}
        </div>
        </section>

        <div id="bancor-trade-panel" ref={tradePanelRef} className="theme-shadow-card scroll-mt-4 p-4 sm:p-5">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <div className="flex items-center gap-2">
              <ArrowDownUp className="h-5 w-5 text-sky-300" />
              <h2 className="text-lg font-semibold text-white">{t("交易", "Trade")}</h2>
            </div>
            <div className="flex rounded-lg border border-white/8 bg-white/[0.025] p-1" role="group" aria-label={t("交易模式", "Trade mode")}>
              {(["standard", "swap"] as const).map((mode) => (
                <button
                  key={mode}
                  type="button"
                  aria-pressed={tradeLayout === mode}
                  className={`rounded-md px-2.5 py-1 text-xs font-medium transition ${tradeLayout === mode ? "bg-sky-400/15 text-sky-100" : "text-white/45 hover:text-white/75"}`}
                  onClick={() => changeTradeLayout(mode)}
                >
                  {mode === "standard" ? t("标准", "Standard") : "SWAP"}
                </button>
              ))}
            </div>
          </div>
          <div className="mt-3 rounded-xl border border-white/8 bg-white/[0.025] p-3">
            <div className="flex items-center justify-between gap-3">
              <div className="flex items-center gap-2">
                <WalletCards className="h-4 w-4 text-sky-300" />
                <h3 className="text-sm font-medium text-white/80">{t("我的余额", "My balances")}</h3>
              </div>
              <button type="button" className="theme-icon-btn" disabled={accountLoading || !nniReady} onClick={() => void fetchAccount()} title={t("签名刷新余额", "Sign to refresh balances")}>
                <RefreshCw className={`h-4 w-4 ${accountLoading ? "animate-spin" : ""}`} />
              </button>
            </div>
            <div className="mt-2 grid min-w-0 gap-2 sm:grid-cols-2">
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
          </div>

          {tradeLayout === "standard" ? (
            <>
              <div className="mt-3 grid grid-cols-2 rounded-xl bg-white/5 p-1">
                {(["sell", "buy"] as const).map((value) => (
                  <button
                    key={value}
                    type="button"
                    className={`rounded-lg px-4 py-2 text-sm font-medium transition ${side === value ? "bg-sky-400/20 text-sky-100" : "text-white/55 hover:text-white/80"}`}
                    onClick={() => changeSide(value)}
                  >
                    {value === "sell" ? t("卖出 POINT", "Sell POINT") : t("买入 POINT", "Buy POINT")}
                  </button>
                ))}
              </div>

              <div className="mt-3">
                <label htmlFor="bancor-standard-input-amount" className="block text-xs text-white/70">
                  {t("支付数量", "Amount to pay")} ({inputAsset})
                </label>
                <div className="mt-1.5 flex items-center rounded-xl border border-white/10 bg-black/10 px-3 focus-within:border-sky-400/50">
                  <input
                    id="bancor-standard-input-amount"
                    value={inputAmount}
                    inputMode="decimal"
                    placeholder="0.0000"
                    className="min-w-0 flex-1 bg-transparent py-2.5 text-base text-white outline-none placeholder:text-white/25"
                    onChange={(event) => {
                      setInputAmount(event.target.value);
                      clearQuote();
                    }}
                  />
                  <span className="text-sm font-semibold text-white/55">{inputAsset}</span>
                </div>
                {quotedOutput ? (
                  <p
                    className="mt-1.5 break-all text-right text-xs font-medium text-sky-200/80"
                    aria-label={t("预计兑换数量", "Estimated exchange amount")}
                  >
                    ≈ {quotedOutput} {outputAsset}
                  </p>
                ) : null}
                <BancorAmountAdjustmentControls
                  t={t}
                  value={inputAmount}
                  onChange={(value) => {
                    setInputAmount(value);
                    clearQuote();
                  }}
                />
              </div>
            </>
          ) : (
            <BancorSwapTradePanel
              t={t}
              side={side}
              inputAmount={inputAmount}
              inputAsset={inputAsset}
              inputBalance={inputBalance ?? null}
              outputAsset={outputAsset}
              outputAmount={quotedOutput}
              onInputChange={(value) => {
                setInputAmount(value);
                clearQuote();
              }}
              onFillBalance={() => inputBalance && fillBalance(side, inputBalance)}
              onFlip={() => changeSide(side === "sell" ? "buy" : "sell")}
            />
          )}
          <div className="mt-3 rounded-xl border border-white/8 bg-white/[0.025] p-3">
            <div className="flex flex-wrap items-center justify-between gap-2">
              <label htmlFor="bancor-slippage-percent" className="text-sm font-medium text-white/70">
                {t("滑点保护与警戒", "Slippage protection and warning")}
              </label>
              <div className="flex flex-wrap gap-1.5" role="group" aria-label={t("常用滑点", "Common slippage settings")}>
                {["0.50", "1.00", "3.00", "5.00"].map((value) => (
                  <button
                    key={value}
                    type="button"
                    aria-pressed={slippagePercent === value}
                    className={`rounded-lg border px-2 py-1 text-xs transition ${slippagePercent === value ? "border-sky-300/35 bg-sky-400/15 text-sky-100" : "border-white/8 text-white/50 hover:text-white/75"}`}
                    onClick={() => changeSlippage(value)}
                  >
                    {Number(value).toFixed(2)}%
                  </button>
                ))}
              </div>
            </div>
            <div className="mt-2 flex items-center rounded-lg border border-white/10 bg-black/10 px-3 focus-within:border-sky-400/50">
              <input
                id="bancor-slippage-percent"
                value={slippagePercent}
                inputMode="decimal"
                aria-invalid={Boolean(slippageError)}
                className="min-w-0 flex-1 bg-transparent py-2 text-sm text-white outline-none"
                onChange={(event) => changeSlippage(event.target.value)}
              />
              <span className="text-sm text-white/55">%</span>
            </div>
            <p className="mt-1.5 text-xs leading-4 text-white/45">
              {t("用于最低到账保护；报价的价格影响超过此值时会标黄警告，但你仍可确认继续。", "Protects the minimum output. A quote whose price impact exceeds this value is highlighted as a warning, but you may still confirm it.")}
            </p>
            {slippageError ? <p className="mt-1 text-xs text-red-200" role="alert">{slippageError}</p> : null}
          </div>
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
            className="theme-primary-btn mt-3 w-full justify-center"
            disabled={!tradingReady || !inputAmount.trim() || Boolean(inputErrorCode) || slippageBps === null || quoteLoading || tradeLoading}
            onClick={() => slippageBps !== null && void preview(side, inputAmount, slippageBps)}
          >
            {quoteLoading
              ? t("正在计算...", "Calculating...")
              : side === "sell"
                ? t("卖出", "Sell")
                : t("买入", "Buy")}
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
                  <span className="font-medium" style={{ color: resolveBancorTradeColor(record.side, t) }}>
                    {record.side === "buy" ? t("买入 POINT", "Buy POINT") : t("卖出 POINT", "Sell POINT")}
                  </span>
                  <p className="mt-1 text-xs text-white/40">{formatUnixDateTime(record.created_at_unix)}</p>
                </div>
                <span className="text-white/55">{record.input_amount} {record.input_asset}</span>
                <span className="font-medium" style={{ color: resolveBancorTradeColor(record.side, t) }}>+ {record.output_amount} {record.output_asset}</span>
              </div>
            )) : (
              <div className="rounded-xl border border-dashed border-white/10 px-4 py-8 text-center text-sm text-white/40">
                {account ? t("还没有成交记录。", "No trades yet.") : t("点击“交易”卡片内的余额刷新按钮读取账户。", "Use the balance refresh button in the Trade card to load the account.")}
              </div>
            )}
          </div>
          {account && account.total_pages > 1 ? (
            <div
              className="mt-4 flex items-center justify-between gap-3"
              data-bancor-trade-pagination="account"
              data-bancor-page-size={BANCOR_TRADE_PAGE_SIZE}
            >
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
              <p className="mt-1 text-sm text-white/50">{t("仅展示最近 100 笔全市场成交。", "Shows only the latest 100 market-wide trades.")}</p>
            </div>
            <div className="flex items-center gap-2">
              <span className="text-xs text-white/40">{marketTrades?.trades.length ?? 0} {t("笔", "trades")}</span>
              <button
                type="button"
                className="theme-icon-btn"
                aria-label={t("刷新市场成交记录", "Refresh market trades")}
                disabled={marketTradesLoading}
                onClick={() => {
                  setMarketTradesPage(1);
                  void fetchMarketTrades();
                }}
              >
                <RefreshCw className={`h-4 w-4 ${marketTradesLoading ? "animate-spin" : ""}`} />
              </button>
            </div>
          </div>
          {marketTradesError ? <p className="mt-3 text-sm text-red-200" role="alert">{marketTradesError}</p> : null}
          <div className="mt-4 grid gap-2">
            {paginatedMarketTrades.items.length ? paginatedMarketTrades.items.map((record) => (
              <div
                key={record.trade_id}
                className="grid gap-2 rounded-xl border border-white/8 bg-white/[0.025] px-4 py-3 text-sm sm:grid-cols-[minmax(0,1fr)_auto_auto] sm:items-center"
                data-bancor-trade-row="market"
              >
                <div className="min-w-0">
                  <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
                    <span className="font-medium" style={{ color: resolveBancorTradeColor(record.side, t) }}>
                      {record.side === "buy" ? t("买入 POINT", "Buy POINT") : t("卖出 POINT", "Sell POINT")}
                    </span>
                    <NniPublicKeyDisplay
                      value={record.device_pubkey_compact}
                      t={t}
                      shorten={{ head: 16, tail: 12 }}
                      valueClassName="text-[11px] text-white/40"
                    />
                  </div>
                  <p className="mt-1 text-xs text-white/40">{formatUnixDateTime(record.created_at_unix)}</p>
                </div>
                <span className="text-white/55">{record.input_amount} {record.input_asset}</span>
                <span className="font-medium" style={{ color: resolveBancorTradeColor(record.side, t) }}>+ {record.output_amount} {record.output_asset}</span>
              </div>
            )) : (
              <div className="rounded-xl border border-dashed border-white/10 px-4 py-8 text-center text-sm text-white/40">
                {marketTradesLoading ? t("正在读取市场成交记录…", "Loading market trades…") : t("市场暂时还没有成交记录。", "No market trades yet.")}
              </div>
            )}
          </div>
          {paginatedMarketTrades.totalPages > 1 ? (
            <div
              className="mt-4 flex items-center justify-between gap-3"
              data-bancor-trade-pagination="market"
              data-bancor-page-size={BANCOR_TRADE_PAGE_SIZE}
            >
              <button
                type="button"
                className="theme-secondary-btn"
                disabled={marketTradesLoading || paginatedMarketTrades.page <= 1}
                onClick={() => setMarketTradesPage(paginatedMarketTrades.page - 1)}
              >
                {t("上一页", "Previous")}
              </button>
              <span className="text-xs text-white/45">
                {t("第", "Page")} {paginatedMarketTrades.page} / {paginatedMarketTrades.totalPages} {t("页", "")}
              </span>
              <button
                type="button"
                className="theme-secondary-btn"
                disabled={marketTradesLoading || paginatedMarketTrades.page >= paginatedMarketTrades.totalPages}
                onClick={() => setMarketTradesPage(paginatedMarketTrades.page + 1)}
              >
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

export function BancorSwapTradePanel({
  t,
  side,
  inputAmount,
  inputAsset,
  inputBalance,
  outputAsset,
  outputAmount,
  onInputChange,
  onFillBalance,
  onFlip,
}: {
  t: Translate;
  side: "buy" | "sell";
  inputAmount: string;
  inputAsset: "POINT" | "USD";
  inputBalance: string | null;
  outputAsset: "POINT" | "USD";
  outputAmount: string | null;
  onInputChange: (value: string) => void;
  onFillBalance: () => void;
  onFlip: () => void;
}) {
  return (
    <div className="mt-3" data-bancor-trade-layout="swap">
      <div className="rounded-xl border border-white/10 bg-black/10 p-3 focus-within:border-sky-400/45">
        <div className="flex items-center justify-between gap-3 text-xs text-white/45">
          <span>{t("支付", "Pay")}</span>
          <button
            type="button"
            className="min-w-0 max-w-[78%] break-all rounded-md px-2 py-1 text-right text-[11px] leading-4 text-white/50 transition hover:bg-white/5 hover:text-sky-100 disabled:cursor-not-allowed disabled:opacity-45"
            disabled={!inputBalance}
            onClick={onFillBalance}
          >
            {t("余额", "Balance")}：{inputBalance ?? "—"}
          </button>
        </div>
        <label className="mt-1 flex items-center gap-3">
          <span className="sr-only">{t("支付数量", "Amount to pay")}</span>
          <input
            id="bancor-swap-input-amount"
            value={inputAmount}
            inputMode="decimal"
            placeholder="0.0000"
            className="min-w-0 flex-1 bg-transparent py-1.5 text-xl font-medium text-white outline-none placeholder:text-white/20"
            onChange={(event) => onInputChange(event.target.value)}
          />
          <span className="rounded-lg border border-white/10 bg-white/[0.04] px-2.5 py-1.5 text-sm font-semibold text-white/80">{inputAsset}</span>
        </label>
        <BancorAmountAdjustmentControls t={t} value={inputAmount} onChange={onInputChange} />
      </div>

      <div className="relative z-10 -my-3 flex justify-center">
        <button
          type="button"
          className="theme-icon-btn h-9 w-9 border border-sky-300/25 bg-slate-900 text-sky-200 shadow-lg"
          onClick={onFlip}
          aria-label={t(`切换为 ${outputAsset} 支付`, `Pay with ${outputAsset}`)}
          title={t("切换兑换方向", "Reverse swap direction")}
        >
          <ArrowDownUp className="h-4 w-4" />
        </button>
      </div>

      <div className="rounded-xl border border-white/8 bg-white/[0.025] p-3 pt-4">
        <div className="flex items-center justify-between gap-3 text-xs text-white/45">
          <span>{t("预计收到", "Estimated output")}</span>
          <span>{side === "sell" ? t("卖出 POINT", "Sell POINT") : t("买入 POINT", "Buy POINT")}</span>
        </div>
        <div className="mt-1 flex items-center gap-3">
          <span className={`min-w-0 flex-1 py-1.5 text-xl font-medium ${outputAmount ? "text-white" : "text-white/25"}`}>
            {outputAmount ?? "—"}
          </span>
          <span className="rounded-lg border border-white/10 bg-white/[0.04] px-2.5 py-1.5 text-sm font-semibold text-white/80">{outputAsset}</span>
        </div>
        <p className="mt-1 text-[11px] leading-4 text-white/35">
          {t("按当前储备预估，最终到账以服务端签名报价为准。", "Estimated from current reserves; the signed server quote is final.")}
        </p>
      </div>
    </div>
  );
}

export function BancorAmountAdjustmentControls({
  t,
  value,
  onChange,
}: {
  t: Translate;
  value: string;
  onChange: (value: string) => void;
}) {
  const controls: Array<{
    adjustment: BancorAmountAdjustment;
    label: string;
    title: string;
  }> = [
    {
      adjustment: "decrease_25_percent",
      label: "−25%",
      title: t("将当前数量减少 25%", "Reduce the current amount by 25%"),
    },
    {
      adjustment: "decrease_50_percent",
      label: "−50%",
      title: t("将当前数量减少 50%", "Reduce the current amount by 50%"),
    },
    {
      adjustment: "decrement",
      label: "−",
      title: t("减少 1", "Decrease by 1"),
    },
    {
      adjustment: "increment",
      label: "+",
      title: t("增加 1", "Increase by 1"),
    },
  ];
  return (
    <div
      className="mt-1.5 flex flex-wrap justify-end gap-1.5"
      role="group"
      aria-label={t("快速调整支付数量", "Quick amount adjustments")}
    >
      {controls.map((control) => {
        const adjusted = adjustBancorInputAmount(value, control.adjustment);
        const disabled = adjusted === null || adjusted === value;
        return (
          <button
            key={control.adjustment}
            type="button"
            className="min-w-10 rounded-lg border border-white/10 bg-white/[0.03] px-2.5 py-1 text-xs font-medium text-white/60 transition hover:border-sky-300/25 hover:bg-sky-400/10 hover:text-sky-100 disabled:cursor-not-allowed disabled:opacity-35"
            disabled={disabled}
            aria-label={control.title}
            title={control.title}
            onClick={() => adjusted !== null && onChange(adjusted)}
          >
            {control.label}
          </button>
        );
      })}
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
  livePrice,
  formatUnixDateTime,
  onTrade,
  t,
}: {
  candles: NniBancorCandle[];
  intervalSeconds: number;
  priceDecimalPlaces: number;
  livePrice?: string | null;
  formatUnixDateTime: (value?: number | null) => string;
  onTrade?: () => void;
  t: Translate;
}) {
  const chartRef = useRef<HTMLDivElement>(null);
  const wheelHandlerRef = useRef<(event: globalThis.WheelEvent) => void>(() => undefined);
  const dragRef = useRef<{
    moved: boolean;
    pointerId: number;
    startOffset: number;
    startX: number;
    startY: number;
  } | null>(null);
  const previousCandleCountRef = useRef(candles.length);
  const chartInstanceId = useId().replace(/:/g, "");
  const [viewportWidth, setViewportWidth] = useState(900);
  const [visibleCountOverride, setVisibleCountOverride] = useState<number | null>(null);
  const [verticalZoom, setVerticalZoom] = useState(1);
  const [offsetFromLatest, setOffsetFromLatest] = useState(0);
  const [hoveredIndex, setHoveredIndex] = useState<number | null>(null);
  const [isDragging, setIsDragging] = useState(false);
  const [maximized, setMaximized] = useState(false);
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
    const element = chartRef.current;
    if (!element) return;
    return bindBancorWheelZoom(element, (event) => wheelHandlerRef.current(event));
  }, []);

  useEffect(() => {
    const previousCount = previousCandleCountRef.current;
    if (candles.length > previousCount && offsetFromLatest > 0) {
      setOffsetFromLatest((current) => current + candles.length - previousCount);
    }
    previousCandleCountRef.current = candles.length;
  }, [candles.length, offsetFromLatest]);

  useEffect(() => {
    if (!maximized) return;
    const previousBodyOverflow = document.body.style.overflow;
    const restoreFromEscape = (event: globalThis.KeyboardEvent) => {
      if (event.key === "Escape") setMaximized(false);
    };
    document.body.style.overflow = "hidden";
    window.addEventListener("keydown", restoreFromEscape);
    return () => {
      document.body.style.overflow = previousBodyOverflow;
      window.removeEventListener("keydown", restoreFromEscape);
    };
  }, [maximized]);

  const defaultVisibleCount = calculateBancorDefaultVisibleCount(allValues.length, viewportWidth);
  const requestedVisibleCount = visibleCountOverride ?? defaultVisibleCount;
  const visibleCount = Math.max(1, Math.min(allValues.length, requestedVisibleCount));
  const visibleWindow = calculateBancorVisibleWindow(allValues.length, visibleCount, offsetFromLatest);
  const values = allValues.slice(visibleWindow.start, visibleWindow.end);
  const visibleHighIndex = values.reduce(
    (bestIndex, value, index) => value.high > values[bestIndex].high ? index : bestIndex,
    0,
  );
  const visibleLowIndex = values.reduce(
    (bestIndex, value, index) => value.low < values[bestIndex].low ? index : bestIndex,
    0,
  );
  const visibleHigh = values[visibleHighIndex];
  const visibleLow = values[visibleLowIndex];
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
  const parsedLivePrice = Number(livePrice);
  const hasLivePrice = typeof livePrice === "string" && livePrice.trim() !== "" && Number.isFinite(parsedLivePrice) && parsedLivePrice > 0;
  const currentPrice = hasLivePrice ? parsedLivePrice : last.close;
  const currentPriceText = hasLivePrice ? livePrice : last.candle.close;
  const currentPriceY = yForPrice(currentPrice);
  const currentPriceIsVisible = currentPriceY >= priceTop && currentPriceY <= priceBottom;
  const focused = hoveredIndex === null ? last : values[Math.min(hoveredIndex, values.length - 1)] ?? last;
  const tickIndexes = new Set([0, Math.floor((values.length - 1) / 2), values.length - 1]);
  const palette = resolveBancorCandlePalette(t);
  const latestVisualState = resolveBancorCandleVisualState(last.candle);
  const latestColor = palette[latestVisualState];
  const currentPriceColor = currentPrice > last.close
    ? palette.up
    : currentPrice < last.close
      ? palette.down
      : latestColor;
  const visibleHighX = plotLeft + step * (visibleHighIndex + 0.5);
  const visibleLowX = plotLeft + step * (visibleLowIndex + 0.5);
  const visibleHighY = yForPrice(visibleHigh.high);
  const visibleLowY = yForPrice(visibleLow.low);
  const showMinuteCloseLine = intervalSeconds === 60;
  const minuteCloseLinePoints = showMinuteCloseLine
    ? values
      .map((value, index) => `${plotLeft + step * (index + 0.5)},${yForPrice(value.close)}`)
      .join(" ")
    : "";
  const maxRequestedVisibleCount = Math.min(160, allValues.length);
  const priceClipId = `bancor-price-plot-${chartInstanceId}`;
  const volumeClipId = `bancor-volume-plot-${chartInstanceId}`;
  const nowUnix = Date.now() / 1_000;
  const clampOffset = (value: number) => Math.max(0, Math.min(visibleWindow.maxOffset, value));
  const panBy = (candlesToOlder: number) => {
    setOffsetFromLatest((current) => clampOffset(current + candlesToOlder));
    setHoveredIndex(null);
  };
  const zoomBy = (delta: number, anchorRatio = 1) => {
    const current = visibleCountOverride ?? defaultVisibleCount;
    const next = Math.max(Math.min(BANCOR_MIN_VISIBLE_CANDLES, allValues.length), Math.min(maxRequestedVisibleCount, current + delta));
    const viewport = calculateBancorZoomViewport({
      total: allValues.length,
      visible: current,
      offsetFromLatest: visibleWindow.offset,
      nextVisible: next,
      anchorRatio,
    });
    setVisibleCountOverride(viewport.visible);
    setOffsetFromLatest(viewport.offsetFromLatest);
    setHoveredIndex(null);
  };
  const verticalZoomBy = (factor: number) => {
    setVerticalZoom((current) => Math.max(0.5, Math.min(64, current * factor)));
    setHoveredIndex(null);
  };
  const updateHoveredCandle = (event: ReactPointerEvent<HTMLDivElement>) => {
    const bounds = event.currentTarget.getBoundingClientRect();
    const svgX = ((event.clientX - bounds.left) / Math.max(bounds.width, 1)) * width;
    const index = calculateBancorPointerCandleIndex({
      pointerX: svgX,
      plotLeft,
      plotRight,
      candleCount: values.length,
    });
    setHoveredIndex(index);
  };
  const handlePointerDown = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    if (event.pointerType === "mouse") event.preventDefault();
    event.currentTarget.setPointerCapture(event.pointerId);
    dragRef.current = {
      moved: false,
      pointerId: event.pointerId,
      startOffset: visibleWindow.offset,
      startX: event.clientX,
      startY: event.clientY,
    };
    setIsDragging(true);
  };
  const handlePointerMove = (event: ReactPointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current;
    if (!drag) {
      updateHoveredCandle(event);
      return;
    }
    const deltaX = event.clientX - drag.startX;
    const deltaY = event.clientY - drag.startY;
    if (!drag.moved && Math.hypot(deltaX, deltaY) < 6) return;
    drag.moved = true;
    if (Math.abs(deltaY) > Math.abs(deltaX)) return;
    const pixelsPerCandle = Math.max((viewportWidth * 0.6) / Math.max(visibleCount, 1), 10);
    const candlesToOlder = Math.round(deltaX / pixelsPerCandle);
    setOffsetFromLatest(clampOffset(drag.startOffset + candlesToOlder));
    setHoveredIndex(null);
  };
  const finishPointerDrag = (event: ReactPointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current;
    if (drag?.pointerId === event.pointerId) {
      dragRef.current = null;
      setIsDragging(false);
      if (!drag.moved) updateHoveredCandle(event);
      if (event.currentTarget.hasPointerCapture(event.pointerId)) {
        event.currentTarget.releasePointerCapture(event.pointerId);
      }
    }
  };
  const handleWheel = (event: globalThis.WheelEvent) => {
    if (event.altKey) {
      event.preventDefault();
      verticalZoomBy(event.deltaY > 0 ? 1 / 1.35 : 1.35);
      return;
    }
    if (Math.abs(event.deltaX) > Math.abs(event.deltaY) || event.shiftKey) {
      event.preventDefault();
      const horizontalDelta = Math.abs(event.deltaX) > 0 ? event.deltaX : event.deltaY;
      panBy(horizontalDelta > 0 ? -2 : 2);
      return;
    }
    event.preventDefault();
    const bounds = chartRef.current?.getBoundingClientRect();
    if (!bounds) return;
    const svgX = ((event.clientX - bounds.left) / Math.max(bounds.width, 1)) * width;
    const anchorRatio = Math.max(0, Math.min(1, (svgX - plotLeft) / Math.max(plotRight - plotLeft, 1)));
    zoomBy(event.deltaY > 0 ? 4 : -4, anchorRatio);
  };
  wheelHandlerRef.current = handleWheel;
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
  const openTradePanel = () => {
    setMaximized(false);
    if (onTrade) window.requestAnimationFrame(onTrade);
  };
  const hoveredX = hoveredIndex === null ? null : plotLeft + step * (hoveredIndex + 0.5);
  const hoveredY = hoveredIndex === null ? null : yForPrice(focused.close);
  const hoveredPriceIsVisible = hoveredY !== null && hoveredY >= priceTop && hoveredY <= priceBottom;

  return (
    <div
      id="bancor-candle-chart"
      className={maximized ? "bancor-chart-maximized" : undefined}
    >
      <div className="mb-2 flex flex-wrap items-center justify-between gap-2 text-xs text-white/45">
        <div className="min-w-0">
          <span className="text-white/65">{formatUnixDateTime(focused.candle.bucket_start_unix)}</span>
          <span className="ml-3">O {focused.candle.open} · H {focused.candle.high} · L {focused.candle.low} · C {focused.candle.close}</span>
          <span className="ml-3">VOL {focused.candle.point_volume} POINT</span>
          <span className="ml-3">
            {focused.candle.trade_count > 0
              ? `${focused.candle.trade_count} ${t("笔成交", "trades")}`
              : t("本周期无成交", "No trades in this interval")}
          </span>
        </div>
        <div className="flex flex-wrap items-center justify-end gap-2">
          <span>
            {visibleWindow.maxOffset > 0
              ? t("左右拖动查看历史；点按查看详情；滚轮缩放", "Drag for history; tap for details; wheel to zoom")
              : t("全部实际成交均价 K 线已显示，暂无更多历史", "All average execution-price candles are visible; no older history is available")}
          </span>
        </div>
      </div>
      <div
        ref={chartRef}
        className={`theme-chart-surface relative overflow-hidden rounded-xl border outline-none transition ${isDragging ? "cursor-grabbing ring-1 ring-sky-400/25" : "cursor-grab focus:ring-1 focus:ring-sky-400/35"}`}
        style={{ touchAction: "pan-y" }}
        data-bancor-tap-details="enabled"
        role="group"
        tabIndex={0}
        aria-label={t("可拖动、缩放并点按查看的 POINT 对 USD 实际成交均价 K 线图", "Draggable, zoomable average execution-price POINT to USD candlestick chart with tap details")}
        onKeyDown={handleKeyDown}
        onPointerDown={handlePointerDown}
        onPointerMove={handlePointerMove}
        onPointerUp={finishPointerDrag}
        onPointerCancel={finishPointerDrag}
        onPointerLeave={(event) => {
          if (!dragRef.current && event.pointerType === "mouse") setHoveredIndex(null);
        }}
      >
        <svg
          viewBox={`0 0 ${width} ${height}`}
          className="block h-auto min-h-72 w-full select-none"
          role="img"
          aria-label={t("POINT 对 USD 的实际成交均价 K 线图", "Average execution-price POINT to USD candlestick chart")}
        >
          <defs>
            <clipPath id={priceClipId}>
              <rect x={plotLeft} y={priceTop} width={plotRight - plotLeft} height={priceBottom - priceTop} />
            </clipPath>
            <clipPath id={volumeClipId}>
              <rect x={plotLeft} y={volumeTop} width={plotRight - plotLeft} height={volumeBottom - volumeTop} />
            </clipPath>
          </defs>
          {[0, 0.25, 0.5, 0.75, 1].map((ratio) => {
            const y = priceTop + ratio * (priceBottom - priceTop);
            const label = (priceHigh - ratio * priceSpan).toFixed(priceDecimalPlaces);
            return (
              <g key={ratio}>
                <line x1={plotLeft} y1={y} x2={plotRight} y2={y} stroke="var(--theme-chart-grid)" strokeDasharray="3 6" />
                <text x={priceAxisX} y={y + 4} fill="var(--theme-chart-label)" fontSize="11">{label}</text>
              </g>
            );
          })}
          {[0.25, 0.5, 0.75].map((ratio) => {
            const x = plotLeft + ratio * (plotRight - plotLeft);
            return <line key={ratio} x1={x} y1={priceTop} x2={x} y2={volumeBottom} stroke="var(--theme-chart-grid-soft)" strokeDasharray="3 7" />;
          })}
          <line x1={plotLeft} y1={volumeTop - 10} x2={plotRight} y2={volumeTop - 10} stroke="var(--theme-chart-grid)" />
          {currentPriceIsVisible ? (
            <line
              data-bancor-chart-layer="live-price-line"
              clipPath={`url(#${priceClipId})`}
              x1={plotLeft}
              y1={currentPriceY}
              x2={plotRight}
              y2={currentPriceY}
              stroke={currentPriceColor.stroke}
              strokeOpacity="0.65"
              strokeDasharray="5 5"
            />
          ) : null}
          {showMinuteCloseLine && values.length > 1 ? (
            <polyline
              data-bancor-chart-layer="one-minute-close-line"
              clipPath={`url(#${priceClipId})`}
              points={minuteCloseLinePoints}
              fill="none"
              stroke="var(--theme-chart-close-line)"
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
            const visualState = resolveBancorCandleVisualState(value.candle);
            const color = palette[visualState];
            const hasTrades = visualState !== "gap";
            const candleOpen = isBancorCandleOpen(value.candle, nowUnix);
            const openY = yForPrice(value.open);
            const closeY = yForPrice(value.close);
            const bodyTop = Math.min(openY, closeY);
            const bodyHeight = Math.max(Math.abs(openY - closeY), 2);
            const bodyBottom = bodyTop + bodyHeight;
            const highY = yForPrice(value.high);
            const lowY = yForPrice(value.low);
            const volumeHeight = (value.pointVolume / maxVolume) * (volumeBottom - volumeTop);
            return (
              <g
                key={`${value.candle.bucket_start_unix}-${index}`}
                data-bancor-candle-direction={visualState}
                data-bancor-candle-state={candleOpen ? "open" : "closed"}
              >
                <title>{`${formatUnixDateTime(value.candle.bucket_start_unix)} · O ${value.candle.open} · H ${value.candle.high} · L ${value.candle.low} · C ${value.candle.close} · ${value.candle.point_volume} POINT · ${value.candle.trade_count} ${t("笔", "trades")}`}</title>
                <g clipPath={`url(#${priceClipId})`}>
                  {hasTrades && highY < bodyTop ? <line x1={x} y1={highY} x2={x} y2={bodyTop} stroke={color.stroke} strokeWidth="1.5" /> : null}
                  {hasTrades && bodyBottom < lowY ? <line x1={x} y1={bodyBottom} x2={x} y2={lowY} stroke={color.stroke} strokeWidth="1.5" /> : null}
                  {hasTrades ? (
                    <rect
                      data-bancor-candle-body="true"
                      x={x - bodyWidth / 2}
                      y={bodyTop}
                      width={bodyWidth}
                      height={bodyHeight}
                      rx="1"
                      fill={color.stroke}
                      stroke={color.stroke}
                      strokeWidth="1.2"
                    />
                  ) : (
                    <circle
                      data-bancor-candle-gap="true"
                      cx={x}
                      cy={closeY}
                      r={Math.max(2.5, Math.min(4, bodyWidth / 2))}
                      fill={color.fill}
                      stroke={color.stroke}
                      strokeWidth="1.4"
                    />
                  )}
                  {candleOpen ? (
                    <>
                      <line
                        data-bancor-current-candle-marker="true"
                        x1={x}
                        y1={priceTop}
                        x2={x}
                        y2={priceBottom}
                        stroke="var(--theme-chart-open)"
                        strokeWidth="1"
                        strokeDasharray="2 5"
                        strokeOpacity="0.72"
                      />
                      <circle cx={x} cy={priceTop + 5} r="3" fill="var(--theme-chart-open)" />
                    </>
                  ) : null}
                </g>
                {hasTrades && volumeHeight > 0 ? (
                  <rect
                    data-bancor-volume-direction={visualState}
                    clipPath={`url(#${volumeClipId})`}
                    x={x - bodyWidth / 2}
                    y={volumeBottom - volumeHeight}
                    width={bodyWidth}
                    height={volumeHeight}
                    rx="1"
                    fill={color.volumeFill}
                  />
                ) : null}
                {tickIndexes.has(index) ? (
                  <text x={x} y={timeAxisY} textAnchor="middle" fill="var(--theme-chart-label)" fontSize="10">
                    {formatUnixDateTime(value.candle.bucket_start_unix)}
                  </text>
                ) : null}
              </g>
            );
          })}
          <g data-bancor-chart-layer="visible-price-extremes" clipPath={`url(#${priceClipId})`} pointerEvents="none">
            <line x1={visibleHighX - 5} y1={visibleHighY} x2={visibleHighX + 5} y2={visibleHighY} stroke="var(--theme-chart-label-strong)" />
            <text
              x={visibleHighX <= (plotLeft + plotRight) / 2 ? visibleHighX + 7 : visibleHighX - 7}
              y={Math.max(priceTop + 11, visibleHighY - 7)}
              textAnchor={visibleHighX <= (plotLeft + plotRight) / 2 ? "start" : "end"}
              fill="var(--theme-chart-label-strong)"
              fontSize="10"
            >
              H {visibleHigh.candle.high}
            </text>
            <line x1={visibleLowX - 5} y1={visibleLowY} x2={visibleLowX + 5} y2={visibleLowY} stroke="var(--theme-chart-label-strong)" />
            <text
              x={visibleLowX <= (plotLeft + plotRight) / 2 ? visibleLowX + 7 : visibleLowX - 7}
              y={Math.min(priceBottom - 2, visibleLowY + 14)}
              textAnchor={visibleLowX <= (plotLeft + plotRight) / 2 ? "start" : "end"}
              fill="var(--theme-chart-label-strong)"
              fontSize="10"
            >
              L {visibleLow.candle.low}
            </text>
          </g>
          {hoveredX !== null && hoveredY !== null ? (
            <g pointerEvents="none">
              <line x1={hoveredX} y1={priceTop} x2={hoveredX} y2={volumeBottom} stroke="var(--theme-chart-crosshair)" strokeDasharray="4 5" />
              {hoveredPriceIsVisible ? (
                <>
                  <line x1={plotLeft} y1={hoveredY} x2={plotRight} y2={hoveredY} stroke="var(--theme-chart-crosshair)" strokeDasharray="4 5" />
                  <rect x={priceAxisX - 4} y={hoveredY - 10} width="96" height="20" rx="3" fill="var(--theme-chart-tooltip-bg)" />
                  <text x={priceAxisX + 3} y={hoveredY + 4} fill="var(--theme-chart-tooltip-text)" fontSize="11">{focused.candle.close}</text>
                </>
              ) : null}
            </g>
          ) : null}
          {currentPriceIsVisible ? (
            <g data-bancor-chart-layer="live-price-label" pointerEvents="none">
              <title>{t("池内即时边际价参考线", "Live pool marginal-price reference")}</title>
              <rect x={priceAxisX - 4} y={currentPriceY - 10} width="96" height="20" rx="3" fill={currentPriceColor.fill} stroke={currentPriceColor.stroke} strokeWidth="1" />
              <text x={priceAxisX + 3} y={currentPriceY + 4} fill={currentPriceColor.stroke} fontSize="11">{currentPriceText}</text>
            </g>
          ) : null}
          <text x={plotLeft} y={volumeTop + 5} fill="var(--theme-chart-label)" fontSize="10">VOL · POINT</text>
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
          <button
            type="button"
            className="theme-icon-btn h-8 w-8"
            onClick={() => setMaximized((current) => !current)}
            title={maximized ? t("恢复 K 线区域", "Restore candlestick chart") : t("最大化 K 线区域", "Maximize candlestick chart")}
            aria-label={maximized ? t("恢复 K 线区域", "Restore candlestick chart") : t("最大化 K 线区域", "Maximize candlestick chart")}
            aria-pressed={maximized}
            aria-controls="bancor-candle-chart"
          >
            {maximized ? <Minimize2 className="h-4 w-4" /> : <Maximize2 className="h-4 w-4" />}
          </button>
        </div>
      </div>
      <div className="bancor-maximized-trade-action">
        <button
          type="button"
          className="theme-primary-btn min-h-11 min-w-40 justify-center px-6"
          disabled={!onTrade}
          onClick={openTradePanel}
          aria-controls="bancor-trade-panel"
          aria-label={t("打开交易面板", "Open trade panel")}
        >
          <ArrowDownUp className="h-4 w-4" />
          {t("交易", "Trade")}
        </button>
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
  const priceImpactWarning = quote.price_impact_bps > quote.slippage_bps;

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
          <QuoteLine label={t("滑点保护", "Slippage protection")} value={`${(quote.slippage_bps / 100).toFixed(2)}%`} />
        </div>

        {priceImpactWarning ? (
          <div className="mx-5 mb-4 rounded-xl border border-amber-300/35 bg-amber-400/10 px-4 py-3 text-sm leading-6 text-amber-50" role="alert">
            {t(
              `价格影响 ${(quote.price_impact_bps / 100).toFixed(2)}% 已超过你设置的 ${(quote.slippage_bps / 100).toFixed(2)}% 滑点警戒值。这笔交易会明显改变池内价格；确认后仍可继续。`,
              `The ${(quote.price_impact_bps / 100).toFixed(2)}% price impact exceeds your ${(quote.slippage_bps / 100).toFixed(2)}% slippage warning threshold. This trade will materially move the pool price; you can still continue after confirming.`,
            )}
          </div>
        ) : null}

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
            {tradeLoading
              ? t("正在签名并提交...", "Signing and submitting...")
              : priceImpactWarning
                ? t("我已了解风险，继续签名", "I understand the risk; sign and continue")
                : t("确认签名交易", "Confirm signed trade")}
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
  const valueSizeClass = balanceValueSizeClass(value);
  return (
    <button
      type="button"
      className="group min-w-0 max-w-full overflow-hidden rounded-lg border border-white/8 bg-white/[0.025] px-3 py-2 text-left transition enabled:hover:border-sky-300/30 enabled:hover:bg-sky-400/[0.07] disabled:cursor-default"
      disabled={disabled}
      onClick={onClick}
      title={actionLabel}
      aria-label={`${actionLabel}: ${value}`}
    >
      <span className="text-xs text-white/45">{label}</span>
      <span
        className={`mt-1 block max-w-full break-all font-mono font-semibold leading-5 text-white ${valueSizeClass}`}
        title={value}
      >
        {value}
      </span>
      {!disabled ? <span className="mt-1 block text-[11px] leading-4 text-sky-200/55 transition group-hover:text-sky-100/80">{actionLabel}</span> : null}
    </button>
  );
}

export function balanceValueSizeClass(value: string): string {
  const length = Array.from(value.trim()).length;
  if (length >= 28) return "text-xs sm:text-sm";
  if (length >= 18) return "text-sm sm:text-base";
  return "text-base sm:text-lg";
}
