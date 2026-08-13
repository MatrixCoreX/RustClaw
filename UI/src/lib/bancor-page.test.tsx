import assert from "node:assert/strict";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";

import {
  BANCOR_CANDLE_INTERVALS,
  BancorPage,
  BancorQuoteDialog,
  BancorSwapTradePanel,
  CandleChart,
  bindBancorWheelZoom,
  balanceValueSizeClass,
  calculateBancorCandleBodyWidth,
  calculateBancorChartGeometry,
  calculateBancorDefaultVisibleCount,
  calculateBancorPointerCandleIndex,
  calculateBancorPriceDomain,
  calculateBancorVisibleWindow,
  calculateBancorZoomViewport,
  isBancorCandleOpen,
  resolveBancorCandlePalette,
  resolveBancorCandleVisualState,
  scaleBancorPriceDomain,
} from "../components/BancorPage";
import type { useBancorRuntime } from "../hooks/useBancorRuntime";

test("BANCOR page presents the forced-liquidity market and shows the 100 million POINT pool", () => {
  const runtime = {
    market: {
      schema_version: 1 as const,
      status: "open" as const,
      market_id: "point-usd-v1",
      point_symbol: "POINT" as const,
      usd_symbol: "USD" as const,
      point_scale: 10000 as const,
      usd_scale: 10000 as const,
      point_reserve_units: "1000000000000",
      point_reserve: "100000000.0000",
      usd_reserve_units: "100000000",
      usd_reserve: "10000.0000",
      marginal_price_usd_per_point: "0.00010000",
      fee_bps: 50,
      fee_totals: {
        point_fee_units: "12500",
        point_fee_amount: "1.2500",
        usd_fee_units: "5000",
        usd_fee_amount: "0.5000",
        updated_at_unix: 1_700_000_000,
      },
      version: 1,
      updated_at_unix: 1_700_000_000,
    },
    candles: {
      schema_version: 1 as const,
      status: "bancor_candles",
      market_id: "point-usd-v1",
      interval_seconds: 3_600,
      start_time_unix: 1_799_996_400,
      end_time_unix: 1_700_003_600,
      price_scale: 1000000000000 as const,
      price_decimal_places: 12,
      candles: [{
        bucket_start_unix: 1_699_996_400,
        bucket_end_unix: 1_700_000_000,
        open: "0.00010000",
        high: "0.00010010",
        low: "0.00009990",
        close: "0.00010005",
        point_volume_units: "10000000",
        point_volume: "1000.0000",
        usd_volume_units: "999",
        usd_volume: "0.0999",
        trade_count: 1,
        has_trades: true,
      }, {
        bucket_start_unix: 1_700_000_000,
        bucket_end_unix: 1_700_003_600,
        open: "0.00010005",
        high: "0.00010008",
        low: "0.00009995",
        close: "0.00010000",
        point_volume_units: "5000000",
        point_volume: "500.0000",
        usd_volume_units: "500",
        usd_volume: "0.0500",
        trade_count: 1,
        has_trades: true,
      }],
    },
    account: null,
    marketTrades: {
      schema_version: 1 as const,
      status: "bancor_market_trades",
      market_id: "point-usd-v1",
      limit: 100 as const,
      trades: [{
        trade_id: "trade-market-1",
        quote_id: "quote-market-1",
        market_id: "point-usd-v1",
        device_pubkey_compact: "ePsnT8z2UzBzD9aB25B6EeqjKmBossaCCkxdoDQXLp5C",
        side: "sell" as const,
        input_asset: "POINT" as const,
        input_units: "3360000",
        input_amount: "336.0000",
        fee_units: "16800",
        fee_amount: "1.6800",
        output_asset: "USD" as const,
        output_units: "334",
        output_amount: "0.0334",
        market_version: 228,
        created_at_unix: 1_700_000_000,
      }],
    },
    quote: null,
    lastTrade: null,
    marketLoading: false,
    candlesLoading: false,
    candlesOlderLoading: false,
    candlesHasOlder: true,
    candlesError: null,
    candleIntervalSeconds: 3_600,
    accountLoading: false,
    marketTradesLoading: false,
    marketTradesError: null,
    quoteLoading: false,
    tradeLoading: false,
    error: null,
    message: null,
    fetchMarket: async () => null,
    fetchCandles: async () => null,
    changeCandleInterval: async () => null,
    loadOlderCandles: async () => null,
    fetchAccount: async () => null,
    fetchMarketTrades: async () => null,
    preview: async () => null,
    trade: async () => null,
    clearQuote: () => undefined,
  } as unknown as ReturnType<typeof useBancorRuntime>;
  const html = renderToStaticMarkup(
    <BancorPage
      t={(zh) => zh}
      runtime={runtime}
      formatUnixDateTime={(value) => String(value ?? "")}
      nniReady
    />,
  );
  assert.match(html, /100000000\.0000 POINT/);
  assert.match(html, /10000\.0000 USD/);
  assert.match(html, /BANCOR储备曲线市场/);
  assert.equal((html.match(/theme-shadow-card/g) ?? []).length, 6);
  assert.doesNotMatch(html, /theme-card border/);
  assert.match(html, /强制流动性算法/);
  assert.doesNotMatch(html, /<h1[^>]*>BANCOR<\/h1>/);
  assert.doesNotMatch(html, /内部 USD/);
  assert.match(html, /浏览器不会接触私钥/);
  assert.match(html, /ePsnT8z2UzBzD9aB25B6EeqjKmBossaCCkxdoDQXLp5C/);
  assert.match(html, /切换为原始十六进制公钥/);
  assert.match(html, /BANCOR 储备曲线公式/);
  assert.match(html, /role="math"/);
  assert.match(html, /有效支付量等于支付量减去手续费/);
  assert.match(html, /到账量等于有效支付量乘以输出储备/);
  assert.match(html, /扣除手续费后的有效支付量/);
  assert.match(html, /输入资产的市场储备/);
  assert.match(html, /输出资产的市场储备/);
  assert.match(html, /向下取整后的实际到账量/);
  assert.match(html, /⌊/);
  assert.match(html, /买入 POINT：输入储备是 USD，输出储备是 POINT/);
  assert.match(html, /卖出 POINT：输入储备是 POINT，输出储备是 USD/);
  assert.match(html, /均保留 4 位小数/);
  assert.doesNotMatch(html, /市场状态/);
  assert.match(html, /交易手续费/);
  assert.match(html, /买入从 USD 扣除，卖出从 POINT 扣除/);
  assert.match(html, /mt-4 grid gap-2 sm:grid-cols-2 xl:grid-cols-4/);
  assert.match(html, /rounded-xl border border-white\/8 bg-white\/\[0\.025\] px-3 py-2\.5/);
  assert.doesNotMatch(html, /grid gap-4 sm:grid-cols-2 xl:grid-cols-4/);
  assert.match(html, /当前手续费[：:]0\.50%/);
  assert.match(html, /累计手续费/);
  assert.match(html, /1\.2500 POINT/);
  assert.match(html, /0\.5000 USD/);
  for (const amount of ["100000000.0000 POINT", "10000.0000 USD", "1.2500 POINT", "0.5000 USD"]) {
    assert.match(
      html,
      new RegExp(`class="mt-1 break-all text-sm font-semibold text-white sm:text-base">${amount.replace(".", "\\.")}</p>`),
    );
  }
  assert.match(html, /0\.5000 USD<\/p><p class="mt-0\.5 text-\[11px\][^>]*>按支付资产分别累计/);
  assert.match(html, /实际成交均价 K 线/);
  assert.match(html, /池内即时边际价/);
  assert.doesNotMatch(html, /当前 K 线价格摘要/);
  assert.match(html, /aria-label="池内即时边际价"/);
  const livePriceIndex = html.indexOf("池内即时边际价");
  const headerPriceIndex = html.indexOf("0.00010000 USD", livePriceIndex);
  assert.ok(livePriceIndex > html.indexOf("实际成交均价 K 线"));
  assert.ok(headerPriceIndex > livePriceIndex);
  assert.ok(html.indexOf("每 15 秒自动刷新") > headerPriceIndex);
  assert.doesNotMatch(html, /可见最高价/);
  assert.doesNotMatch(html, /可见最低价/);
  assert.match(html, /data-bancor-chart-layer="live-price-line"/);
  assert.match(html, /data-bancor-chart-layer="live-price-label"/);
  assert.match(html, /data-bancor-chart-layer="visible-price-extremes"/);
  assert.doesNotMatch(html, /最后一根未收盘|data-bancor-current-candle/);
  assert.equal((html.match(/data-bancor-candle-state="closed"/g) ?? []).length, 2);
  assert.match(html, />H 0\.00010010<\/text>/);
  assert.match(html, />L 0\.00009990<\/text>/);
  assert.match(html, /<h2[^>]*>交易<\/h2>/);
  assert.match(html, /我的余额/);
  assert.match(html, /mt-2 grid min-w-0 gap-2 sm:grid-cols-2/);
  assert.match(html, /group min-w-0 max-w-full overflow-hidden/);
  assert.match(html, /点击填入全部 POINT 余额/);
  assert.match(html, /点击填入全部 USD 余额/);
  assert.ok(html.indexOf("我的余额") > html.indexOf("<h2 class=\"text-lg font-semibold text-white\">交易</h2>"));
  assert.doesNotMatch(html, /没有成交的时间窗口沿用上一收盘价/);
  assert.match(html, /grid gap-5 lg:grid-cols-\[minmax\(0,2fr\)_minmax\(20rem,1fr\)\] lg:items-stretch/);
  assert.match(html, /theme-shadow-card scroll-mt-4 p-4 sm:p-5/);
  assert.ok(
    html.indexOf("<h2 class=\"text-lg font-semibold text-white\">实际成交均价 K 线</h2>")
      < html.indexOf("<h2 class=\"text-lg font-semibold text-white\">交易</h2>"),
  );
  assert.match(html, /实际成交均价 K 线.*每 15 秒自动刷新/);
  assert.doesNotMatch(html, /价格来自真实成交/);
  assert.match(html, /立即刷新 K 线/);
  assert.match(html, /aria-label="交易模式"/);
  assert.match(html, /id="bancor-trade-panel"/);
  assert.match(html, /id="bancor-standard-input-amount"/);
  assert.match(html, /aria-label="打开交易面板"/);
  assert.match(html, /aria-controls="bancor-trade-panel"/);
  assert.match(html, /aria-label="快速调整支付数量"/);
  assert.match(html, /aria-label="将当前数量减少 25%"/);
  assert.match(html, /aria-label="将当前数量减少 50%"/);
  assert.match(html, /aria-label="减少 0\.0001"/);
  assert.match(html, /aria-label="增加 0\.0001"/);
  assert.match(html, /aria-pressed="true"[^>]*>标准<\/button>/);
  assert.match(html, /aria-pressed="false"[^>]*>SWAP<\/button>/);
  assert.match(html, /滑点保护与警戒/);
  assert.match(html, /value="0\.50"/);
  assert.match(html, /价格影响超过此值时会标黄警告/);
  assert.match(html, /class="theme-primary-btn mt-3 w-full justify-center"[^>]*>卖出<\/button>/);
  assert.doesNotMatch(html, /红色表示上涨，绿色表示下跌/);
  assert.match(html, /全部实际成交均价 K 线已显示/);
  assert.match(html, /回到最新/);
  assert.doesNotMatch(html, /MACD|RSI|均线/);
  assert.match(html, /#f87171/);
  assert.match(html, /#34d399/);
  assert.match(html, /<rect[^>]+fill="#f87171"/);
  assert.match(html, /<rect[^>]+fill="#34d399"/);
  assert.match(html, /0\.00010005/);
  assert.match(html, /实际成交均价 K 线/);
  assert.match(html, /加载更早 K 线/);
  assert.match(html, /data-bancor-history-loader="available"/);
  assert.doesNotMatch(html, /先查看报价|读取私人余额|设备：/);
  assert.match(html, /市场成交记录/);
  assert.match(html, /仅展示最近 100 笔全市场成交/);
  assert.match(html, /ePsnT8z2UzBzD9aB25B6EeqjKmBossaCCkxdoDQXLp5C/);
  assert.doesNotMatch(html, /a2c887498554••••••••331016eb/);
  assert.doesNotMatch(html, /a2c887498554407638cbec1d0ccf11264aa1ab7749bd7913fc6753fac72cfbdb/);
  assert.match(html, /grid gap-5 lg:grid-cols-2 lg:items-start/);
  assert.ok(html.indexOf("储备曲线交易公式") > html.indexOf("我的成交记录"));
});

test("BANCOR balance values shrink by content length without losing precision", () => {
  assert.equal(balanceValueSizeClass("12.3400"), "text-base sm:text-lg");
  assert.equal(balanceValueSizeClass("1234567890123.3400"), "text-sm sm:text-base");
  assert.equal(balanceValueSizeClass("12345678901234567890123.3400"), "text-xs sm:text-sm");
});

test("BANCOR swap mode uses stacked pay and estimated-output windows", () => {
  const html = renderToStaticMarkup(
    <BancorSwapTradePanel
      t={(zh) => zh}
      side="sell"
      inputAmount="100.0000"
      inputAsset="POINT"
      inputBalance="125.0000"
      outputAsset="USD"
      outputAmount="0.0099"
      onInputChange={() => undefined}
      onFillBalance={() => undefined}
      onFlip={() => undefined}
    />,
  );

  assert.match(html, /data-bancor-trade-layout="swap"/);
  assert.ok(html.indexOf("支付") < html.indexOf("预计收到"));
  assert.match(html, /100\.0000/);
  assert.match(html, /0\.0099/);
  assert.match(html, /切换为 USD 支付/);
  assert.match(html, /最终到账以服务端签名报价为准/);
  assert.match(html, /aria-label="快速调整支付数量"/);
  assert.match(html, />−25%<\/button>/);
  assert.match(html, />−50%<\/button>/);
});

test("BANCOR candlesticks follow Chinese and English market color conventions", () => {
  const chinese = resolveBancorCandlePalette((zh) => zh);
  assert.equal(chinese.up.stroke, "#f87171");
  assert.equal(chinese.down.stroke, "#34d399");

  const english = resolveBancorCandlePalette((_zh, en) => en);
  assert.equal(english.up.stroke, "#34d399");
  assert.equal(english.down.stroke, "#f87171");
  assert.equal(chinese.flat.stroke, "var(--theme-chart-neutral)");
  assert.equal(chinese.gap.stroke, "var(--theme-chart-gap)");
});

test("BANCOR candle visual state never reports a flat or empty interval as up", () => {
  const base = {
    bucket_start_unix: 100,
    bucket_end_unix: 160,
    high: "1.0000",
    low: "1.0000",
    point_volume_units: "1",
    point_volume: "0.0001",
    usd_volume_units: "1",
    usd_volume: "0.0001",
  };
  assert.equal(resolveBancorCandleVisualState({ ...base, open: "1", close: "2", trade_count: 2, has_trades: true }), "up");
  assert.equal(resolveBancorCandleVisualState({ ...base, open: "2", close: "1", trade_count: 2, has_trades: true }), "down");
  assert.equal(resolveBancorCandleVisualState({ ...base, open: "1", close: "1", trade_count: 1, has_trades: true }), "flat");
  assert.equal(resolveBancorCandleVisualState({ ...base, open: "1", close: "1", trade_count: 0, has_trades: false }), "gap");
});

test("BANCOR current candle marker follows bucket end time", () => {
  const candle = {
    bucket_start_unix: 100,
    bucket_end_unix: 160,
    open: "1",
    high: "1",
    low: "1",
    close: "1",
    point_volume_units: "1",
    point_volume: "0.0001",
    usd_volume_units: "1",
    usd_volume: "0.0001",
    trade_count: 1,
    has_trades: true,
  };
  assert.equal(isBancorCandleOpen(candle, 99), false);
  assert.equal(isBancorCandleOpen(candle, 100), true);
  assert.equal(isBancorCandleOpen(candle, 159.999), true);
  assert.equal(isBancorCandleOpen(candle, 160), false);
});

test("BANCOR renders the current interval without an open-candle text badge", () => {
  const nowUnix = Math.floor(Date.now() / 1_000);
  const html = renderToStaticMarkup(
    <CandleChart
      candles={[{
        bucket_start_unix: nowUnix - 10,
        bucket_end_unix: nowUnix + 50,
        open: "1.0000",
        high: "1.0000",
        low: "1.0000",
        close: "1.0000",
        point_volume_units: "10",
        point_volume: "0.0010",
        usd_volume_units: "10",
        usd_volume: "0.0010",
        trade_count: 1,
        has_trades: true,
      }]}
      intervalSeconds={60}
      priceDecimalPlaces={4}
      formatUnixDateTime={(value) => String(value ?? "")}
      t={(zh) => zh}
    />,
  );
  assert.match(html, /data-bancor-candle-state="open"/);
  assert.match(html, /data-bancor-current-candle-marker="true"/);
  assert.match(html, /data-bancor-candle-direction="flat"/);
  assert.match(html, /data-bancor-volume-direction="flat"/);
  assert.doesNotMatch(html, /最后一根未收盘|Latest candle is still open|data-bancor-current-candle-state/);
});

test("BANCOR candlesticks distinguish traded flat bars from neutral empty intervals", () => {
  const candles = [{
    bucket_start_unix: 1_800_000_000,
    bucket_end_unix: 1_800_000_060,
    open: "0.00010000",
    high: "0.00010010",
    low: "0.00009990",
    close: "0.00010005",
    point_volume_units: "10000000",
    point_volume: "1000.0000",
    usd_volume_units: "999",
    usd_volume: "0.0999",
    trade_count: 1,
    has_trades: true,
  }, {
    bucket_start_unix: 1_800_000_060,
    bucket_end_unix: 1_800_000_120,
    open: "0.00010005",
    high: "0.00010005",
    low: "0.00010005",
    close: "0.00010005",
    point_volume_units: "0",
    point_volume: "0.0000",
    usd_volume_units: "0",
    usd_volume: "0.0000",
    trade_count: 0,
    has_trades: false,
  }];
  const minuteHtml = renderToStaticMarkup(
    <CandleChart
      candles={candles}
      intervalSeconds={60}
      priceDecimalPlaces={12}
      formatUnixDateTime={(value) => String(value ?? "")}
      t={(zh) => zh}
    />,
  );
  const longerHtml = renderToStaticMarkup(
    <CandleChart
      candles={candles}
      intervalSeconds={300}
      priceDecimalPlaces={12}
      formatUnixDateTime={(value) => String(value ?? "")}
      t={(zh) => zh}
    />,
  );

  assert.match(minuteHtml, /data-bancor-chart-layer="one-minute-close-line"/);
  assert.match(minuteHtml, /<polyline[^>]+stroke="var\(--theme-chart-close-line\)"/);
  assert.match(minuteHtml, /aria-label="最大化 K 线区域"/);
  assert.match(minuteHtml, /aria-label="打开交易面板"/);
  assert.match(minuteHtml, /aria-controls="bancor-trade-panel"/);
  assert.equal((minuteHtml.match(/data-bancor-candle-body="true"/g) ?? []).length, 1);
  assert.equal((minuteHtml.match(/data-bancor-candle-gap="true"/g) ?? []).length, 1);
  assert.doesNotMatch(longerHtml, /one-minute-close-line/);
  assert.equal((longerHtml.match(/data-bancor-candle-body="true"/g) ?? []).length, 1);
  assert.equal((longerHtml.match(/data-bancor-candle-gap="true"/g) ?? []).length, 1);
  assert.match(longerHtml, /data-bancor-candle-direction="gap"/);
  assert.match(longerHtml, /data-bancor-tap-details="enabled"/);
  assert.match(longerHtml, /clip-path="url\(#bancor-price-plot-/);
  assert.match(longerHtml, /var\(--theme-chart-label\)/);
});

test("BANCOR quote review and final confirmation use a centered modal", () => {
  const html = renderToStaticMarkup(
    <BancorQuoteDialog
      t={(zh) => zh}
      quote={{
        schema_version: 1,
        status: "quoted",
        side: "sell",
        input_asset: "POINT",
        input_units: "100000",
        input_amount: "10.0000",
        fee_asset: "POINT",
        fee_units: "500",
        fee_amount: "0.0500",
        curve_input_units: "99500",
        curve_input_amount: "9.9500",
        output_asset: "USD",
        output_units: "10",
        output_amount: "0.0010",
        price_impact_bps: 12,
        fee_bps: 50,
        market_id: "point-usd-v1",
        market_version: 3,
        slippage_bps: 50,
        min_output_units: "9",
        min_output_amount: "0.0009",
      }}
      tradeLoading={false}
      tradeError={null}
      onClose={() => undefined}
      onConfirm={() => undefined}
    />,
  );

  assert.match(html, /fixed inset-0[^>]*items-center justify-center/);
  assert.match(html, /role="dialog"/);
  assert.match(html, /aria-modal="true"/);
  assert.match(html, /查看报价并确认交易/);
  assert.match(html, /10\.0000 POINT/);
  assert.match(html, /0\.0010 USD/);
  assert.match(html, /0\.0500 POINT/);
  assert.match(html, /bancor-sign-trade-btn/);
  assert.match(html, /确认签名交易/);
  assert.match(html, /返回修改/);
  assert.doesNotMatch(html, /已超过你设置的/);
});

test("BANCOR quote modal warns but still permits confirmation when price impact exceeds slippage", () => {
  const html = renderToStaticMarkup(
    <BancorQuoteDialog
      t={(zh) => zh}
      quote={{
        schema_version: 1,
        status: "quoted",
        side: "buy",
        input_asset: "USD",
        input_units: "50000000",
        input_amount: "5000.0000",
        fee_asset: "USD",
        fee_units: "250000",
        fee_amount: "25.0000",
        curve_input_units: "49750000",
        curve_input_amount: "4975.0000",
        output_asset: "POINT",
        output_units: "332220367278",
        output_amount: "33222036.7278",
        price_impact_bps: 3353,
        fee_bps: 50,
        market_id: "point-usd-v1",
        market_version: 3,
        slippage_bps: 50,
        min_output_units: "330559265441",
        min_output_amount: "33055926.5441",
      }}
      tradeLoading={false}
      tradeError={null}
      onClose={() => undefined}
      onConfirm={() => undefined}
    />,
  );

  assert.match(html, /价格影响 33\.53% 已超过你设置的 0\.50% 滑点警戒值/);
  assert.match(html, /确认后仍可继续/);
  assert.match(html, /我已了解风险，继续签名/);
  assert.match(html, /role="alert"/);
});

test("BANCOR candlestick periods include weekly and yearly views", () => {
  assert.deepEqual(
    BANCOR_CANDLE_INTERVALS.slice(-2),
    [
      { seconds: 604_800, zh: "1周", en: "1W" },
      { seconds: 31_536_000, zh: "1年", en: "1Y" },
    ],
  );
});

test("BANCOR candlestick price domain follows the visible range instead of flattening small moves", () => {
  const domain = calculateBancorPriceDomain([
    { high: 0.0001001, low: 0.0000999 },
    { high: 0.00010008, low: 0.00009995 },
  ]);
  assert.ok(domain.high > 0.0001001);
  assert.ok(domain.low < 0.0000999);
  assert.ok(domain.high - domain.low < 0.000001);

  const flat = calculateBancorPriceDomain([{ high: 0.0001, low: 0.0001 }]);
  assert.ok(flat.high > flat.low);
  assert.ok(flat.low >= 0);

  const zoomed = scaleBancorPriceDomain(domain, 4);
  assert.ok(zoomed.high - zoomed.low < domain.high - domain.low);
  const zoomedOut = scaleBancorPriceDomain(domain, 0.5);
  assert.ok(zoomedOut.high - zoomedOut.low > domain.high - domain.low);
});

test("BANCOR candlestick viewport pans from the latest bars toward history", () => {
  assert.equal(calculateBancorDefaultVisibleCount(5), 5);
  assert.equal(calculateBancorDefaultVisibleCount(6), 6);
  assert.equal(calculateBancorDefaultVisibleCount(8), 6);
  assert.equal(calculateBancorDefaultVisibleCount(25), 19);
  assert.equal(calculateBancorDefaultVisibleCount(27), 21);
  assert.equal(calculateBancorDefaultVisibleCount(99), 93);
  assert.equal(calculateBancorDefaultVisibleCount(100), 94);
  assert.equal(calculateBancorDefaultVisibleCount(101), 95);
  assert.equal(calculateBancorDefaultVisibleCount(106), 100);
  assert.equal(calculateBancorDefaultVisibleCount(107), 100);
  assert.equal(calculateBancorDefaultVisibleCount(300), 100);
  assert.equal(calculateBancorDefaultVisibleCount(300, 320), 29);
  assert.equal(calculateBancorDefaultVisibleCount(300, 480), 56);
  assert.deepEqual(
    calculateBancorVisibleWindow(27, calculateBancorDefaultVisibleCount(27), 0),
    { start: 6, end: 27, maxOffset: 6, offset: 0 },
  );
  assert.deepEqual(calculateBancorVisibleWindow(100, 30, 0), {
    start: 70,
    end: 100,
    maxOffset: 70,
    offset: 0,
  });
  assert.deepEqual(calculateBancorVisibleWindow(100, 30, 12), {
    start: 58,
    end: 88,
    maxOffset: 70,
    offset: 12,
  });
  assert.deepEqual(calculateBancorVisibleWindow(100, 30, 999), {
    start: 0,
    end: 30,
    maxOffset: 70,
    offset: 70,
  });
});

test("BANCOR tap detail index clamps to the drawable plot", () => {
  assert.equal(calculateBancorPointerCandleIndex({
    pointerX: 18,
    plotLeft: 18,
    plotRight: 196,
    candleCount: 29,
  }), 0);
  assert.equal(calculateBancorPointerCandleIndex({
    pointerX: 196,
    plotLeft: 18,
    plotRight: 196,
    candleCount: 29,
  }), 28);
  assert.equal(calculateBancorPointerCandleIndex({
    pointerX: 17,
    plotLeft: 18,
    plotRight: 196,
    candleCount: 29,
  }), null);
  assert.equal(calculateBancorPointerCandleIndex({
    pointerX: 80,
    plotLeft: 18,
    plotRight: 196,
    candleCount: 0,
  }), null);
});

test("BANCOR wheel zoom keeps the pointed candle anchored when history allows it", () => {
  assert.deepEqual(calculateBancorZoomViewport({
    total: 300,
    visible: 100,
    offsetFromLatest: 80,
    nextVisible: 60,
    anchorRatio: 0.25,
  }), {
    visible: 60,
    offsetFromLatest: 110,
  });
  assert.deepEqual(calculateBancorZoomViewport({
    total: 300,
    visible: 60,
    offsetFromLatest: 110,
    nextVisible: 100,
    anchorRatio: 0.25,
  }), {
    visible: 100,
    offsetFromLatest: 80,
  });
  assert.deepEqual(calculateBancorZoomViewport({
    total: 30,
    visible: 20,
    offsetFromLatest: 0,
    nextVisible: 28,
    anchorRatio: 1,
  }), {
    visible: 28,
    offsetFromLatest: 0,
  });
});

test("BANCOR wheel zoom uses a non-passive listener and removes the same listener", () => {
  let registeredListener: EventListener | null = null;
  let registeredOptions: AddEventListenerOptions | undefined;
  let removedListener: EventListener | null = null;
  const target = {
    addEventListener(type: "wheel", listener: EventListener, options?: AddEventListenerOptions) {
      assert.equal(type, "wheel");
      registeredListener = listener;
      registeredOptions = options;
    },
    removeEventListener(type: "wheel", listener: EventListener) {
      assert.equal(type, "wheel");
      removedListener = listener;
    },
  };

  const unbind = bindBancorWheelZoom(target, () => undefined);

  assert.ok(registeredListener);
  assert.equal(registeredOptions?.passive, false);
  unbind();
  assert.equal(removedListener, registeredListener);
});

test("BANCOR candlestick bodies stay close without becoming excessively wide", () => {
  assert.equal(calculateBancorCandleBodyWidth(1), 1);
  assert.ok(Math.abs(calculateBancorCandleBodyWidth(5) - 3.9) < 0.000000001);
  assert.ok(Math.abs(calculateBancorCandleBodyWidth(30) - 23.4) < 0.000000001);
  assert.equal(calculateBancorCandleBodyWidth(100), 72);
});

test("BANCOR candlestick geometry keeps text at the real half-width scale", () => {
  assert.deepEqual(calculateBancorChartGeometry(600), {
    width: 600,
    plotRight: 476,
    priceAxisX: 490,
  });
  assert.deepEqual(calculateBancorChartGeometry(280), {
    width: 320,
    plotRight: 196,
    priceAxisX: 210,
  });
});
