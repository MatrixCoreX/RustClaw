import assert from "node:assert/strict";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";

import {
  NniAprPage,
  persistNniAprDevicePrice,
  readNniAprDevicePrice,
} from "../components/NniAprPage";
import {
  calculateNniAprEstimate,
  calculateNniPeriodAprEstimate,
  latestNniRewardRecord,
  NNI_APR_AUTO_REFRESH_SECONDS,
  parsePositiveNniDevicePrice,
} from "./nni-apr";
import type { NniBancorMarketResponse, NniRewardsResponse } from "../types/api";

const rewards: NniRewardsResponse = {
  schema_version: 1,
  status: "heartbeat_rewards",
  device_pubkey: "device-key",
  reward_aic_scale: 100000000,
  reward_decimal_places: 8,
  total_reward_units: "1500000000",
  total_reward_aic: "15.00000000",
  reward_grant_count: 2,
  latest_period_end_unix: 1_800_001_200,
  reward_windows: [
    {
      key: "week",
      window_seconds: 604_800,
      window_start_unix: 1_799_396_410,
      window_end_unix: 1_800_001_210,
      total_reward_units: "7000000000",
      total_reward_aic: "70.00000000",
      reward_grant_count: 7,
    },
    {
      key: "month",
      window_seconds: 2_592_000,
      window_start_unix: 1_797_409_210,
      window_end_unix: 1_800_001_210,
      total_reward_units: "30000000000",
      total_reward_aic: "300.00000000",
      reward_grant_count: 30,
    },
    {
      key: "year",
      window_seconds: 31_536_000,
      window_start_unix: 1_768_465_210,
      window_end_unix: 1_800_001_210,
      total_reward_units: "365000000000",
      total_reward_aic: "3650.00000000",
      reward_grant_count: 365,
    },
  ],
  reward_policy: {
    interval_seconds: 600,
    initial_reward_pool_aic: 5000,
    current_reward_pool_units: "500000000000",
    current_reward_pool_aic: "5000.00000000",
    distribution: "equal_per_eligible_device",
    halving_epoch_unix: 1_800_000_000,
    halving_interval_seconds: 126_144_000,
    halving_era: 0,
    rewards_ended: false,
    next_halving_at_unix: 1_926_144_000,
  },
  page: 1,
  per_page: 100,
  total: 2,
  total_pages: 1,
  history_limit: 100,
  history_truncated: false,
  records: [
    {
      id: 1,
      period_start_unix: 1_800_000_000,
      period_end_unix: 1_800_000_600,
      heartbeat_count_in_period: 2,
      eligibility_units: 1,
      reward_aic_units: "500000000",
      reward_aic_scale: 100000000,
      reward_aic: "5.00000000",
      rounding_adjustment_units: 0,
      awarded_at_unix: 1_800_000_601,
    },
    {
      id: 2,
      period_start_unix: 1_800_000_600,
      period_end_unix: 1_800_001_200,
      heartbeat_count_in_period: 3,
      eligibility_units: 1,
      reward_aic_units: "1000000000",
      reward_aic_scale: 100000000,
      reward_aic: "10.00000000",
      rounding_adjustment_units: 0,
      awarded_at_unix: 1_800_001_201,
    },
  ],
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
  usd_reserve_units: "5000000000000000",
  usd_reserve: "50000000.00000000",
  marginal_price_usd_per_aic: "0.50000000",
  daily_marginal_price: {
    price_kind: "pool_marginal_usd_per_aic",
    timezone: "UTC",
    day_start_unix: 1_800_000_000,
    open_usd_per_aic: "0.50000000",
    high_usd_per_aic: "0.51000000",
    low_usd_per_aic: "0.49000000",
    change_percent: "0.00",
    trade_count: 0,
  },
  fee_bps: 50,
  version: 1,
  updated_at_unix: 1_800_001_210,
};

