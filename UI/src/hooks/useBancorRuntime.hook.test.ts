import assert from "node:assert/strict";
import test from "node:test";

import React from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { ripemd160 } from "@noble/hashes/legacy.js";
import { base58 } from "@scure/base";

import { validateNniOwnerPrivateKey } from "../lib/nni-owner-public-key";
import type { ApiResponse, NniBancorCandlesResponse } from "../types/api";
import { formatBancorApiError, useBancorRuntime } from "./useBancorRuntime";

function encodeTestOwnerPrivateKey(secretKey: Uint8Array): string {
  const suffix = new TextEncoder().encode("K1");
  const payload = new Uint8Array(secretKey.length + suffix.length);
  payload.set(secretKey);
  payload.set(suffix, secretKey.length);
  const checksum = ripemd160(payload).slice(0, 4);
  const encoded = new Uint8Array(secretKey.length + checksum.length);
  encoded.set(secretKey);
  encoded.set(checksum, secretKey.length);
  return base58.encode(encoded);
}

function candles(intervalSeconds: number, bucketStartUnix = 1_800_000_000): NniBancorCandlesResponse {
  return {
    schema_version: 1,
    status: "bancor_candles",
    market_id: "aic-usd-v1",
    market_version: 7,
    market_created_at_unix: 1_800_000_000,
    price_kind: "execution_average_usd_per_aic",
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
      aic_volume_units: "10000",
      aic_volume: "1.00000000",
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

test("BANCOR explains protected repricing failures without exposing machine error codes", () => {
  const t = (zh: string) => zh;
  assert.equal(
    formatBancorApiError("nni_bancor_slippage_exceeded", t, "fallback"),
    "价格变化已超出你设置的最低到账保护，本次交易未成交。请重新获取报价。",
  );
  assert.equal(
    formatBancorApiError("nni_bancor_fee_limit_exceeded", t, "fallback"),
    "当前手续费已超过签名时允许的上限，本次交易未成交。请重新获取报价。",
  );
});

test("BANCOR refreshes a changed dynamic minimum after the backend rejects a stale preview", async () => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  let marketRequestCount = 0;
  const apiFetch = (path: string): Promise<Response> => {
    if (path === "/v1/nni/bancor/market") {
      marketRequestCount += 1;
      const minimumUnits = marketRequestCount === 1 ? "200" : "400";
      const minimumAmount = marketRequestCount === 1 ? "0.00000200" : "0.00000400";
      return Promise.resolve(apiResponse({
        status: "open",
        market_id: "aic-usd-v1",
        fee_bps: 50,
        min_trade_usd: minimumAmount,
        min_trade_usd_units: minimumUnits,
        min_trade_aic: "0.00010052",
        min_trade_aic_units: "10052",
        minimum_fee_units: "1",
        minimum_output_units: "1",
        aic_reserve_units: "10000000000000000",
        usd_reserve_units: "1000000000000",
      }));
    }
    if (path === "/v1/nni/bancor/quote") {
      return Promise.resolve(new Response(JSON.stringify({
        ok: false,
        data: null,
        error: "nni_bancor_trade_below_minimum",
      }), {
        status: 400,
        headers: { "content-type": "application/json" },
      }));
    }
    throw new Error(`unexpected request: ${path}`);
  };
  let runtime: ReturnType<typeof useBancorRuntime> | null = null;
  function Probe() {
    runtime = useBancorRuntime({ apiFetch, cacheScope: "dynamic-minimum-refresh-test", t: (zh) => zh });
    return null;
  }

  let renderer: ReactTestRenderer | null = null;
  await act(async () => {
    renderer = create(React.createElement(Probe));
  });
  await act(async () => {
    await runtime!.fetchMarket();
  });
  await act(async () => {
    await runtime!.preview("buy", "0.00000200", 300, true);
  });

  assert.equal(marketRequestCount, 2);
  assert.equal(runtime!.market?.min_trade_usd_units, "400");
  assert.equal(runtime!.error, "金额不能小于 0.000004 USD。");

  await act(async () => {
    renderer!.unmount();
  });
});

test("BANCOR keeps the asset-account setup guide until account access succeeds", async () => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  let ownerMissing = true;
  const apiFetch = (path: string): Promise<Response> => {
    if (path.startsWith("/v1/nni/bancor/account?")) {
      if (ownerMissing) {
        return Promise.resolve(new Response(JSON.stringify({
          ok: false,
          data: null,
          error: "nni_asset_owner_required",
        }), {
          status: 409,
          headers: { "content-type": "application/json" },
        }));
      }
      return Promise.resolve(apiResponse({ trades: [] }));
    }
    if (path === "/v1/nni/bancor/market") {
      return Promise.resolve(apiResponse({ status: "open" }));
    }
    throw new Error(`unexpected request: ${path}`);
  };
  let runtime: ReturnType<typeof useBancorRuntime> | null = null;
  function Probe() {
    runtime = useBancorRuntime({ apiFetch, cacheScope: "asset-owner-guide-test", t: (zh) => zh });
    return null;
  }

  let renderer: ReactTestRenderer | null = null;
  await act(async () => {
    renderer = create(React.createElement(Probe));
  });
  await act(async () => {
    await runtime!.fetchAccount(1);
  });
  assert.equal(runtime!.assetOwnerRequired, true);
  assert.match(runtime!.error ?? "", /生成并绑定资产账号/);
  assert.doesNotMatch(runtime!.error ?? "", /nni_asset_owner_required/);

  await act(async () => {
    await runtime!.fetchMarket();
  });
  assert.equal(runtime!.assetOwnerRequired, true);

  ownerMissing = false;
  await act(async () => {
    await runtime!.fetchAccount(1);
  });
  assert.equal(runtime!.assetOwnerRequired, false);

  await act(async () => {
    renderer!.unmount();
  });
});

