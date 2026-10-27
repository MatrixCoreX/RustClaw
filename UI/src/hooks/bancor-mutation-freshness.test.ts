import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { useBancorRuntime } from "./useBancorRuntime";

globalThis.IS_REACT_ACT_ENVIRONMENT = true;
const quote = { side: "buy", input_amount: "1", min_output_amount: "1", slippage_bps: 300 };
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}
function json(data: unknown) { return Response.json({ ok: true, data }); }
function candles(marketVersion = 1) {
  return json({
    schema_version: 1, status: "bancor_candles", market_id: "fixture", market_version: marketVersion,
    price_kind: "pool_marginal_usd_per_aic", interval_seconds: 3600,
    market_created_at_unix: 1000, start_time_unix: 0, end_time_unix: 3600,
    price_scale: 1000000000000, price_decimal_places: 12, candles: [],
  });
}
async function mount(apiFetch: (path: string, init?: RequestInit) => Promise<Response>) {
  let runtime!: ReturnType<typeof useBancorRuntime>;
  function Probe() { runtime = useBancorRuntime({ apiFetch, t: (zh) => zh, cacheScope: "mutation-fixture" }); return null; }
  let renderer!: ReactTestRenderer;
  await act(async () => { renderer = create(React.createElement(Probe)); });
  return { get: () => runtime, close: async () => { await act(async () => renderer.unmount()); } };
}

for (const method of ["fetchMarket", "fetchAccount", "fetchMarketTrades", "fetchCandles"] as const) {
  test(`confirmed trade refresh does not reuse a pre-trade ${method} response`, async () => {
    const old = deferred<Response>();
    const routes = { fetchMarket: "/market", fetchAccount: "/account?", fetchMarketTrades: "/trades", fetchCandles: "/candles?" };
    let reads = 0;
    let trades = 0;
    const mounted = await mount(async (path) => {
      if (path.endsWith("/quote")) return json(quote);
      if (path.endsWith("/trade")) { trades++; return json({ trade: { trade_id: "committed" } }); }
      if (path.includes(routes[method])) return ++reads === 1 ? old.promise
        : method === "fetchCandles" ? candles(2) : json({ marker: "post-trade", page: 1 });
      if (path.includes("/candles?")) return candles();
      return json({ marker: "other" });
    });
    await act(async () => { await mounted.get().preview("buy", "1", 300, true); });
    let pending!: Promise<unknown>;
    let trading!: Promise<unknown>;
    await act(async () => { pending = mounted.get()[method](); });
    await act(async () => { trading = mounted.get().trade(); });
    assert.equal(trades, 1);
    assert.equal(reads, 2);
    await act(async () => {
      old.resolve(method === "fetchCandles" ? candles(1) : json({ marker: "pre-trade", page: 1 }));
      assert.equal(await pending, null);
      assert.ok(await trading);
    });
    const field = method === "fetchMarket" ? "market" : method === "fetchAccount" ? "account" : "marketTrades";
    if (method === "fetchCandles") assert.equal(mounted.get().candles?.market_version, 2);
    else assert.equal((mounted.get()[field] as unknown as { marker: string }).marker, "post-trade");
    assert.equal(mounted.get().tradeLoading, false);
    await mounted.close();
  });
}

test("confirmed trade invalidates a pending quote without submitting another trade", async () => {
  const pending = deferred<Response>();
  const committed = deferred<Response>();
  let quotes = 0;
  let trades = 0;
  const mounted = await mount(async (path) => {
    if (path.endsWith("/quote")) return ++quotes === 1 ? json(quote) : pending.promise;
    if (path.endsWith("/trade")) { trades++; return committed.promise; }
    if (path.includes("/candles?")) return candles();
    return json({ page: 1 });
  });
  await act(async () => { await mounted.get().preview("buy", "1", 300, true); });
  let trading!: Promise<unknown>;
  await act(async () => { trading = mounted.get().trade(); });
  let preview!: Promise<unknown>;
  await act(async () => { preview = mounted.get().preview("buy", "2", 300, true); });
  await act(async () => { committed.resolve(json({ trade: { trade_id: "committed" } })); assert.ok(await trading); });
  await act(async () => { pending.resolve(json({ ...quote, input_amount: "2" })); assert.equal(await preview, null); });
  assert.equal(mounted.get().quote, null);
  assert.equal(mounted.get().quoteLoading, false);
  assert.equal(trades, 1);
  await mounted.close();
});

test("failed post-trade refresh preserves success without claiming refreshed balances", async () => {
  let trades = 0;
  const mounted = await mount(async (path) => {
    if (path.endsWith("/quote")) return json(quote);
    if (path.endsWith("/trade")) { trades++; return json({ trade: { trade_id: "committed" } }); }
    return Response.json({ ok: false, error: "fixture_read_failed" }, { status: 400 });
  });
  await act(async () => { await mounted.get().preview("buy", "1", 300, true); });
  await act(async () => { assert.ok(await mounted.get().trade()); });
  assert.equal(mounted.get().lastTrade?.trade.trade_id, "committed");
  assert.equal(mounted.get().error, null);
  assert.equal(mounted.get().message, "交易已完成，部分数据暂未刷新，请稍后刷新页面。");
  assert.equal(trades, 1);
  await mounted.close();
});

test("a late pre-trade account error cannot revoke the fresh account or leave loading stuck", async () => {
  const oldBody = deferred<unknown>();
  let reads = 0;
  const mounted = await mount(async (path) => {
    if (path.endsWith("/quote")) return json(quote);
    if (path.endsWith("/trade")) return json({ trade: { trade_id: "committed" } });
    if (path.includes("/account?")) return ++reads === 1
      ? { ok: false, json: () => oldBody.promise } as Response : json({ page: 1, marker: "fresh" });
    if (path.includes("/candles?")) return candles();
    return json({});
  });
  await act(async () => { await mounted.get().preview("buy", "1", 300, true); });
  let pending!: Promise<unknown>;
  await act(async () => { pending = mounted.get().fetchAccount(); });
  await act(async () => { assert.ok(await mounted.get().trade()); });
  await act(async () => { oldBody.resolve({ ok: false, error: "nni_asset_device_not_authorized" }); assert.equal(await pending, null); });
  assert.equal(reads, 2);
  assert.equal(mounted.get().assetOwnerRequired, false);
  assert.equal(mounted.get().error, null);
  assert.equal(mounted.get().accountLoading, false);
  assert.equal((mounted.get().account as unknown as { marker: string }).marker, "fresh");
  await mounted.close();
});

test("rejected trade preserves the quote and does not invalidate a legitimate in-flight read", async () => {
  const pending = deferred<Response>();
  let reads = 0;
  let trades = 0;
  const mounted = await mount(async (path) => {
    if (path.endsWith("/quote")) return json(quote);
    if (path.endsWith("/trade")) { trades++; return Response.json({ ok: false, error: "fixture_rejected" }, { status: 400 }); }
    reads++;
    return pending.promise;
  });
  await act(async () => { await mounted.get().preview("buy", "1", 300, true); });
  let market!: Promise<unknown>;
  await act(async () => { market = mounted.get().fetchMarket(); });
  await act(async () => { assert.equal(await mounted.get().trade(), null); });
  assert.equal(reads, 1);
  assert.equal(trades, 1);
  await act(async () => { pending.resolve(json({ marker: "legitimate" })); assert.ok(await market); });
  assert.equal(mounted.get().lastTrade, null);
  assert.equal(mounted.get().quote?.input_amount, "1");
  assert.equal(mounted.get().message, null);
  await mounted.close();
});
