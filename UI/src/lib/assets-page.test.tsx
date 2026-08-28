import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";

import { AssetsPage, calculateAssetPortfolioValues } from "../components/AssetsPage";
import type { NniBancorAccountResponse, NniBancorMarketResponse } from "../types/api";

const account: NniBancorAccountResponse = {
  schema_version: 1,
  status: "bancor_account",
  device_pubkey: "device-public-key",
  aic_balance_units: "10000000000",
  aic_balance: "100.00000000",
  usd_balance_units: "500000000",
  usd_balance: "5.00000000",
  account_version: 1,
  page: 1,
  per_page: 10,
  total: 0,
  total_pages: 0,
  trades: [],
};

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
    day_start_unix: 1_700_000_000,
    open_usd_per_aic: "0.00010000",
    high_usd_per_aic: "0.00010000",
    low_usd_per_aic: "0.00010000",
    change_percent: "0",
    trade_count: 0,
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
};

const baseProps = {
  t: (zh: string) => zh,
  account,
  market,
  assetOwnerPubkey: "asset-owner-public-key",
  signingDeviceReady: true,
  accountLoading: false,
  marketLoading: false,
  error: null,
  hardwareAccountAccessUnavailable: false,
  transferLoading: false,
  transferError: null,
  transferMessage: null,
  onTransfer: async () => null,
  onClearTransferFeedback: () => undefined,
  onRefresh: () => undefined,
  onOpenBancor: () => undefined,
  onOpenNni: () => undefined,
};

test("asset portfolio valuation uses fixed decimal arithmetic", () => {
  assert.deepEqual(calculateAssetPortfolioValues(account, market), {
    aicValueUsd: "0.01000000",
    totalValueUsd: "5.01000000",
  });

  assert.equal(calculateAssetPortfolioValues(
    { ...account, aic_balance: "not-a-number" },
    market,
  ), null);
});

test("asset wallet shows AIC, USD, account identity, and market estimate", () => {
  const markup = renderToStaticMarkup(<AssetsPage {...baseProps} />);

  assert.match(markup, /data-assets-page="true"/);
  assert.match(markup, /资产总览/);
  assert.match(markup, /总资产估值/);
  assert.match(markup, /data-assets-total-value="5\.01000000"/);
  assert.match(markup, /data-assets-list-ready="true"/);
  assert.match(markup, />AIC</);
  assert.match(markup, />USD</);
  assert.match(markup, /asset-owner-public-key/);
  assert.match(markup, /data-assets-account-selector="true"/);
  assert.match(markup, /本机绑定账户/);
  assert.match(markup, /本机绑定账户 · asset-owner-public-key/);
  assert.match(markup, /不代表实际成交金额/);
  assert.match(markup, /data-assets-overview-actions="true"/);
  assert.match(markup, />转账</);
  assert.ok(markup.indexOf(">转账<") < markup.indexOf(">刷新资产<"));
  assert.doesNotMatch(markup, /查看当前资产账户中的余额与按市场价格估算的价值/);
  assert.doesNotMatch(markup, /<h2[^>]*>资产<\/h2>/);
});

test("asset account selector reserves additional wallet options", () => {
  const markup = renderToStaticMarkup(
    <AssetsPage
      {...baseProps}
      additionalAssetAccounts={[{
        id: "cold-wallet",
        publicKey: "external-asset-public-key",
        source: "external",
        label: "冷钱包",
      }]}
    />,
  );

  assert.match(markup, /本机绑定账户/);
  assert.match(markup, /冷钱包/);
  assert.match(markup, /value="cold-wallet"/);
  assert.match(markup, /冷钱包 · external-asset-public-key/);
  assert.equal((markup.match(/<option/g) ?? []).length, 2);
});

test("asset wallet explains missing account setup without exposing stale balances", () => {
  const markup = renderToStaticMarkup(
    <AssetsPage
      {...baseProps}
      account={null}
      assetOwnerPubkey={null}
      signingDeviceReady={false}
    />,
  );

  assert.match(markup, /data-assets-empty-state="true"/);
  assert.match(markup, /尚未绑定资产账户/);
  assert.match(markup, /请前往 NNI 页面创建或绑定资产账户/);
  assert.match(markup, /前往 NNI 绑定/);
  assert.doesNotMatch(markup, /data-assets-list-ready/);
});

test("asset wallet keeps the English interface fully localized", () => {
  const markup = renderToStaticMarkup(
    <AssetsPage
      {...baseProps}
      t={(_zh, en) => en}
    />,
  );

  assert.match(markup, /Asset overview/);
  assert.match(markup, /Estimated portfolio value/);
  assert.match(markup, /Asset list/);
  assert.match(markup, /Manage account/);
  assert.doesNotMatch(markup, /View balances in the current asset account/);
  assert.doesNotMatch(markup, /资产总览|总资产估值|资产列表|管理账户/);
});

test("asset navigation is between Bancor and account binding", () => {
  const source = readFileSync(new URL("../hooks/useConsoleProjections.tsx", import.meta.url), "utf8");
  const bancor = source.indexOf('id: "bancor" as const');
  const assets = source.indexOf('id: "assets" as const');
  const channels = source.indexOf('id: "channels" as const');

  assert.ok(bancor >= 0);
  assert.ok(assets > bancor);
  assert.ok(channels > assets);
  assert.match(source.slice(assets, channels), /label: t\("资产", "Assets"\)/);
});
