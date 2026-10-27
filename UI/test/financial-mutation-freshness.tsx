import { useRef } from "react";
import { createRoot } from "react-dom/client";
import { useBancorRuntime } from "../src/hooks/useBancorRuntime";
import "../src/index.css";

function Fixture() {
  const marketVersion = useRef(0);
  const slowReads = useRef(false);
  const failRefresh = useRef(false);
  const pendingTrade = useRef<(() => void) | null>(null);
  const apiFetch = async (path: string, init?: RequestInit) => {
    const version = marketVersion.current;
    const quote = path.endsWith("/quote");
    const trade = path.endsWith("/trade");
    const inputAmount = init?.body ? JSON.parse(String(init.body)).input_amount : undefined;
    if (trade) {
      await new Promise<void>(resolve => { pendingTrade.current = resolve; });
      pendingTrade.current = null;
      marketVersion.current++;
      return Response.json({ ok: true, data: { trade: { trade_id: `committed-${marketVersion.current}` } } });
    }
    await new Promise(resolve => setTimeout(resolve,
      quote ? inputAmount === "2" ? 600 : 10 : slowReads.current && version === 0 ? 600 : 10));
    if (quote) return Response.json({ ok: true, data: { side: "buy", input_amount: inputAmount,
      min_output_amount: "1", slippage_bps: 300 } });
    if (failRefresh.current && version > 0) return Response.json({ ok: false, error: "fixture_refresh_failed" }, { status: 400 });
    const data = path.includes("/candles?") ? {
      schema_version: 1, status: "bancor_candles", market_id: "fixture", market_version: version,
      market_created_at_unix: 0, price_kind: "pool_marginal_usd_per_aic", interval_seconds: 3600,
      start_time_unix: 0, end_time_unix: 3600, price_scale: 1000000000000, price_decimal_places: 12, candles: [],
    } : path.includes("/account?") ? { page: 1, aic_balance_units: version ? "900" : "1000", usd_balance_units: "10000000000" }
      : { version, trades: [] };
    return Response.json({ ok: true, data });
  };
  const runtime = useBancorRuntime({ apiFetch, t: (_zh, en) => en, cacheScope: "browser-mutation-fixture" });
  const prepare = () => { void runtime.preview("buy", "1", 300, true); };
  const reads = () => {
    slowReads.current = true;
    void runtime.fetchMarket();
    void runtime.fetchAccount();
    void runtime.fetchMarketTrades();
    void runtime.fetchCandles();
  };
  return <main className="mx-auto max-w-3xl space-y-4 p-4">
    <h1 className="text-lg">Post-trade freshness</h1>
    <div className="flex flex-wrap gap-2">
      <button className="theme-btn-secondary rounded-md border px-3 py-2" onClick={prepare}>Prepare quote</button>
      <button className="theme-btn-secondary rounded-md border px-3 py-2" onClick={reads}>Slow observations</button>
      <button className="theme-btn-secondary rounded-md border px-3 py-2" onClick={() => void runtime.trade()}>Complete mock trade</button>
      <button className="theme-btn-secondary rounded-md border px-3 py-2" onClick={() => pendingTrade.current?.()}>Release trade</button>
      <button className="theme-btn-secondary rounded-md border px-3 py-2" onClick={() => void runtime.preview("buy", "2", 300, true)}>Late preview</button>
      <button className="theme-btn-secondary rounded-md border px-3 py-2" onClick={() => { failRefresh.current = true; }}>Fail refresh</button>
    </div>
    <dl className="grid grid-cols-2 gap-2 text-sm">
      <dt>Market version</dt><dd data-testid="market">{runtime.market?.version ?? "none"}</dd>
      <dt>Balance</dt><dd data-testid="balance">{runtime.account?.aic_balance_units ?? "none"}</dd>
      <dt>Candle version</dt><dd data-testid="candles">{runtime.candles?.market_version ?? "none"}</dd>
      <dt>Quote</dt><dd data-testid="quote">{runtime.quote?.input_amount ?? "none"}</dd>
      <dt>Last trade</dt><dd data-testid="trade">{runtime.lastTrade?.trade.trade_id ?? "none"}</dd>
      <dt>Loading</dt><dd data-testid="loading">{String(runtime.marketLoading || runtime.accountLoading || runtime.candlesLoading || runtime.marketTradesLoading || runtime.tradeLoading || runtime.quoteLoading)}</dd>
    </dl>
    <output className="block" data-testid="message">{runtime.message ?? ""}</output>
    <output className="block" data-testid="error">{runtime.error ?? ""}</output>
  </main>;
}

createRoot(document.getElementById("root")!).render(<Fixture />);