test("BANCOR keeps a revoked-device recovery guide until account access succeeds", async () => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  let deviceAuthorized = false;
  const apiFetch = (path: string): Promise<Response> => {
    if (!path.startsWith("/v1/nni/bancor/account?")) {
      throw new Error(`unexpected request: ${path}`);
    }
    if (!deviceAuthorized) {
      return Promise.resolve(new Response(JSON.stringify({
        ok: false,
        data: null,
        error: "nni_asset_device_not_authorized",
      }), {
        status: 403,
        headers: { "content-type": "application/json" },
      }));
    }
    return Promise.resolve(apiResponse({ trades: [] }));
  };
  let runtime: ReturnType<typeof useBancorRuntime> | null = null;
  function Probe() {
    runtime = useBancorRuntime({ apiFetch, cacheScope: "revoked-device-guide-test", t: (zh) => zh });
    return null;
  }

  let renderer: ReactTestRenderer | null = null;
  await act(async () => {
    renderer = create(React.createElement(Probe));
  });
  await act(async () => {
    await runtime!.fetchAccount(1);
  });
  assert.equal(runtime!.assetOwnerRequired, true);
  assert.equal(runtime!.assetOwnerAccessErrorCode, "nni_asset_device_not_authorized");
  assert.match(runtime!.error ?? "", /重新绑定资产账号/);
  assert.doesNotMatch(runtime!.error ?? "", /nni_asset_device_not_authorized/);

  deviceAuthorized = true;
  await act(async () => {
    await runtime!.fetchAccount(1);
  });
  assert.equal(runtime!.assetOwnerRequired, false);
  assert.equal(runtime!.assetOwnerAccessErrorCode, null);

  await act(async () => {
    renderer!.unmount();
  });
});

