import { useRef, useState } from "react";

import type {
  ApiResponse,
  NniBancorAccountResponse,
  NniBancorCandlesResponse,
  NniBancorMarketResponse,
  NniBancorMarketTradesResponse,
  NniBancorQuoteResponse,
  NniBancorTradeResponse,
} from "../types/api";
import {
  BANCOR_CANDLE_REQUEST_MAX_CANDLES,
  calculateBancorCandleRefreshLimit,
  isBancorCandleResponse,
  mergeBancorCandleResponses,
  readBancorCandleCache,
  writeBancorCandleCache,
} from "../lib/bancor-candle-cache";

type Translate = (zh: string, en: string) => string;
type ApiFetch = (path: string, init?: RequestInit) => Promise<Response>;
const BANCOR_ASSET_SCALE = 10_000n;
const BANCOR_MAX_UNITS = 9_223_372_036_854_775_807n;
export const BANCOR_DEFAULT_CANDLE_INTERVAL_SECONDS = 300;
export const BANCOR_DEFAULT_SLIPPAGE_BPS = 50;
export const BANCOR_MAX_SLIPPAGE_BPS = 5_000;
export const BANCOR_MARKET_TRADE_LIMIT = 100;

export function projectBancorCandlesForInterval(
  current: NniBancorCandlesResponse | null,
  intervalSeconds: number,
  cached: NniBancorCandlesResponse | null,
): NniBancorCandlesResponse | null {
  if (cached?.interval_seconds === intervalSeconds) return cached;
  return current?.interval_seconds === intervalSeconds ? current : null;
}

export function buildBancorCandlesPath(
  intervalSeconds: number,
  limit: number,
  endTimeUnix?: number,
): string {
  const params = new URLSearchParams({
    interval_seconds: String(intervalSeconds),
    limit: String(Math.min(BANCOR_CANDLE_REQUEST_MAX_CANDLES, Math.max(1, Math.floor(limit)))),
  });
  if (endTimeUnix !== undefined) params.set("end_time_unix", String(Math.max(0, Math.floor(endTimeUnix))));
  return `/v1/nni/bancor/candles?${params}`;
}

export function hasEarlierBancorCandles(response: NniBancorCandlesResponse): boolean {
  const oldestBucketStart = response.candles[0]?.bucket_start_unix;
  if (oldestBucketStart == null) return false;
  return oldestBucketStart > response.market_created_at_unix;
}

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

export type BancorAmountAdjustment = "decrease_25_percent" | "decrease_50_percent" | "decrement" | "increment";

export function adjustBancorInputAmount(
  value: string,
  adjustment: BancorAmountAdjustment,
): string | null {
  const normalized = value.trim();
  if (normalized === "" && adjustment === "increment") return formatBancorUnits(1n);
  const match = /^(0|[1-9][0-9]*)(?:\.([0-9]{1,4}))?$/.exec(normalized);
  if (!match) return null;
  const fraction = (match[2] || "").padEnd(4, "0");
  const units = BigInt(match[1]) * BANCOR_ASSET_SCALE + BigInt(fraction || "0");
  if (units > BANCOR_MAX_UNITS) return null;

  if (adjustment === "increment") {
    return units < BANCOR_MAX_UNITS ? formatBancorUnits(units + 1n) : null;
  }
  if (units <= 0n) return null;
  if (adjustment === "decrement") return formatBancorUnits(units > 1n ? units - 1n : 1n);
  const remainingPercent = adjustment === "decrease_25_percent" ? 75n : 50n;
  const adjusted = (units * remainingPercent) / 100n;
  return formatBancorUnits(adjusted > 0n ? adjusted : 1n);
}

