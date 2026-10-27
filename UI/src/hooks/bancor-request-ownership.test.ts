import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { useBancorRuntime } from "./useBancorRuntime";

globalThis.IS_REACT_ACT_ENVIRONMENT = true;
const t = (zh: string) => zh;
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}
function json(data: unknown) {
  return new Response(JSON.stringify({ ok: true, data }), { status: 200 });
}
const quote = { side: "buy", input_amount: "1", min_output_amount: "1", slippage_bps: 300 };

for (const [method, field, loading] of [
  ["fetchMarket", "market", "marketLoading"],
  ["fetchAccount", "account", "accountLoading"],
  ["fetchMarketTrades", "marketTrades", "marketTradesLoading"],
] as const) {
  test(`Bancor ${method} rejects old owner responses without clearing new loading state`, async () => {
    const old = deferred<Response>();
    const fresh = deferred<Response>();
    let calls = 0;
    const apiFetch = async () => ++calls === 1 ? old.promise : fresh.promise;
    let runtime!: ReturnType<typeof useBancorRuntime>;
    function Probe({ scope }: { scope: string }) {
      runtime = useBancorRuntime({ apiFetch, t, cacheScope: "endpoint", requestScope: scope });
      return null;
    }
    let renderer!: ReactTestRenderer;
    await act(async () => { renderer = create(React.createElement(Probe, { scope: "A" })); });
    let a!: Promise<unknown>;
    let b!: Promise<unknown>;
    await act(async () => { a = runtime[method](); });
    await act(async () => { renderer.update(React.createElement(Probe, { scope: "B" })); });
    await act(async () => { renderer.update(React.createElement(Probe, { scope: "A" })); });
    await act(async () => { b = runtime[method](); });
    assert.equal(calls, 2);
    await act(async () => { old.resolve(json({ marker: "old" })); assert.equal(await a, null); });
    assert.equal(runtime[field], null);
    assert.equal(runtime[loading], true);
    await act(async () => { fresh.resolve(json({ marker: "fresh" })); await b; });
    assert.equal((runtime[field] as unknown as { marker: string }).marker, "fresh");
    assert.equal(runtime[loading], false);
    await act(async () => { renderer.unmount(); });
  });
}

test("Bancor only accepts the latest quote and clearQuote invalidates pending previews", async () => {
  const old = deferred<Response>();
  const fresh = deferred<Response>();
  const third = deferred<Response>();
  const requests = [old, fresh, third];
  let calls = 0;
  const apiFetch = async () => requests[calls++].promise;
  let runtime!: ReturnType<typeof useBancorRuntime>;
  function Probe() {
    runtime = useBancorRuntime({ apiFetch, t, cacheScope: "endpoint" });
    return null;
  }
  let renderer!: ReactTestRenderer;
  await act(async () => { renderer = create(React.createElement(Probe)); });
  let a!: Promise<unknown>;
  let b!: Promise<unknown>;
  await act(async () => {
    a = runtime.preview("buy", "1", 300, true);
    b = runtime.preview("buy", "2", 300, true);
  });
  await act(async () => { fresh.resolve(json({ ...quote, input_amount: "2" })); await b; });
  await act(async () => { old.resolve(json(quote)); assert.equal(await a, null); });
  assert.equal(runtime.quote?.input_amount, "2");
  let c!: Promise<unknown>;
  await act(async () => { c = runtime.preview("buy", "3", 300, true); });
  await act(async () => { runtime.clearQuote(); });
  await act(async () => { third.resolve(json({ ...quote, input_amount: "3" })); assert.equal(await c, null); });
  assert.equal(runtime.quote, null);
  assert.equal(runtime.quoteLoading, false);
  await act(async () => { renderer.unmount(); });
});

test("Bancor account pagination keeps the latest selected page", async () => {
  const first = deferred<Response>();
  const second = deferred<Response>();
  const apiFetch = async (path: string) => path.includes("page=2") ? second.promise : first.promise;
  let runtime!: ReturnType<typeof useBancorRuntime>;
  function Probe() { runtime = useBancorRuntime({ apiFetch, t, cacheScope: "endpoint" }); return null; }
  let renderer!: ReactTestRenderer;
  await act(async () => { renderer = create(React.createElement(Probe)); });
  let a!: Promise<unknown>;
  let b!: Promise<unknown>;
  await act(async () => { a = runtime.fetchAccount(1); b = runtime.fetchAccount(2); });
  await act(async () => { second.resolve(json({ page: 2 })); await b; });
  await act(async () => { first.resolve(json({ page: 1 })); assert.equal(await a, null); });
  assert.equal(runtime.account?.page, 2);
  await act(async () => { renderer.unmount(); });
});

test("Bancor rejects stale trade closures and suppresses same-tick duplicate trades", async () => {
  const pending = deferred<Response>();
  let trades = 0;
  const apiFetch = async (path: string) => {
    if (path.endsWith("/quote")) return json(quote);
    if (path.endsWith("/trade")) { trades++; return pending.promise; }
    throw new Error("unexpected_read");
  };
  let runtime!: ReturnType<typeof useBancorRuntime>;
  function Probe({ scope }: { scope: string }) {
    runtime = useBancorRuntime({ apiFetch, t, cacheScope: "endpoint", requestScope: scope });
    return null;
  }
  let renderer!: ReactTestRenderer;
  await act(async () => { renderer = create(React.createElement(Probe, { scope: "A" })); });
  await act(async () => { await runtime.preview("buy", "1", 300, true); });
  const staleTrade = runtime.trade;
  let first!: Promise<unknown>;
  await act(async () => {
    first = runtime.trade();
    assert.equal(await runtime.trade(), null);
  });
  assert.equal(trades, 1);
  await act(async () => { renderer.update(React.createElement(Probe, { scope: "B" })); });
  assert.equal(runtime.quote, null);
  await act(async () => { assert.equal(await staleTrade(), null); });
  await act(async () => {
    pending.resolve(json({ trade: { id: "completed-old-owner" } }));
    assert.equal(await first, null);
  });
  assert.equal(runtime.lastTrade, null);
  assert.equal(runtime.message, null);
  assert.equal(trades, 1);
  await act(async () => { renderer.unmount(); });
});