test("NNI APR uses the latest settled device reward and the Bancor marginal price", () => {
  assert.equal(NNI_APR_AUTO_REFRESH_SECONDS, 600);
  assert.equal(latestNniRewardRecord([...rewards.records].reverse())?.id, 2);
  const estimate = calculateNniAprEstimate({
    devicePriceUsd: "1000",
    rewards,
    market,
  });
  assert.ok(estimate);
  assert.equal(estimate.record.id, 2);
  assert.equal(estimate.periodSeconds, 600);
  assert.equal(estimate.periodValueUsd, 5);
  assert.equal(estimate.aprBasisRewardUsd, 262800);
  assert.equal(estimate.aprPercent, 26280);
});

test("NNI period APR uses the complete selected reward window", () => {
  const weekly = calculateNniPeriodAprEstimate({
    devicePriceUsd: "1000",
    rewards,
    market,
    windowKey: "week",
  });
  assert.ok(weekly);
  assert.equal(weekly.window.key, "week");
  assert.equal(weekly.rewardAic, 70);
  assert.equal(weekly.windowValueUsd, 35);
  assert.equal(weekly.aprBasisRewardUsd, 1825);
  assert.equal(weekly.aprPercent, 182.5);

  const monthly = calculateNniPeriodAprEstimate({
    devicePriceUsd: "1000",
    rewards,
    market,
    windowKey: "month",
  });
  assert.ok(monthly);
  assert.equal(monthly.rewardAic, 300);
  assert.equal(monthly.windowValueUsd, 150);
  assert.equal(monthly.aprPercent, 182.5);
});

test("NNI APR rejects empty, zero, negative, and malformed device prices", () => {
  assert.equal(parsePositiveNniDevicePrice(""), null);
  assert.equal(parsePositiveNniDevicePrice("0"), null);
  assert.equal(parsePositiveNniDevicePrice("-1"), null);
  assert.equal(parsePositiveNniDevicePrice("10 USD"), null);
  assert.equal(parsePositiveNniDevicePrice(".5"), 0.5);
  assert.equal(calculateNniAprEstimate({ devicePriceUsd: "0", rewards, market }), null);
});

test("NNI APR device price persists through the product-neutral storage key", () => {
  const values = new Map<string, string>();
  const storage = {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => values.set(key, value),
  };
  persistNniAprDevicePrice(storage, "399.99");
  assert.equal(readNniAprDevicePrice(storage), "399.99");
  assert.match([...values.keys()][0] ?? "", /^agent-runtime\./);
});

test("NNI APR page explains its inputs, refresh cadence, and estimate boundary", () => {
  const markup = renderToStaticMarkup(
    <NniAprPage
      lang="zh"
      t={(zh) => zh}
      joined
      rewards={rewards}
      market={market}
      rewardsLoading={false}
      marketLoading={false}
      rewardsError={null}
      marketError={null}
      formatUnixDateTime={(value) => String(value ?? "")}
      onBack={() => undefined}
      onOpenBancor={() => undefined}
      onRefresh={() => undefined}
    />,
  );
  assert.match(markup, /NNI 奖励 APR/);
  assert.match(markup, /设备价格（USD）/);
  assert.match(markup, /实时 APR/);
  assert.match(markup, /周期 APR/);
  assert.match(markup, /周（最近 7 天）/);
  assert.match(markup, /月（最近 30 天）/);
  assert.match(markup, /年（最近 365 天）/);
  assert.match(markup, /假定这些奖励 AIC 没有卖出/);
  assert.match(markup, /每 10 分钟自动刷新/);
  assert.match(markup, /不含复利、交易手续费和价格影响/);
  const prohibitedRiskTerm = String.fromCodePoint(0x5e74, 0x5316);
  assert.doesNotMatch(markup, new RegExp(prohibitedRiskTerm));
  assert.match(markup, /返回 NNI/);
  assert.match(markup, /查看市场/);
});
