import { useState } from "react";

import type {
  ApiResponse,
  NniBancorAccountResponse,
  NniBancorCandlesResponse,
  NniBancorMarketResponse,
  NniBancorQuoteResponse,
  NniBancorTradeResponse,
} from "../types/api";

type Translate = (zh: string, en: string) => string;
type ApiFetch = (path: string, init?: RequestInit) => Promise<Response>;
const BANCOR_ASSET_SCALE = 10_000n;
const BANCOR_MAX_UNITS = 9_223_372_036_854_775_807n;
export const BANCOR_DEFAULT_CANDLE_INTERVAL_SECONDS = 300;

function parseBancorInputUnits(value: string): bigint | null {
  const match = /^(0|[1-9][0-9]*)(?:\.([0-9]{1,4}))?$/.exec(value.trim());
  if (!match) return null;
  const fraction = (match[2] || "").padEnd(4, "0");
  const units = BigInt(match[1]) * BANCOR_ASSET_SCALE + BigInt(fraction || "0");
  return units > 0n && units <= BANCOR_MAX_UNITS ? units : null;
}

function formatBancorUnits(units: bigint): string {
  return `${units / BANCOR_ASSET_SCALE}.${String(units % BANCOR_ASSET_SCALE).padStart(4, "0")}`;
}

export function calculateBancorInputFee(inputAmount: string, feeBps: number): string | null {
  const inputUnits = parseBancorInputUnits(inputAmount);
  if (inputUnits === null || !Number.isSafeInteger(feeBps) || feeBps < 0 || feeBps >= 10_000) {
    return null;
  }
  const feeUnits = feeBps === 0
    ? 0n
    : (inputUnits * BigInt(feeBps) + 9_999n) / 10_000n;
  return formatBancorUnits(feeUnits);
}

export function validateBancorTradeInput({
  side,
  inputAmount,
  market,
  account,
}: {
  side: "buy" | "sell";
  inputAmount: string;
  market: NniBancorMarketResponse | null;
  account: NniBancorAccountResponse | null;
}): string | null {
  const inputUnits = parseBancorInputUnits(inputAmount);
  if (inputUnits === null) return "nni_bancor_amount_invalid";
  if (market) {
    const feeUnits = market.fee_bps === 0
      ? 0n
      : (inputUnits * BigInt(market.fee_bps) + 9_999n) / 10_000n;
    const curveInputUnits = inputUnits - feeUnits;
    if (curveInputUnits <= 0n) return "nni_bancor_input_after_fee_too_small";
    const inputReserveUnits = BigInt(side === "buy" ? market.usd_reserve_units : market.point_reserve_units);
    const outputReserveUnits = BigInt(side === "buy" ? market.point_reserve_units : market.usd_reserve_units);
    const outputUnits = (curveInputUnits * outputReserveUnits) / (inputReserveUnits + curveInputUnits);
    if (outputUnits <= 0n) return "nni_bancor_output_too_small";
  }
  if (!account) return "nni_bancor_account_required";
  const availableUnits = BigInt(side === "buy" ? account.usd_balance_units : account.point_balance_units);
  if (inputUnits > availableUnits) {
    return side === "buy"
      ? "nni_bancor_insufficient_usd_balance"
      : "nni_bancor_insufficient_point_balance";
  }
  return null;
}

