import assert from "node:assert/strict";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";

import { BancorPriceChangePage } from "../components/BancorPriceChangePage";
import { resolveBancorMarketDirectionColor } from "./bancor-market-colors";
import { calculateBancorPriceChange } from "./bancor-price-change";
import type { NniBancorMarketResponse } from "../types/api";

const market: NniBancorMarketResponse = {
  schema_version: 1,
  status: "open",
  market_id: "aic-usd-v1",
  aic_symbol: "AIC",
  usd_symbol: "USD",
  aic_scale: 100000000,
  usd_scale: 100000000,
  aic_reserve_units: "10000000000000000",
  aic_reserve: "100000000.00000000",
  usd_reserve_units: "1000000000000",
  usd_reserve: "10000.00000000",
  marginal_price_usd_per_aic: "0.00010000",
  daily_marginal_price: {
    price_kind: "pool_marginal_usd_per_aic",
    timezone: "UTC",
    day_start_unix: 1_800_000_000,
    open_usd_per_aic: "0.00010000",
    high_usd_per_aic: "0.00010000",
    low_usd_per_aic: "0.00010000",
    change_percent: "0.00",
    trade_count: 0,
  },
  fee_bps: 50,
  version: 8,
  updated_at_unix: 1_800_000_000,
};

test("BANCOR buy price-change projection matches the server integer quote and reserve rules", () => {
  const result = calculateBancorPriceChange({ side: "buy", inputAmount: "1.00000000", market });
  assert.equal(result.ok, true);
  if (!result.ok) return;
  assert.deepEqual(result.projection, {
    side: "buy",
    inputAsset: "USD",
    outputAsset: "AIC",
    inputAmount: "1.00000000",
    feeAmount: "0.00500000",
    effectiveInputAmount: "0.99500000",
    outputAmount: "9949.01007349",
    aicReserveAfter: "99990050.98992651",
    usdReserveAfter: "10000.99500000",
    currentMarginalPrice: "0.00010000",
    marginalPriceAfter: "0.00010001",
    marginalPriceChangePercent: "+0.0199%",
  });
});

test("BANCOR sell price-change projection keeps fees outside the pool", () => {
  const result = calculateBancorPriceChange({ side: "sell", inputAmount: "100.00000000", market });
  assert.equal(result.ok, true);
  if (!result.ok) return;
  assert.equal(result.projection.feeAmount, "0.50000000");
  assert.equal(result.projection.effectiveInputAmount, "99.50000000");
  assert.equal(result.projection.outputAmount, "0.00994999");
  assert.equal(result.projection.aicReserveAfter, "100000099.50000000");
  assert.equal(result.projection.usdReserveAfter, "9999.99005001");
  assert.equal(result.projection.marginalPriceAfter, "0.00009999");
  assert.equal(result.projection.marginalPriceChangePercent, "-0.0002%");
});

test("BANCOR price-change calculator rejects malformed, zero-output, and missing-market inputs", () => {
  assert.deepEqual(
    calculateBancorPriceChange({ side: "buy", inputAmount: "1.000000001", market }),
    { ok: false, error: "amount_invalid" },
  );
  assert.deepEqual(
    calculateBancorPriceChange({ side: "buy", inputAmount: "0.00000001", market }),
    { ok: false, error: "amount_too_small" },
  );
  assert.deepEqual(
    calculateBancorPriceChange({ side: "sell", inputAmount: "1.00000000", market: null }),
    { ok: false, error: "market_invalid" },
  );
  assert.deepEqual(
    calculateBancorPriceChange({
      side: "sell",
      inputAmount: "10000000.00000000",
      market: { ...market, fee_bps: 0, aic_reserve_units: "9223372036854775807" },
    }),
    { ok: false, error: "market_capacity_exceeded" },
  );
});

test("BANCOR price-change colors follow Chinese and international market conventions", () => {
  assert.equal(resolveBancorMarketDirectionColor("up", (zh) => zh), "#f87171");
  assert.equal(resolveBancorMarketDirectionColor("down", (zh) => zh), "#34d399");
  assert.equal(resolveBancorMarketDirectionColor("up", (_zh, en) => en), "#34d399");
  assert.equal(resolveBancorMarketDirectionColor("down", (_zh, en) => en), "#f87171");
});

test("BANCOR price-change page exposes two local-only beginner calculators", () => {
  const html = renderToStaticMarkup(
    <BancorPriceChangePage market={market} onBack={() => undefined} t={(zh) => zh} />,
  );
  assert.match(html, /data-bancor-view="price-change-calculator"/);
  assert.match(html, /仅本地计算/);
  assert.match(html, /不会签名、提交或成交/);
  assert.match(html, /data-bancor-calculator-side="buy"/);
  assert.match(html, /data-bancor-calculator-side="sell"/);
  assert.match(html, /aria-label="计划支付 USD"/);
  assert.match(html, /aria-label="计划支付 AIC"/);
  assert.equal((html.match(/data-bancor-price-change-result="ready"/g) ?? []).length, 2);
  assert.match(html, /成交后 AIC 储备/);
  assert.match(html, /成交后 USD 储备/);
  assert.match(html, /池内边际价变化/);
  assert.match(html, /data-bancor-price-change-formula="true"/);
  assert.match(html, /价格变化计算公式/);
  assert.match(html, /买入 AIC（支付 USD）/);
  assert.match(html, /卖出 AIC（收到 USD）/);
  assert.match(html, /向上取整的手续费/);
  assert.match(html, /向下取整的预计到账/);
  assert.match(html, /ΔP/);
  assert.match(html, /成交前 \/ 成交后池内边际价/);
  assert.match(html, /返回市场/);
});

test("BANCOR English calculator renders green gains and red declines", () => {
  const html = renderToStaticMarkup(
    <BancorPriceChangePage market={market} onBack={() => undefined} t={(_zh, en) => en} />,
  );
  assert.match(html, /color:#34d399[^>]*>.*data-nni-decimal-amount="\+0\.0199%"/);
  assert.match(html, /color:#f87171[^>]*>.*data-nni-decimal-amount="-0\.0002%"/);
});
