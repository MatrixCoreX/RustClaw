import assert from "node:assert/strict";
import test from "node:test";

import React from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";

import type { ApiResponse, NniBancorCandlesResponse } from "../types/api";
import { useBancorRuntime } from "./useBancorRuntime";

function candles(intervalSeconds: number, bucketStartUnix = 1_800_000_000): NniBancorCandlesResponse {
  return {
    schema_version: 1,
    status: "bancor_candles",
    market_id: "point-usd-v1",
    market_version: 7,
    market_created_at_unix: 1_800_000_000,
    price_kind: "execution_average_usd_per_point",
    interval_seconds: intervalSeconds,
    start_time_unix: bucketStartUnix,
    end_time_unix: bucketStartUnix + intervalSeconds,
    price_scale: 1_000_000_000_000,
    price_decimal_places: 12,
    candles: [{
      bucket_start_unix: bucketStartUnix,
      bucket_end_unix: bucketStartUnix + intervalSeconds,
      open: "0.000100000000",
      high: "0.000100000000",
      low: "0.000100000000",
      close: "0.000100000000",
      point_volume_units: "10000",
      point_volume: "1.00000000",
      usd_volume_units: "1",
      usd_volume: "0.00010000",
      trade_count: 1,
      has_trades: true,
    }],
  };
}

function response(data: NniBancorCandlesResponse, etag?: string): Response {
  const body: ApiResponse<NniBancorCandlesResponse> = { ok: true, data, error: null };
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: {
      "content-type": "application/json",
      ...(etag ? { etag } : {}),
    },
  });
}

function apiResponse(data: unknown): Response {
  return new Response(JSON.stringify({ ok: true, data, error: null }), {
    status: 200,
    headers: { "content-type": "application/json" },
  });
}

test("BANCOR hook never exposes old-period candles after a failed interval switch", async () => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  let rejectOneMinute: ((cause: Error) => void) | undefined;
  const oneMinuteResponse = new Promise<Response>((_resolve, reject) => {
    rejectOneMinute = reject;
  });
  const apiFetch = (path: string): Promise<Response> => {
    const interval = new URL(path, "http://runtime.invalid").searchParams.get("interval_seconds");
    return interval === "60"
      ? oneMinuteResponse
      : Promise.resolve(response(candles(300)));
  };
  let runtime: ReturnType<typeof useBancorRuntime> | null = null;
  function Probe() {
    runtime = useBancorRuntime({ apiFetch, cacheScope: "hook-test", t: (zh) => zh });
    return null;
  }

  let renderer: ReactTestRenderer | null = null;
  await act(async () => {
    renderer = create(React.createElement(Probe));
  });
  await act(async () => {
    await runtime!.fetchCandles(300);
  });
  assert.equal(runtime!.candles?.interval_seconds, 300);

  let switching: Promise<NniBancorCandlesResponse | null> | null = null;
  await act(async () => {
    switching = runtime!.changeCandleInterval(60);
    await Promise.resolve();
  });
  assert.equal(runtime!.candleIntervalSeconds, 60);
  assert.equal(runtime!.candles, null);
  assert.equal(runtime!.candlesLoading, true);

  await act(async () => {
    rejectOneMinute!(new Error("network unavailable"));
    await switching!;
  });
  assert.equal(runtime!.candleIntervalSeconds, 60);
  assert.equal(runtime!.candles, null);
  assert.equal(runtime!.candlesLoading, false);
  assert.match(runtime!.candlesError ?? "", /network unavailable/);

  await act(async () => {
    renderer!.unmount();
  });
});

test("BANCOR hook paginates older candles without reusing the latest-page ETag", async () => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  let historicalHeaders: Headers | null = null;
  let refreshedLatestHeaders: Headers | null = null;
  let latestRequestCount = 0;
  const apiFetch = (path: string, init?: RequestInit): Promise<Response> => {
    const query = new URL(path, "http://runtime.invalid").searchParams;
    if (query.has("end_time_unix")) {
      historicalHeaders = new Headers(init?.headers);
      return Promise.resolve(response(candles(300, 1_800_000_000)));
    }
    latestRequestCount += 1;
    if (latestRequestCount > 1) refreshedLatestHeaders = new Headers(init?.headers);
    return Promise.resolve(response(candles(300, 1_800_000_300), '"latest-page"'));
  };
  let runtime: ReturnType<typeof useBancorRuntime> | null = null;
  function Probe() {
    runtime = useBancorRuntime({ apiFetch, cacheScope: "pagination-test", t: (zh) => zh });
    return null;
  }

  let renderer: ReactTestRenderer | null = null;
  await act(async () => {
    renderer = create(React.createElement(Probe));
  });
  await act(async () => {
    await runtime!.fetchCandles(300);
  });
  assert.equal(runtime!.candlesHasOlder, true);
  await act(async () => {
    await runtime!.loadOlderCandles();
  });
  assert.equal(historicalHeaders?.has("If-None-Match"), false);
  assert.deepEqual(
    runtime!.candles?.candles.map((candle) => candle.bucket_start_unix),
    [1_800_000_000, 1_800_000_300],
  );
  assert.equal(runtime!.candlesHasOlder, false);
  await act(async () => {
    await runtime!.fetchCandles(300);
  });
  assert.equal(refreshedLatestHeaders?.get("If-None-Match"), '"latest-page"');

  await act(async () => {
    renderer!.unmount();
  });
});

