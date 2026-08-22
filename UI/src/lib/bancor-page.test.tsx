import assert from "node:assert/strict";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";

import {
  BANCOR_CANDLE_INTERVALS,
  BancorPage,
  BancorQuoteDialog,
  BancorSwapTradePanel,
  CandleChart,
  bancorMarketWorkspaceClass,
  bindBancorWheelZoom,
  balanceValueSizeClass,
  buildBancorAssetAccountOptions,
  calculateBancorCandleBodyWidth,
  calculateBancorChartGeometry,
  calculateBancorDefaultVisibleCount,
  calculateBancorPointerCandleIndex,
  calculateBancorPriceDomain,
  calculateBancorVisibleWindow,
  calculateBancorZoomViewport,
  isBancorCandleOpen,
  paginateBancorTrades,
  persistBancorSlippagePercent,
  persistBancorTradeLayout,
  persistBancorTradeSide,
  readBancorSlippagePercent,
  readBancorTradeLayout,
  readBancorTradeSide,
  formatBancorDayChangePercent,
  formatBancorAssetAccountOption,
  resolveBancorCandlePalette,
  resolveBancorDayChangeColor,
  resolveBancorCandleVisualState,
  resolveBancorTradeColor,
  scaleBancorPriceDomain,
} from "../components/BancorPage";
import type { useBancorRuntime } from "../hooks/useBancorRuntime";