test("BANCOR account reads never expose a nested NNI admission code", async () => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  const apiFetch = (path: string): Promise<Response> => {
    if (!path.startsWith("/v1/nni/bancor/account?")) {
      throw new Error(`unexpected request: ${path}`);
    }
    return Promise.resolve(new Response(JSON.stringify({
      ok: false,
      data: {
        attempts: [{
          node_url: "https://nni.example.invalid",
          error_code: "nni_public_key_whitelist_empty",
        }],
      },
      error: "nni_bancor_account_nodes_unavailable",
    }), {
      status: 502,
      headers: { "content-type": "application/json" },
    }));
  };
  let runtime: ReturnType<typeof useBancorRuntime> | null = null;
  function Probe() {
    runtime = useBancorRuntime({ apiFetch, cacheScope: "device-admission-error-test", t: (zh) => zh });
    return null;
  }

  let renderer: ReactTestRenderer | null = null;
  await act(async () => {
    renderer = create(React.createElement(Probe));
  });
  await act(async () => {
    await runtime!.fetchAccount(1);
  });
  assert.equal(runtime!.error, null);
  assert.equal(runtime!.hardwareAccountAccessUnavailable, true);

  await act(async () => {
    renderer!.unmount();
  });
});

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
      : Promise.resolve(response(candles(3_600)));
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
    await runtime!.fetchCandles();
  });
  assert.equal(runtime!.candles?.interval_seconds, 3_600);

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
      return Promise.resolve(response(candles(3_600, 1_800_000_000)));
    }
    latestRequestCount += 1;
    if (latestRequestCount > 1) refreshedLatestHeaders = new Headers(init?.headers);
    return Promise.resolve(response(candles(3_600, 1_800_003_600), '"latest-page"'));
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
    await runtime!.fetchCandles();
  });
  assert.equal(runtime!.candlesHasOlder, true);
  await act(async () => {
    await runtime!.loadOlderCandles();
  });
  assert.equal(historicalHeaders?.has("If-None-Match"), false);
  assert.deepEqual(
    runtime!.candles?.candles.map((candle) => candle.bucket_start_unix),
    [1_800_000_000, 1_800_003_600],
  );
  assert.equal(runtime!.candlesHasOlder, false);
  await act(async () => {
    await runtime!.fetchCandles();
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
  const ownerPrivateKey = encodeTestOwnerPrivateKey(
    Uint8Array.from({ length: 32 }, (_, index) => index + 1),
  );
  const ownerValidation = validateNniOwnerPrivateKey(ownerPrivateKey);
  assert.equal(ownerValidation.ok, true);
  if (!ownerValidation.ok) return;
  const candleHeaders: Headers[] = [];
  let candleRequestCount = 0;
  const apiFetch = (path: string, init?: RequestInit): Promise<Response> => {
    requestedPaths.push(path);
    if (path.startsWith("/v1/nni/bancor/candles?")) {
      candleHeaders.push(new Headers(init?.headers));
      candleRequestCount += 1;
      const data = candles(3_600);
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
      if (tradeBodies.length === 1) {
        return Promise.resolve(apiResponse({
          status: "bancor_trade_challenge_created",
          task_id: "trade-task-1",
          quote_id: "quote-1",
          signing_payload: "bancor-trade-signing-payload",
          asset_owner_pubkey: ownerValidation.publicKey,
          node_url: "https://nni.example.test",
        }));
      }
      return Promise.resolve(apiResponse({ trade: { trade_id: "trade-1" } }));
    }
    if (path === "/v1/nni/bancor/market") {
      return Promise.resolve(apiResponse({
        status: "open",
        market_id: "aic-usd-v1",
        fee_bps: 50,
        min_trade_usd: "0.00000200",
        min_trade_usd_units: "200",
        min_trade_aic: "0.00010052",
        min_trade_aic_units: "10052",
        minimum_fee_units: "1",
        minimum_output_units: "1",
        aic_reserve_units: "10000000000000000",
        usd_reserve_units: "1000000000000",
      }));
    }
    if (path.startsWith("/v1/nni/bancor/account?")) {
      return Promise.resolve(apiResponse({
        aic_balance_units: "10000000000",
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
      ownerPrivateKey,
    }));
  });

  assert.equal(candleRequestCount, 2);
  assert.equal(candleHeaders[1].has("If-None-Match"), false);
  assert.ok(requestedPaths.some((path) => path.includes("/v1/nni/bancor/candles?interval_seconds=3600")));
  assert.ok(requestedPaths.includes("/v1/nni/bancor/market"));
  assert.ok(requestedPaths.includes("/v1/nni/bancor/account?page=1&per_page=10"));
  assert.ok(requestedPaths.includes("/v1/nni/bancor/trades"));
  assert.equal(tradeBodies.length, 2);
  assert.equal(tradeBodies[0].asset_owner_pubkey, ownerValidation.publicKey);
  assert.equal(tradeBodies[1].task_id, "trade-task-1");
  assert.equal(typeof tradeBodies[1].owner_signature, "string");
  for (const body of tradeBodies) {
    assert.equal(Object.hasOwn(body, "owner_private_key"), false);
  }
  assert.equal(runtime!.candles?.candles[0]?.close, "0.000110000000");
  assert.equal(runtime!.message, "交易已完成，余额和市场储备已经更新。");

  await act(async () => {
    renderer!.unmount();
  });
});