test("BANCOR refreshes the active candlesticks without a stale ETag after a successful trade", async () => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  const requestedPaths: string[] = [];
  const tradeBodies: Array<Record<string, unknown>> = [];
  const candleHeaders: Headers[] = [];
  let candleRequestCount = 0;
  const apiFetch = (path: string, init?: RequestInit): Promise<Response> => {
    requestedPaths.push(path);
    if (path.startsWith("/v1/nni/bancor/candles?")) {
      candleHeaders.push(new Headers(init?.headers));
      candleRequestCount += 1;
      const data = candles(300);
      data.market_version = candleRequestCount;
      data.candles[0].close = candleRequestCount === 1 ? "0.000100000000" : "0.000110000000";
      return Promise.resolve(response(data, `"candles-${candleRequestCount}"`));
    }
    if (path === "/v1/nni/bancor/quote") {
      return Promise.resolve(apiResponse({
        quote_id: "quote-1",
        side: "sell",
        input_amount: "100.00000000",
        min_output_amount: "0.00010000",
        slippage_bps: 50,
      }));
    }
    if (path === "/v1/nni/bancor/trade") {
      tradeBodies.push(JSON.parse(String(init?.body)) as Record<string, unknown>);
      return Promise.resolve(apiResponse({ trade: { trade_id: "trade-1" } }));
    }
    if (path === "/v1/nni/bancor/market") {
      return Promise.resolve(apiResponse({
        status: "open",
        market_id: "point-usd-v1",
        fee_bps: 50,
        point_reserve_units: "10000000000000000",
        usd_reserve_units: "1000000000000",
      }));
    }
    if (path.startsWith("/v1/nni/bancor/account?")) {
      return Promise.resolve(apiResponse({
        point_balance_units: "10000000000",
        usd_balance_units: "1000000000",
        page: 1,
        per_page: 10,
        total: 1,
        total_pages: 1,
        trades: [],
      }));
    }
    if (path === "/v1/nni/bancor/trades") {
      return Promise.resolve(apiResponse({ limit: 100, trades: [] }));
    }
    throw new Error(`unexpected request: ${path}`);
  };
  let runtime: ReturnType<typeof useBancorRuntime> | null = null;
  function Probe() {
    runtime = useBancorRuntime({ apiFetch, cacheScope: "trade-refresh-test", t: (zh) => zh });
    return null;
  }

  let renderer: ReactTestRenderer | null = null;
  await act(async () => {
    renderer = create(React.createElement(Probe));
  });
  await act(async () => {
    await Promise.all([
      runtime!.fetchCandles(),
      runtime!.fetchMarket(),
      runtime!.fetchAccount(1),
    ]);
  });
  assert.equal(candleHeaders[0].has("If-None-Match"), false);

  await act(async () => {
    await runtime!.preview("sell", "100.00000000", 50);
  });
  await act(async () => {
    assert.ok(await runtime!.trade({
      authorizationMode: "asset_owner",
      ownerPrivateKey: "transient-owner-private-key",
    }));
  });

  assert.equal(candleRequestCount, 2);
  assert.equal(candleHeaders[1].has("If-None-Match"), false);
  assert.ok(requestedPaths.some((path) => path.includes("/v1/nni/bancor/candles?interval_seconds=300")));
  assert.ok(requestedPaths.includes("/v1/nni/bancor/market"));
  assert.ok(requestedPaths.includes("/v1/nni/bancor/account?page=1&per_page=10"));
  assert.ok(requestedPaths.includes("/v1/nni/bancor/trades"));
  assert.deepEqual(tradeBodies, [{
    side: "sell",
    input_amount: "100.00000000",
    min_output: "0.00010000",
    slippage_bps: 50,
    authorization_mode: "asset_owner",
    owner_private_key: "transient-owner-private-key",
  }]);
  assert.equal(runtime!.candles?.candles[0]?.close, "0.000110000000");
  assert.equal(runtime!.message, "交易已完成，余额和市场储备已经更新。");

  await act(async () => {
    renderer!.unmount();
  });
});