test("BANCOR page presents the forced-liquidity market and shows the 100 million AIC pool", () => {
  const runtime = {
    market: {
      schema_version: 1 as const,
      status: "open" as const,
      market_id: "aic-usd-v1",
      aic_symbol: "AIC" as const,
      usd_symbol: "USD" as const,
      aic_scale: 100000000 as const,
      usd_scale: 100000000 as const,
      aic_reserve_units: "10000000000000000",
      aic_reserve: "100000000.00000000",
      usd_reserve_units: "1000000000000",
      usd_reserve: "10000.00000000",
      marginal_price_usd_per_aic: "0.00010000",
      daily_marginal_price: {
        price_kind: "pool_marginal_usd_per_aic" as const,
        timezone: "UTC" as const,
        day_start_unix: 1_699_920_000,
        open_usd_per_aic: "0.00009950",
        high_usd_per_aic: "0.00010100",
        low_usd_per_aic: "0.00009900",
        change_percent: "0.50",
        trade_count: 17,
      },
      min_trade_usd: "0.00000200",
      min_trade_usd_units: "200",
      min_trade_aic: "0.00010052",
      min_trade_aic_units: "10052",
      minimum_fee_units: "1",
      minimum_output_units: "1",
      fee_bps: 50,
      version: 1,
      updated_at_unix: 1_700_000_000,
    },
    candles: {
      schema_version: 1 as const,
      status: "bancor_candles",
      market_id: "aic-usd-v1",
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
        aic_volume_units: "100000000000",
        aic_volume: "1000.00000000",
        usd_volume_units: "9990000",
        usd_volume: "0.09990000",
        trade_count: 1,
        has_trades: true,
      }, {
        bucket_start_unix: 1_700_000_000,
        bucket_end_unix: 1_700_003_600,
        open: "0.00010005",
        high: "0.00010008",
        low: "0.00009995",
        close: "0.00010000",
        aic_volume_units: "50000000000",
        aic_volume: "500.00000000",
        usd_volume_units: "5000000",
        usd_volume: "0.05000000",
        trade_count: 1,
        has_trades: true,
      }],
    },
    account: {
      schema_version: 1 as const,
      status: "bancor_account",
      device_pubkey: "ePsnT8z2UzBzD9aB25B6EeqjKmBossaCCkxdoDQXLp5C",
      aic_balance_units: "10000000000",
      aic_balance: "100.00000000",
      usd_balance_units: "500000000",
      usd_balance: "5.00000000",
      account_version: 1,
      page: 1,
      per_page: 10,
      total: 1,
      total_pages: 1,
      trades: [{
        trade_id: "trade-account-1",
        quote_id: "quote-account-1",
        market_id: "aic-usd-v1",
        side: "buy" as const,
        input_asset: "USD" as const,
        input_units: "12340000",
        input_amount: "0.12340000",
        fee_units: "61700",
        fee_amount: "0.00061700",
        output_asset: "AIC" as const,
        output_units: "120000000000",
        output_amount: "1200.00000000",
        market_version: 228,
        created_at_unix: 1_700_000_000,
      }],
    },
    marketTrades: {
      schema_version: 1 as const,
      status: "bancor_market_trades",
      market_id: "aic-usd-v1",
      limit: 100 as const,
      trades: Array.from({ length: 11 }, (_, index) => ({
        trade_id: `trade-market-${index + 1}`,
        quote_id: `quote-market-${index + 1}`,
        market_id: "aic-usd-v1",
        asset_owner_pubkey: "5p78kHbL33Rn3JWkTWRE2B9uz6gy4r1KbfAKLNQGE3ovLY8E9M",
        side: "sell" as const,
        input_asset: "AIC" as const,
        input_units: "33600000000",
        input_amount: "336.00000000",
        fee_units: "168000000",
        fee_amount: "1.68000000",
        output_asset: "USD" as const,
        output_units: "3340000",
        output_amount: "0.03340000",
        market_version: 228 + index,
        created_at_unix: 1_700_000_000 + index,
      })),
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
      signingDeviceReady
      assetOwnerReady
      assetOwnerPubkey="5p78kHbL33Rn3JWkTWRE2B9uz6gy4r1KbfAKLNQGE3ovLY8E9M"
      onOpenNni={() => undefined}
    />,
  );
  const signingUnavailableHtml = renderToStaticMarkup(
    <BancorPage
      t={(zh) => zh}
      runtime={runtime}
      formatUnixDateTime={(value) => String(value ?? "")}
      signingDeviceReady={false}
      assetOwnerReady={false}
      assetOwnerPubkey={null}
      onOpenNni={() => undefined}
    />,
  );
  const assetOwnerRequiredHtml = renderToStaticMarkup(
    <BancorPage
      t={(zh) => zh}
      runtime={{
        ...runtime,
        assetOwnerRequired: true,
        error: "请先到 NNI 页面生成并绑定资产账号，然后再进行交易。",
      }}
      formatUnixDateTime={(value) => String(value ?? "")}
      signingDeviceReady
      assetOwnerReady={false}
      assetOwnerPubkey={null}
      onOpenNni={() => undefined}
    />,
  );
  const revokedDeviceHtml = renderToStaticMarkup(
    <BancorPage
      t={(zh) => zh}
      runtime={{
        ...runtime,
        assetOwnerRequired: true,
        assetOwnerAccessErrorCode: "nni_asset_device_not_authorized",
        error: "当前设备的资产绑定已经解除，无法读取余额或进行交易。请前往 NNI 页面重新绑定资产账号。",
      }}
      formatUnixDateTime={(value) => String(value ?? "")}
      signingDeviceReady
      assetOwnerReady={false}
      assetOwnerPubkey={null}
      onOpenNni={() => undefined}
    />,
  );
  const hardwareAccountUnavailableHtml = renderToStaticMarkup(
    <BancorPage
      t={(zh) => zh}
      runtime={{
        ...runtime,
        account: null,
        hardwareAccountAccessUnavailable: true,
      }}
      formatUnixDateTime={(value) => String(value ?? "")}
      signingDeviceReady
      assetOwnerReady
      assetOwnerPubkey="5p78kHbL33Rn3JWkTWRE2B9uz6gy4r1KbfAKLNQGE3ovLY8E9M"
      onOpenNni={() => undefined}
    />,
  );
  assert.match(signingUnavailableHtml, /请选择并准备一种可用的账户签名方式/);
  assert.doesNotMatch(signingUnavailableHtml, /请先在 NNI 页面加入网络/);
  assert.doesNotMatch(signingUnavailableHtml, /合法设备|NNI 网络准入/);
  assert.doesNotMatch(signingUnavailableHtml, /data-bancor-account-selector="true"/);
  assert.match(html, /data-bancor-account-selector="true"/);
  assert.match(html, /交易账户/);
  assert.match(html, /本机绑定账户/);
  const tradingAccountIndex = html.indexOf("交易账户");
  const assetPublicKeyIndex = html.indexOf("5p78kHbL33Rn3JWkTWRE2B9uz6gy4r1KbfAKLNQGE3ovLY8E9M");
  const balancesIndex = html.indexOf("我的余额");
  assert.ok(assetPublicKeyIndex > tradingAccountIndex);
  assert.ok(assetPublicKeyIndex < balancesIndex);
  assert.match(hardwareAccountUnavailableHtml, /data-bancor-hardware-account-unavailable="true"/);
  assert.match(hardwareAccountUnavailableHtml, /仍可选择资产密钥签名完成交易/);
  assert.doesNotMatch(hardwareAccountUnavailableHtml, /合法设备|NNI 网络准入/);
  assert.match(assetOwnerRequiredHtml, /data-bancor-asset-owner-required="true"/);
  assert.match(assetOwnerRequiredHtml, /请先到 NNI 页面生成并绑定资产账号/);
  assert.match(assetOwnerRequiredHtml, /data-bancor-open-nni="asset-owner"/);
  assert.match(assetOwnerRequiredHtml, /前往 NNI 页面/);
  assert.doesNotMatch(assetOwnerRequiredHtml, /nni_asset_owner_required/);
  assert.match(revokedDeviceHtml, /data-bancor-asset-owner-required="true"/);
  assert.match(revokedDeviceHtml, /重新绑定资产账号/);
  assert.match(revokedDeviceHtml, /data-bancor-open-nni="asset-owner"/);
  assert.doesNotMatch(revokedDeviceHtml, /nni_asset_device_not_authorized/);
  assert.match(html, /data-nni-decimal-amount="100000000\.00000000 AIC"[^>]*data-nni-decimal-fraction-size="normal"/);
  assert.match(html, /data-nni-decimal-amount="10000\.00000000 USD"[^>]*data-nni-decimal-fraction-size="normal"/);
  assert.match(html, /BANCOR储备曲线市场/);
  assert.match(html, /获得奖励/);
  assert.match(html, /data-bancor-open-price-change="true"/);
  assert.match(html, /价格变化计算/);
  assert.equal((html.match(/theme-shadow-card/g) ?? []).length, 6);
  assert.doesNotMatch(html, /theme-card border/);
  assert.match(html, /强制流动性算法/);
  assert.doesNotMatch(html, /<h1[^>]*>BANCOR<\/h1>/);
  assert.doesNotMatch(html, /内部 USD/);
  assert.match(html, /临时输入资产私钥自行签名/);
  assert.doesNotMatch(html, /ePsnT8z2UzBzD9aB25B6EeqjKmBossaCCkxdoDQXLp5C/);
  assert.doesNotMatch(html, /资产账号公钥/);
  assert.doesNotMatch(html, /Asset account public key/);
  assert.match(html, /5p78kHbL33Rn3JWkTWRE2B9uz6gy4r1KbfAKLNQGE3ovLY8E9M/);
  assert.doesNotMatch(html, /切换为原始十六进制公钥/);
  assert.match(html, /BANCOR 储备曲线公式/);
  assert.match(html, /role="math"/);
  assert.match(html, /有效支付量等于支付量减去手续费/);
  assert.match(html, /到账量等于有效支付量乘以输出储备/);
  assert.match(html, /扣除手续费后的有效支付量/);
  assert.match(html, /输入资产的市场储备/);
  assert.match(html, /输出资产的市场储备/);
  assert.match(html, /向下取整后的实际到账量/);
  assert.match(html, /⌊/);
  assert.match(html, /买入 AIC：输入储备是 USD，输出储备是 AIC/);
  assert.match(html, /卖出 AIC：输入储备是 AIC，输出储备是 USD/);
  assert.match(html, /均保留 8 位小数/);
  assert.doesNotMatch(html, /市场状态/);
  assert.match(html, /交易手续费/);
  assert.doesNotMatch(html, /每 1 AIC|Per AIC/);
  assert.match(html, /data-bancor-daily-marginal-price="UTC"/);
  assert.match(html, /今日最高/);
  assert.match(html, /data-nni-decimal-amount="0\.00010100 USD"/);
  assert.match(html, /data-nni-decimal-amount="0\.00010100 USD"[^>]*data-nni-decimal-fraction-size="normal"/);
  assert.match(html, /今日最低/);
  assert.match(html, /data-nni-decimal-amount="0\.00009900 USD"/);
  assert.match(html, /日涨跌幅/);
  assert.match(html, /data-nni-decimal-amount="\+0\.50%"/);
  assert.match(html, /买入从 USD 扣除，卖出从 AIC 扣除/);
  assert.match(html, /mt-4 grid gap-2 sm:grid-cols-2 xl:grid-cols-3/);
  assert.match(html, /rounded-xl border border-white\/8 bg-white\/\[0\.025\] px-3 py-2\.5/);
  assert.doesNotMatch(html, /grid gap-4 sm:grid-cols-2 xl:grid-cols-4/);
  assert.match(html, /当前手续费[：:].*data-nni-decimal-amount="0\.50%"/);
  assert.match(html, /data-nni-decimal-amount="0\.50%"[^>]*data-nni-decimal-fraction-size="normal"/);
  assert.doesNotMatch(html, /累计手续费|Cumulative fees|按支付资产分别累计/);
  assert.doesNotMatch(html, /1\.2500 AIC|0\.5000 USD/);
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
  assert.match(html, /title="AIC: 100\n点击填入全部 AIC 余额"/);
  assert.match(html, /title="USD: 5\n点击填入全部 USD 余额"/);
  assert.match(html, /data-bancor-balance-full-value="100"/);
  assert.match(html, /data-bancor-balance-full-value="5"/);
  assert.match(html, /data-nni-decimal-amount="100\.00"[^>]*data-nni-decimal-fraction-size="normal"/);
  assert.match(html, /data-nni-decimal-amount="5\.00"[^>]*data-nni-decimal-fraction-size="normal"/);
  assert.doesNotMatch(html, />点击填入全部 (?:AIC|USD) 余额<\/span>/);
  assert.ok(html.indexOf("我的余额") > html.indexOf("<h2 class=\"text-lg font-semibold text-white\">交易</h2>"));
  assert.doesNotMatch(html, /没有成交的时间窗口沿用上一收盘价/);
  assert.match(html, /data-bancor-market-workspace="true"/);
  assert.match(html, /data-chart-maximized="false"/);
  assert.match(html, /grid gap-5 lg:grid-cols-\[minmax\(0,2fr\)_minmax\(20rem,1fr\)\] lg:items-stretch/);
  assert.match(html, /bancor-market-trade-panel theme-shadow-card scroll-mt-4 p-4 sm:p-5/);
  assert.ok(
    html.indexOf("<h2 class=\"text-lg font-semibold text-white\">实际成交均价 K 线</h2>")
      < html.indexOf("<h2 class=\"text-lg font-semibold text-white\">交易</h2>"),
  );
  assert.match(html, /实际成交均价 K 线.*每 15 秒自动刷新/);
  assert.doesNotMatch(html, /价格来自真实成交/);
  assert.match(html, /立即刷新 K 线/);
  assert.match(html, /aria-label="交易模式"/);
  assert.match(html, /id="bancor-trade-panel"/);
  assert.match(html, /bancor-trade-account/);
  assert.match(html, /bancor-trade-order-panel/);
  assert.match(html, /bancor-trade-risk-panel/);
  assert.match(html, /bancor-trade-action-panel/);
  assert.match(html, /id="bancor-standard-input-amount"/);
  assert.match(html, /aria-label="最大化 K 线与交易区域"/);
  assert.match(html, /aria-controls="bancor-market-workspace"/);
  assert.match(html, /aria-label="快速调整支付数量"/);
  assert.match(html, /aria-label="将当前数量减少 25%"/);
  assert.match(html, /aria-label="将当前数量减少 50%"/);
  assert.match(html, /aria-label="减少 1"/);
  assert.match(html, /aria-label="增加 1"/);
  assert.match(html, /aria-pressed="true"[^>]*>标准<\/button>/);
  assert.match(html, /aria-pressed="false"[^>]*>SWAP<\/button>/);
  assert.match(html, /滑点保护与警戒/);
  assert.match(html, /value="3\.00"/);
  assert.match(html, /价格影响超过此值时会标黄警告/);
  assert.match(html, /class="theme-primary-btn mt-3 w-full justify-center"[^>]*>卖出<\/button>/);
  assert.doesNotMatch(html, /红色表示上涨，绿色表示下跌/);
  assert.doesNotMatch(html, /左右拖动查看历史|点按查看详情|滚轮缩放|全部实际成交均价 K 线已显示/);
  assert.match(html, /回到最新/);
  assert.doesNotMatch(html, /MACD|RSI|均线/);
  assert.match(html, /#f87171/);
  assert.match(html, /#34d399/);
  assert.match(html, /<rect[^>]+fill="#f87171"/);
  assert.match(html, /<rect[^>]+fill="#34d399"/);
  assert.match(html, /0\.00010005/);
  assert.match(html, /实际成交均价 K 线/);
  assert.doesNotMatch(html, /加载更早 K 线|加载后可用左移按钮查看|data-bancor-history-loader/);
  assert.doesNotMatch(html, /先查看报价|读取私人余额|设备：/);
  assert.match(html, /市场成交记录/);
  assert.match(html, /仅展示最近 100 笔全市场成交/);
  assert.doesNotMatch(html, /设备公钥使用紧凑格式|compact device public keys/);
  assert.equal((html.match(/data-bancor-trade-row="market"/g) ?? []).length, 10);
  assert.match(html, /data-bancor-trade-pagination="market"/);
  assert.match(html, /data-bancor-page-size="10"/);
  assert.doesNotMatch(html, /ePsnT8z2UzBzD9aB25B6EeqjKmBossaCCkxdoDQXLp5C/);
  assert.match(html, /5p78kHbL33Rn3JWkTWRE2B9uz6gy4r1KbfAKLNQGE3ovLY8E9M/);
  assert.doesNotMatch(html, /切换为原始十六进制公钥|切换为 Base58 编码公钥/);
  assert.doesNotMatch(html, /a2c887498554••••••••331016eb/);
  assert.doesNotMatch(html, /a2c887498554407638cbec1d0ccf11264aa1ab7749bd7913fc6753fac72cfbdb/);
  assert.match(html, /data-nni-decimal-amount="0\.1234 USD"[^>]*data-nni-decimal-fraction-size="normal"/);
  assert.match(html, />\+1200 AIC<\/span>/);
  assert.match(html, />336 AIC<\/span>/);
  assert.match(html, /data-nni-decimal-amount="\+0\.0334 USD"[^>]*data-nni-decimal-fraction-size="normal"/);
  assert.doesNotMatch(html, /1200\.00000000 AIC|336\.00000000 AIC|0\.03340000 USD/);
  assert.match(html, /grid gap-5 lg:grid-cols-2 lg:items-start/);
  assert.ok(html.indexOf("储备曲线交易公式") > html.indexOf("我的成交记录"));
});

test("BANCOR trade layout persists through the product-neutral storage key", () => {
  const values = new Map<string, string>();
  const storage = {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => values.set(key, value),
  };

  assert.equal(readBancorTradeLayout(storage), "standard");
  persistBancorTradeLayout(storage, "swap");
  assert.equal(readBancorTradeLayout(storage), "swap");
  persistBancorTradeLayout(storage, "standard");
  assert.equal(readBancorTradeLayout(storage), "standard");
  assert.match([...values.keys()][0] ?? "", /^agent-runtime\./);
});

test("BANCOR swap direction and valid slippage survive a refresh", () => {
  const values = new Map<string, string>();
  const storage = {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => values.set(key, value),
  };

  assert.equal(readBancorTradeSide(storage), "sell");
  assert.equal(readBancorSlippagePercent(storage), "3.00");

  persistBancorTradeSide(storage, "buy");
  persistBancorSlippagePercent(storage, "4.25");
  assert.equal(readBancorTradeSide(storage), "buy");
  assert.equal(readBancorSlippagePercent(storage), "4.25");

  persistBancorSlippagePercent(storage, "invalid");
  assert.equal(readBancorSlippagePercent(storage), "4.25");
  assert.ok([...values.keys()].every((key) => key.startsWith("agent-runtime.")));
});

test("BANCOR account options keep the local binding first and leave room for external accounts", () => {
  const localPublicKey = "5p78kHbL33Rn3JWkTWRE2B9uz6gy4r1KbfAKLNQGE3ovLY8E9M";
  const externalPublicKey = "7gY3W3iKnU7Nd4MCY7N9FY4U5ABQG1nB7eVwjvp23uLnzUE5nL";
  const options = buildBancorAssetAccountOptions(localPublicKey, [
    { id: "external:primary", publicKey: externalPublicKey, source: "external", label: "冷钱包" },
    { id: "external:duplicate", publicKey: localPublicKey, source: "external" },
    { id: "external:primary", publicKey: "8x1gP42dC6eABn91QkKTuWJ4AVREkLX2cw7NLo9pRC7Y", source: "external" },
  ]);

  assert.deepEqual(options.map((option) => option.source), ["local_binding", "external"]);
  assert.equal(options[0]?.publicKey, localPublicKey);
  assert.match(formatBancorAssetAccountOption(options[0]!, (zh) => zh), /^本机绑定账户/);
  assert.match(formatBancorAssetAccountOption(options[1]!, (zh) => zh), /^冷钱包/);
  assert.deepEqual(buildBancorAssetAccountOptions(null), []);
});

test("BANCOR market history pagination returns ten rows and clamps page bounds", () => {
  const trades = Array.from({ length: 23 }, (_, index) => index + 1);
  assert.deepEqual(paginateBancorTrades(trades, 2), {
    items: [11, 12, 13, 14, 15, 16, 17, 18, 19, 20],
    page: 2,
    totalPages: 3,
  });
  assert.deepEqual(paginateBancorTrades(trades, 99), {
    items: [21, 22, 23],
    page: 3,
    totalPages: 3,
  });
  assert.deepEqual(paginateBancorTrades([], Number.NaN), {
    items: [],
    page: 1,
    totalPages: 1,
  });
});

test("BANCOR balance values shrink by content length without losing precision", () => {
  assert.equal(balanceValueSizeClass("12.34000000"), "text-base sm:text-lg");
  assert.equal(balanceValueSizeClass("1234567890123.34000000"), "text-sm sm:text-base");
  assert.equal(balanceValueSizeClass("12345678901234567890123.34000000"), "text-xs sm:text-sm");
});

test("BANCOR swap mode uses stacked pay and estimated-output windows", () => {
  const html = renderToStaticMarkup(
    <BancorSwapTradePanel
      t={(zh) => zh}
      side="sell"
      inputAmount="100.00000000"
      inputAsset="AIC"
      inputBalance="125.00000000"
      outputAsset="USD"
      outputAmount="0.00990000"
      minimumInputAmount="0.00010052"
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
  assert.match(html, /最低 0\.00010052 AIC/);
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

test("BANCOR trade records follow localized market color conventions", () => {
  assert.equal(resolveBancorTradeColor("buy", (zh) => zh), "#f87171");
  assert.equal(resolveBancorTradeColor("sell", (zh) => zh), "#34d399");
  assert.equal(resolveBancorTradeColor("buy", (_zh, en) => en), "#34d399");
  assert.equal(resolveBancorTradeColor("sell", (_zh, en) => en), "#f87171");
});

test("BANCOR daily change formats signed percentages and follows localized colors", () => {
  assert.equal(formatBancorDayChangePercent("1.25"), "+1.25%");
  assert.equal(formatBancorDayChangePercent("-0.75"), "-0.75%");
  assert.equal(formatBancorDayChangePercent("-0.00"), "0.00%");
  assert.equal(formatBancorDayChangePercent(undefined), "—");
  assert.equal(resolveBancorDayChangeColor("1.25", (zh) => zh), "#f87171");
  assert.equal(resolveBancorDayChangeColor("-0.75", (zh) => zh), "#34d399");
  assert.equal(resolveBancorDayChangeColor("1.25", (_zh, en) => en), "#34d399");
  assert.equal(resolveBancorDayChangeColor("0.00", (zh) => zh), "var(--theme-chart-neutral)");
});

test("BANCOR candle visual state never reports a flat or empty interval as up", () => {
  const base = {
    bucket_start_unix: 100,
    bucket_end_unix: 160,
    high: "1.00000000",
    low: "1.00000000",
    aic_volume_units: "10000",
    aic_volume: "0.00010000",
    usd_volume_units: "10000",
    usd_volume: "0.00010000",
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
    aic_volume_units: "10000",
    aic_volume: "0.00010000",
    usd_volume_units: "10000",
    usd_volume: "0.00010000",
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
        open: "1.00000000",
        high: "1.00000000",
        low: "1.00000000",
        close: "1.00000000",
        aic_volume_units: "100000",
        aic_volume: "0.00100000",
        usd_volume_units: "100000",
        usd_volume: "0.00100000",
        trade_count: 1,
        has_trades: true,
      }]}
      intervalSeconds={60}
      priceDecimalPlaces={4}
      formatUnixDateTime={(value) => String(value ?? "")}
      maximized={false}
      onMaximizedChange={() => undefined}
      t={(zh) => zh}
    />,
  );
  assert.match(html, /data-bancor-candle-state="open"/);
  assert.match(html, /data-bancor-current-candle-marker="true"/);
  assert.match(html, /data-bancor-candle-direction="flat"/);
  assert.match(html, /data-bancor-volume-direction="flat"/);
  assert.doesNotMatch(html, /最后一根未收盘|Latest candle is still open|data-bancor-current-candle-state/);
});

test("BANCOR keeps focused candle details out of the chart layout", () => {
  const html = renderToStaticMarkup(
    <CandleChart
      candles={[{
        bucket_start_unix: 1_800_000_000,
        bucket_end_unix: 1_800_000_060,
        open: "0.00010000",
        high: "0.00010010",
        low: "0.00009990",
        close: "0.00010005",
        aic_volume_units: "100000000000",
        aic_volume: "1000.00000000",
        usd_volume_units: "9990000",
        usd_volume: "0.09990000",
        trade_count: 1,
        has_trades: true,
      }]}
      intervalSeconds={60}
      priceDecimalPlaces={12}
      formatUnixDateTime={() => "UNIQUE_CANDLE_TIMESTAMP"}
      maximized={false}
      onMaximizedChange={() => undefined}
      t={(zh) => zh}
    />,
  );

  const svgStart = html.indexOf("<svg");
  assert.ok(svgStart > 0);
  assert.doesNotMatch(html.slice(0, svgStart), /UNIQUE_CANDLE_TIMESTAMP|O 0\.00010000|VOL 1000\.00000000/);
  assert.match(html.slice(svgStart), /<title>UNIQUE_CANDLE_TIMESTAMP · O /);
});

test("BANCOR candlesticks distinguish traded flat bars from neutral empty intervals", () => {
  const candles = [{
    bucket_start_unix: 1_800_000_000,
    bucket_end_unix: 1_800_000_060,
    open: "0.00010000",
    high: "0.00010010",
    low: "0.00009990",
    close: "0.00010005",
    aic_volume_units: "100000000000",
    aic_volume: "1000.00000000",
    usd_volume_units: "9990000",
    usd_volume: "0.09990000",
    trade_count: 1,
    has_trades: true,
  }, {
    bucket_start_unix: 1_800_000_060,
    bucket_end_unix: 1_800_000_120,
    open: "0.00010005",
    high: "0.00010005",
    low: "0.00010005",
    close: "0.00010005",
    aic_volume_units: "0",
    aic_volume: "0.00000000",
    usd_volume_units: "0",
    usd_volume: "0.00000000",
    trade_count: 0,
    has_trades: false,
  }];
  const minuteHtml = renderToStaticMarkup(
    <CandleChart
      candles={candles}
      intervalSeconds={60}
      priceDecimalPlaces={12}
      formatUnixDateTime={(value) => String(value ?? "")}
      maximized={false}
      onMaximizedChange={() => undefined}
      t={(zh) => zh}
    />,
  );
  const longerHtml = renderToStaticMarkup(
    <CandleChart
      candles={candles}
      intervalSeconds={300}
      priceDecimalPlaces={12}
      formatUnixDateTime={(value) => String(value ?? "")}
      maximized
      onMaximizedChange={() => undefined}
      t={(zh) => zh}
    />,
  );

  assert.match(minuteHtml, /data-bancor-chart-layer="one-minute-close-line"/);
  assert.match(minuteHtml, /<polyline[^>]+stroke="var\(--theme-chart-close-line\)"/);
  assert.match(minuteHtml, /data-bancor-chart-maximized="false"/);
  assert.match(minuteHtml, /aria-label="最大化 K 线与交易区域"/);
  assert.match(minuteHtml, /aria-controls="bancor-market-workspace"/);
  assert.match(longerHtml, /data-bancor-chart-maximized="true"/);
  assert.match(longerHtml, /aria-label="恢复市场布局"/);
  assert.equal(bancorMarketWorkspaceClass(true), "bancor-market-workspace-maximized");
  assert.match(bancorMarketWorkspaceClass(false), /lg:grid-cols-/);
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
        input_asset: "AIC",
        input_units: "1000000000",
        input_amount: "10.00000000",
        fee_asset: "AIC",
        fee_units: "5000000",
        fee_amount: "0.05000000",
        curve_input_units: "995000000",
        curve_input_amount: "9.95000000",
        output_asset: "USD",
        output_units: "100000",
        output_amount: "0.00100000",
        price_impact_bps: 12,
        fee_bps: 50,
        market_id: "aic-usd-v1",
        market_version: 3,
        slippage_bps: 50,
        min_output_units: "90000",
        min_output_amount: "0.00090000",
      }}
      tradeLoading={false}
      tradeError={null}
      signingDeviceReady
      assetOwnerReady
      onClose={() => undefined}
      onConfirm={() => undefined}
    />,
  );

  assert.match(html, /fixed inset-0[^>]*items-center justify-center/);
  assert.match(html, /role="dialog"/);
  assert.match(html, /aria-modal="true"/);
  assert.match(html, /查看报价并确认交易/);
  assert.match(html, /data-nni-decimal-amount="10\.00000000 AIC"/);
  assert.match(html, /data-nni-decimal-amount="0\.00100000 USD"/);
  assert.match(html, /data-nni-decimal-amount="0\.00100000 USD"[^>]*data-nni-decimal-fraction-size="normal"/);
  assert.match(html, /data-nni-decimal-amount="0\.05000000 AIC"/);
  assert.match(html, /data-nni-decimal-amount="0\.05000000 AIC"[^>]*data-nni-decimal-fraction-size="normal"/);
  assert.match(html, /bancor-sign-trade-btn/);
  assert.match(html, /确认签名交易/);
  assert.match(html, /当前硬件代理签名/);
  assert.match(html, /使用资产密钥自行签名/);
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
        input_units: "500000000000",
        input_amount: "5000.00000000",
        fee_asset: "USD",
        fee_units: "2500000000",
        fee_amount: "25.00000000",
        curve_input_units: "497500000000",
        curve_input_amount: "4975.00000000",
        output_asset: "AIC",
        output_units: "3322203672780000",
        output_amount: "33222036.72780000",
        price_impact_bps: 3353,
        fee_bps: 50,
        market_id: "aic-usd-v1",
        market_version: 3,
        slippage_bps: 50,
        min_output_units: "3305592654410000",
        min_output_amount: "33055926.54410000",
      }}
      tradeLoading={false}
      tradeError={null}
      signingDeviceReady
      assetOwnerReady
      onClose={() => undefined}
      onConfirm={() => undefined}
    />,
  );

  assert.match(html, /价格影响 33\.53% 已超过你设置的 0\.50% 滑点警戒值/);
  assert.match(html, /data-nni-decimal-amount="33222036\.72780000 AIC"/);
  assert.match(html, /data-nni-decimal-amount="33055926\.54410000 AIC"/);
  assert.match(html, /确认后仍可继续/);
  assert.match(html, /接受当前价格影响，确认签名/);
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

  const flat = calculateBancorPriceDomain([{ high: 0.00010000, low: 0.00010000 }]);
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