export function formatBancorApiError(
  code: string | null | undefined,
  t: Translate,
  fallback: string,
) {
  if (code === "nni_bancor_amount_invalid" || code === "nni_bancor_input_amount_invalid") {
    return t(
      "交易金额必须大于 0、最多保留 4 位小数，并且不能超过系统可安全保存的范围。",
      "The trade amount must be greater than zero, use at most four decimal places, and stay within the safely stored range.",
    );
  }
  if (
    code === "nni_bancor_trade_below_minimum" ||
    code === "nni_bancor_input_after_fee_too_small" ||
    code === "nni_bancor_output_too_small"
  ) {
    return t(
      "交易金额太小：扣除手续费后或预计到账金额不能为 0.0000，请增加交易金额。",
      "The trade amount is too small: the amount after fees and the expected output must not be 0.0000. Increase the trade amount.",
    );
  }
  if (code === "nni_bancor_trade_above_maximum") {
    return t(
      "交易金额超过单笔安全上限，请减少交易金额。",
      "The trade amount exceeds the per-trade safety limit. Reduce the amount.",
    );
  }
  if (code === "nni_bancor_account_required") {
    return t(
      "请先刷新“我的余额”，读取 POINT 和 USD 可用余额后再交易。",
      "Refresh My balances first so the available POINT and USD balances can be checked before trading.",
    );
  }
  if (code === "nni_bancor_market_not_open") {
    return t("交易市场尚未开启。现在可以查看储备，但不能报价或成交。", "The market is not open yet. Reserves remain visible, but quotes and trades are unavailable.");
  }
  if (code === "nni_bancor_insufficient_point_balance") {
    return t("POINT 余额不足，请减少卖出数量。", "Your POINT balance is too low. Reduce the sell amount.");
  }
  if (code === "nni_bancor_insufficient_usd_balance") {
    return t("USD 记账余额不足，请减少买入金额。", "Your USD account balance is too low. Reduce the buy amount.");
  }
  if (code === "nni_bancor_quote_stale") {
    return t("市场储备已经变化，请重新获取报价。", "Market reserves changed. Please request a new quote.");
  }
  if (code === "nni_bancor_auto_pause_threshold_reached" || code === "nni_bancor_usd_reserve_floor_reached") {
    return t("USD 储备接近安全下限，市场已自动暂停。请等待管理员检查储备。", "The USD reserve is near its safety limit, so the market paused automatically. Wait for an administrator to review the reserves.");
  }
  if (code === "nni_bancor_daily_trade_limit_exceeded") {
    return t("当前设备今天的交易额度已用完，请明天再试。", "This device has reached today's trade limit. Try again tomorrow.");
  }
  if (code === "nni_bancor_trade_rate_limited" || code === "nni_bancor_ip_rate_limited") {
    return t("请求太频繁，请稍等片刻再试。", "Requests are too frequent. Wait a moment and try again.");
  }
  if (code === "nni_bancor_trade_outcome_unknown") {
    return t(
      "提交成交后网络中断，结果暂时无法确认。请先刷新余额和成交记录，不要立即重复交易。",
      "The connection ended after submission, so the outcome is not yet known. Refresh balances and trade history before trying again.",
    );
  }
  if (code === "nni_bancor_price_impact_exceeded") {
    return t("这笔数量对价格影响过大，请减少交易数量。", "This amount would move the price too much. Reduce the trade size.");
  }
  return code || fallback;
}