export function parseBancorSlippagePercent(value: string): number | null {
  const match = /^(0|[1-9][0-9]*)(?:\.([0-9]{1,2}))?$/.exec(value.trim());
  if (!match) return null;
  const basisPoints = Number(match[1]) * 100 + Number((match[2] || "").padEnd(2, "0"));
  return Number.isSafeInteger(basisPoints) && basisPoints <= BANCOR_MAX_SLIPPAGE_BPS
    ? basisPoints
    : null;
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

export function calculateBancorEstimatedOutput({
  side,
  inputAmount,
  market,
}: {
  side: "buy" | "sell";
  inputAmount: string;
  market: NniBancorMarketResponse | null;
}): string | null {
  const inputUnits = parseBancorInputUnits(inputAmount);
  if (inputUnits === null || !market || !Number.isSafeInteger(market.fee_bps) || market.fee_bps < 0 || market.fee_bps >= 10_000) {
    return null;
  }
  const feeUnits = market.fee_bps === 0
    ? 0n
    : (inputUnits * BigInt(market.fee_bps) + 9_999n) / 10_000n;
  const curveInputUnits = inputUnits - feeUnits;
  if (curveInputUnits <= 0n) return null;
  const inputReserveUnits = BigInt(side === "buy" ? market.usd_reserve_units : market.point_reserve_units);
  const outputReserveUnits = BigInt(side === "buy" ? market.point_reserve_units : market.usd_reserve_units);
  const outputUnits = (curveInputUnits * outputReserveUnits) / (inputReserveUnits + curveInputUnits);
  return outputUnits > 0n ? formatBancorUnits(outputUnits) : null;
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
  if (code === "nni_bancor_trade_rate_limited" || code === "nni_bancor_ip_rate_limited") {
    return t("请求太频繁，请稍等片刻再试。", "Requests are too frequent. Wait a moment and try again.");
  }
  if (code === "nni_bancor_trade_outcome_unknown") {
    return t(
      "提交成交后网络中断，结果暂时无法确认。请先刷新余额和成交记录，不要立即重复交易。",
      "The connection ended after submission, so the outcome is not yet known. Refresh balances and trade history before trying again.",
    );
  }
  if (
    code === "nni_bancor_candles_contract_invalid"
    || code === "nni_bancor_candles_body_invalid"
  ) {
    return t(
      "K 线数据版本不完整，已停止合并旧数据。请刷新后重试。",
      "The candlestick response is incomplete, so older data was not merged. Refresh and try again.",
    );
  }
  return code || fallback;
}

export function useBancorRuntime({
  apiFetch,
  cacheScope,
  t,
}: {
  apiFetch: ApiFetch;
  cacheScope: string;
  t: Translate;
}) {
  const [market, setMarket] = useState<NniBancorMarketResponse | null>(null);
  const [candles, setCandles] = useState<NniBancorCandlesResponse | null>(null);
  const [account, setAccount] = useState<NniBancorAccountResponse | null>(null);
  const [marketTrades, setMarketTrades] = useState<NniBancorMarketTradesResponse | null>(null);
  const [quote, setQuote] = useState<NniBancorQuoteResponse | null>(null);
  const [lastTrade, setLastTrade] = useState<NniBancorTradeResponse | null>(null);
  const [marketLoading, setMarketLoading] = useState(false);
  const [candlesLoading, setCandlesLoading] = useState(false);
  const [candlesOlderLoading, setCandlesOlderLoading] = useState(false);
  const [candlesHasOlder, setCandlesHasOlder] = useState(false);
  const [candlesError, setCandlesError] = useState<string | null>(null);
  const [candleIntervalSeconds, setCandleIntervalSeconds] = useState(BANCOR_DEFAULT_CANDLE_INTERVAL_SECONDS);
  const [accountLoading, setAccountLoading] = useState(false);
  const [marketTradesLoading, setMarketTradesLoading] = useState(false);
  const [marketTradesError, setMarketTradesError] = useState<string | null>(null);
  const [quoteLoading, setQuoteLoading] = useState(false);
  const [tradeLoading, setTradeLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const candleIntervalRef = useRef(BANCOR_DEFAULT_CANDLE_INTERVAL_SECONDS);
  const candleCacheRef = useRef(new Map<string, {
    response: NniBancorCandlesResponse;
    etag: string | null;
    etagRequestLimit: number | null;
  }>());
  const candleRequestsRef = useRef(new Map<string, Promise<NniBancorCandlesResponse | null>>());
  const candleCacheScopeRef = useRef(cacheScope);
  candleCacheScopeRef.current = cacheScope;

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

  const fetchMarketTrades = async (silent = false) => {
    if (!silent) setMarketTradesLoading(true);
    try {
      const response = await apiFetch("/v1/nni/bancor/trades");
      const body = (await response.json()) as ApiResponse<NniBancorMarketTradesResponse>;
      if (!response.ok || !body.ok || !body.data) {
        throw new Error(readError(body, `Market trades load failed (${response.status})`));
      }
      setMarketTrades(body.data);
      setMarketTradesError(null);
      return body.data;
    } catch (cause) {
      if (!silent) {
        setMarketTradesError(
          cause instanceof Error ? cause.message : t("市场成交记录读取失败。", "Market trades could not be loaded."),
        );
      }
      return null;
    } finally {
      if (!silent) setMarketTradesLoading(false);
    }
  };

  const fetchCandles = (
    intervalSeconds = candleIntervalRef.current,
    silent = false,
    forceAfterInFlight = false,
    endTimeUnix?: number,
  ): Promise<NniBancorCandlesResponse | null> => {
    const requestScope = cacheScope;
    const cacheKey = `${requestScope}\n${intervalSeconds}`;
    const requestKey = `${cacheKey}\n${endTimeUnix ?? "latest"}`;
    const inFlight = candleRequestsRef.current.get(requestKey);
    if (inFlight) {
      if (!forceAfterInFlight) return inFlight;
      return inFlight.then(() => fetchCandles(intervalSeconds, silent, false, endTimeUnix));
    }
    const loadingLatest = !silent && endTimeUnix === undefined;
    if (loadingLatest && candleIntervalRef.current === intervalSeconds) setCandlesLoading(true);

    const request = (async () => {
      let cachedSnapshot = candleCacheRef.current.get(cacheKey) ?? null;
      if (!cachedSnapshot) {
        const persistent = await readBancorCandleCache(requestScope, intervalSeconds);
        if (persistent) {
          cachedSnapshot = {
            response: persistent.response,
            etag: persistent.etag,
            etagRequestLimit: persistent.etagRequestLimit,
          };
          candleCacheRef.current.set(cacheKey, cachedSnapshot);
          if (
            candleCacheScopeRef.current === requestScope
            && candleIntervalRef.current === intervalSeconds
          ) {
            setCandles(persistent.response);
            setCandlesHasOlder(hasEarlierBancorCandles(persistent.response));
            setCandlesError(null);
          }
        }
      }

      const refreshLimit = endTimeUnix === undefined
        ? calculateBancorCandleRefreshLimit(cachedSnapshot?.response ?? null, intervalSeconds)
        : BANCOR_CANDLE_REQUEST_MAX_CANDLES;
      const requestHeaders: Record<string, string> = {};
      if (
        endTimeUnix === undefined
        &&
        cachedSnapshot?.etag
        && cachedSnapshot.etagRequestLimit === refreshLimit
      ) {
        requestHeaders["If-None-Match"] = cachedSnapshot.etag;
      }

      try {
        const response = await apiFetch(buildBancorCandlesPath(intervalSeconds, refreshLimit, endTimeUnix), {
          cache: "no-store",
          headers: requestHeaders,
        });
        if (response.status === 304) {
          if (!cachedSnapshot) throw new Error("nni_bancor_candle_cache_miss");
          const validatedSnapshot = {
            response: cachedSnapshot.response,
            etag: response.headers.get("etag") || cachedSnapshot.etag,
            etagRequestLimit: refreshLimit,
          };
          candleCacheRef.current.set(cacheKey, validatedSnapshot);
          void writeBancorCandleCache({
            scope: requestScope,
            intervalSeconds,
            response: validatedSnapshot.response,
            etag: validatedSnapshot.etag,
            etagRequestLimit: refreshLimit,
          });
          if (
            candleCacheScopeRef.current === requestScope
            && candleIntervalRef.current === intervalSeconds
          ) {
            setCandles(validatedSnapshot.response);
            setCandlesError(null);
          }
          return validatedSnapshot.response;
        }

        const body = (await response.json()) as ApiResponse<NniBancorCandlesResponse>;
        if (!response.ok || !body.ok || !body.data) {
          throw new Error(readError(body, `Candle load failed (${response.status})`));
        }
        if (!isBancorCandleResponse(body.data, intervalSeconds)) {
          throw new Error(formatBancorApiError("nni_bancor_candles_contract_invalid", t, ""));
        }
        const merged = mergeBancorCandleResponses(
          cachedSnapshot?.response ?? null,
          body.data,
        );
        const updatedSnapshot = {
          response: merged,
          etag: endTimeUnix === undefined
            ? response.headers.get("etag")
            : cachedSnapshot?.etag ?? null,
          etagRequestLimit: endTimeUnix === undefined
            ? refreshLimit
            : cachedSnapshot?.etagRequestLimit ?? null,
        };
        candleCacheRef.current.set(cacheKey, updatedSnapshot);
        void writeBancorCandleCache({
          scope: requestScope,
          intervalSeconds,
          response: merged,
          etag: updatedSnapshot.etag,
          etagRequestLimit: refreshLimit,
        });
        if (
          candleCacheScopeRef.current === requestScope
          && candleIntervalRef.current === intervalSeconds
        ) {
          setCandles(merged);
          if (endTimeUnix === undefined) {
            setCandlesHasOlder(hasEarlierBancorCandles(merged));
          } else {
            setCandlesHasOlder(
              body.data.candles.length >= BANCOR_CANDLE_REQUEST_MAX_CANDLES
              && hasEarlierBancorCandles(merged),
            );
          }
          setCandlesError(null);
        }
        return merged;
      } catch (cause) {
        if (
          !silent
          && candleCacheScopeRef.current === requestScope
          && candleIntervalRef.current === intervalSeconds
        ) {
          const contractError = formatBancorApiError("nni_bancor_candles_contract_invalid", t, "");
          if (cause instanceof Error && cause.message === contractError) {
            setCandlesError(contractError);
          } else if (cachedSnapshot) {
            setCandlesError(t(
              "暂时无法更新，正在显示浏览器中最近保存的 K 线。",
              "The latest update is unavailable, so the most recently saved candlesticks are shown.",
            ));
          } else {
            setCandlesError(cause instanceof Error ? cause.message : t("K 线读取失败。", "Candlestick data could not be loaded."));
          }
        }
        return cachedSnapshot?.response ?? null;
      } finally {
        if (
          loadingLatest
          && candleCacheScopeRef.current === requestScope
          && candleIntervalRef.current === intervalSeconds
        ) {
          setCandlesLoading(false);
        }
      }
    })();

    candleRequestsRef.current.set(requestKey, request);
    void request.finally(() => {
      if (candleRequestsRef.current.get(requestKey) === request) {
        candleRequestsRef.current.delete(requestKey);
      }
    });
    return request;
  };

  const changeCandleInterval = async (intervalSeconds: number) => {
    const cached = candleCacheRef.current.get(`${cacheScope}\n${intervalSeconds}`)?.response ?? null;
    candleIntervalRef.current = intervalSeconds;
    setCandleIntervalSeconds(intervalSeconds);
    setCandles((current) => projectBancorCandlesForInterval(current, intervalSeconds, cached));
    setCandlesError(null);
    setCandlesHasOlder(cached ? hasEarlierBancorCandles(cached) : false);
    return fetchCandles(intervalSeconds);
  };

  const loadOlderCandles = async () => {
    const intervalSeconds = candleIntervalRef.current;
    const snapshot = candleCacheRef.current.get(`${cacheScope}\n${intervalSeconds}`)?.response;
    const current = snapshot?.interval_seconds === intervalSeconds ? snapshot : candles;
    const oldestBucketStart = current?.candles[0]?.bucket_start_unix;
    if (oldestBucketStart == null || oldestBucketStart <= current.market_created_at_unix) {
      setCandlesHasOlder(false);
      return current ?? null;
    }
    setCandlesOlderLoading(true);
    try {
      const result = await fetchCandles(intervalSeconds, false, false, oldestBucketStart - 1);
      return result;
    } finally {
      if (candleIntervalRef.current === intervalSeconds) setCandlesOlderLoading(false);
    }
  };

  const preview = async (
    side: "buy" | "sell",
    inputAmount: string,
    slippageBps = BANCOR_DEFAULT_SLIPPAGE_BPS,
  ) => {
    setError(null);
    setMessage(null);
    setLastTrade(null);
    const validationError = validateBancorTradeInput({ side, inputAmount, market, account });
    if (validationError) {
      setQuote(null);
      setError(formatBancorApiError(validationError, t, t("金额无法交易。", "This amount cannot be traded.")));
      return null;
    }
    if (!Number.isSafeInteger(slippageBps) || slippageBps < 0 || slippageBps > BANCOR_MAX_SLIPPAGE_BPS) {
      setQuote(null);
      setError(t("滑点必须在 0% 到 50% 之间，最多保留两位小数。", "Slippage must be between 0% and 50%, with at most two decimal places."));
      return null;
    }
    setQuoteLoading(true);
    try {
      const response = await apiFetch("/v1/nni/bancor/quote", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ side, input_amount: inputAmount, slippage_bps: slippageBps }),
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
        fetchMarketTrades(true),
        fetchCandles(candleIntervalSeconds, true, true),
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
    marketTrades,
    quote,
    lastTrade,
    marketLoading,
    candlesLoading,
    candlesOlderLoading,
    candlesHasOlder,
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
    loadOlderCandles,
    fetchAccount,
    fetchMarketTrades,
    preview,
    trade,
    clearQuote,
  };
}