export function useBancorRuntime({ apiFetch, t }: { apiFetch: ApiFetch; t: Translate }) {
  const [market, setMarket] = useState<NniBancorMarketResponse | null>(null);
  const [candles, setCandles] = useState<NniBancorCandlesResponse | null>(null);
  const [account, setAccount] = useState<NniBancorAccountResponse | null>(null);
  const [quote, setQuote] = useState<NniBancorQuoteResponse | null>(null);
  const [lastTrade, setLastTrade] = useState<NniBancorTradeResponse | null>(null);
  const [marketLoading, setMarketLoading] = useState(false);
  const [candlesLoading, setCandlesLoading] = useState(false);
  const [candlesError, setCandlesError] = useState<string | null>(null);
  const [candleIntervalSeconds, setCandleIntervalSeconds] = useState(BANCOR_DEFAULT_CANDLE_INTERVAL_SECONDS);
  const [accountLoading, setAccountLoading] = useState(false);
  const [quoteLoading, setQuoteLoading] = useState(false);
  const [tradeLoading, setTradeLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);

  const readError = (body: ApiResponse<unknown>, fallback: string) => {
    const data = body.data as { attempts?: Array<{ error_code?: string | null }> } | undefined;
    const code = data?.attempts?.find((attempt) => attempt.error_code)?.error_code || body.error;
    return formatBancorApiError(code, t, fallback);
  };

  const fetchMarket = async (silent = false) => {
    if (!silent) setMarketLoading(true);
    try {
      const response = await apiFetch("/v1/nni/bancor/market");
      const body = (await response.json()) as ApiResponse<NniBancorMarketResponse>;
      if (!response.ok || !body.ok || !body.data) throw new Error(readError(body, `Market load failed (${response.status})`));
      setMarket(body.data);
      setError(null);
      return body.data;
    } catch (cause) {
      if (!silent) setError(cause instanceof Error ? cause.message : t("市场读取失败。", "Market load failed."));
      return null;
    } finally {
      if (!silent) setMarketLoading(false);
    }
  };

  const fetchAccount = async (page = account?.page ?? 1, silent = false) => {
    if (!silent) setAccountLoading(true);
    try {
      const params = new URLSearchParams({ page: String(Math.max(1, page)), per_page: "20" });
      const response = await apiFetch(`/v1/nni/bancor/account?${params}`);
      const body = (await response.json()) as ApiResponse<NniBancorAccountResponse>;
      if (!response.ok || !body.ok || !body.data) throw new Error(readError(body, `Account load failed (${response.status})`));
      setAccount(body.data);
      setError(null);
      return body.data;
    } catch (cause) {
      if (!silent) setError(cause instanceof Error ? cause.message : t("余额读取失败。", "Balance load failed."));
      return null;
    } finally {
      if (!silent) setAccountLoading(false);
    }
  };

  const fetchCandles = async (intervalSeconds = candleIntervalSeconds, silent = false) => {
    if (!silent) setCandlesLoading(true);
    try {
      const params = new URLSearchParams({
        interval_seconds: String(intervalSeconds),
        // Load the full server-supported window so sparse real-trade candles
        // still leave enough history for the chart's horizontal pan.
        limit: "300",
      });
      const response = await apiFetch(`/v1/nni/bancor/candles?${params}`);
      const body = (await response.json()) as ApiResponse<NniBancorCandlesResponse>;
      if (!response.ok || !body.ok || !body.data) {
        throw new Error(readError(body, `Candle load failed (${response.status})`));
      }
      setCandles(body.data);
      setCandlesError(null);
      return body.data;
    } catch (cause) {
      if (!silent) {
        setCandlesError(cause instanceof Error ? cause.message : t("K 线读取失败。", "Candlestick data could not be loaded."));
      }
      return null;
    } finally {
      if (!silent) setCandlesLoading(false);
    }
  };

  const changeCandleInterval = async (intervalSeconds: number) => {
    setCandleIntervalSeconds(intervalSeconds);
    return fetchCandles(intervalSeconds);
  };

  const preview = async (side: "buy" | "sell", inputAmount: string) => {
    setError(null);
    setMessage(null);
    setLastTrade(null);
    const validationError = validateBancorTradeInput({ side, inputAmount, market, account });
    if (validationError) {
      setQuote(null);
      setError(formatBancorApiError(validationError, t, t("金额无法交易。", "This amount cannot be traded.")));
      return null;
    }
    setQuoteLoading(true);
    try {
      const response = await apiFetch("/v1/nni/bancor/quote", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ side, input_amount: inputAmount, slippage_bps: 50 }),
      });
      const body = (await response.json()) as ApiResponse<NniBancorQuoteResponse>;
      if (!response.ok || !body.ok || !body.data) throw new Error(readError(body, `Quote failed (${response.status})`));
      setQuote(body.data);
      return body.data;
    } catch (cause) {
      setQuote(null);
      setError(cause instanceof Error ? cause.message : t("报价失败。", "Quote failed."));
      return null;
    } finally {
      setQuoteLoading(false);
    }
  };

  const trade = async () => {
    if (!quote) return null;
    setTradeLoading(true);
    setError(null);
    setMessage(null);
    try {
      const response = await apiFetch("/v1/nni/bancor/trade", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          side: quote.side,
          input_amount: quote.input_amount,
          min_output: quote.min_output_amount,
          slippage_bps: quote.slippage_bps,
        }),
      });
      const body = (await response.json()) as ApiResponse<NniBancorTradeResponse>;
      if (!response.ok || !body.ok || !body.data) throw new Error(readError(body, `Trade failed (${response.status})`));
      setLastTrade(body.data);
      setQuote(null);
      setMessage(t("交易已完成，余额和市场储备已经更新。", "Trade completed. Balances and market reserves are updated."));
      await Promise.all([
        fetchMarket(true),
        fetchAccount(1, true),
        fetchCandles(candleIntervalSeconds, true),
      ]);
      return body.data;
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : t("交易没有完成。", "The trade was not completed."));
      return null;
    } finally {
      setTradeLoading(false);
    }
  };

  const clearQuote = () => {
    setQuote(null);
    setMessage(null);
  };

  return {
    market,
    candles,
    account,
    quote,
    lastTrade,
    marketLoading,
    candlesLoading,
    candlesError,
    candleIntervalSeconds,
    accountLoading,
    quoteLoading,
    tradeLoading,
    error,
    message,
    fetchMarket,
    fetchCandles,
    changeCandleInterval,
    fetchAccount,
    preview,
    trade,
    clearQuote,
  };
}
