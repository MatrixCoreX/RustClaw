import assert from "node:assert/strict";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";

import { formatNniRewardCountdown, NniRewardsPanel } from "../components/NniRewardsPanel";
import type { NniRewardsResponse } from "../types/api";

const rewards: NniRewardsResponse = {
  schema_version: 1,
  status: "heartbeat_rewards",
  device_pubkey:
    "2b9c9d84fa15f4e178ce58d0a40a9f5e150e9c502e689a24d0c0f221337870c" +
    "726f0e463d730a75401c425bfde0db0c442e314027d83885a84c535eaa35460a0",
  node_url: "https://nni.example.test",
  reward_aic_scale: 100000000,
  reward_decimal_places: 8,
  total_reward_units: "750000000000",
  total_reward_aic: "7500.00000000",
  reward_grant_count: 2,
  first_period_start_unix: 1_800_000_000,
  latest_period_end_unix: 1_800_001_200,
  reward_policy: {
    phase: "scheduled",
    accepting_reward_heartbeats: false,
    activation_not_before_unix: 1_800_010_000,
    reward_start_time_unix: null,
    starts_in_seconds: 3_661,
    first_settlement_at_unix: null,
    interval_seconds: 600,
    initial_reward_pool_aic: 5000,
    current_reward_pool_units: null,
    current_reward_pool_aic: null,
    distribution: "equal_per_eligible_device",
    halving_epoch_unix: null,
    halving_interval_seconds: 126_144_000,
    halving_era: null,
    rewards_ended: false,
    next_halving_at_unix: null,
  },
  page: 1,
  per_page: 10,
  total: 2,
  total_pages: 1,
  history_limit: 100,
  history_truncated: false,
  records: [
    {
      id: 2,
      period_start_unix: 1_800_000_600,
      period_end_unix: 1_800_001_200,
      heartbeat_count_in_period: 3,
      eligibility_units: 1,
      reward_aic_units: "500000000000",
      reward_aic_scale: 100000000,
      reward_aic: "5000.00000000",
      rounding_adjustment_units: 0,
      awarded_at_unix: 1_800_001_201,
    },
  ],
};

test("renders the signed device reward total and period record", () => {
  const markup = renderToStaticMarkup(
    <NniRewardsPanel
      rewards={rewards}
      currentAicBalance="6250.12500000"
      currentAicBalanceLoading={false}
      loading={false}
      error={null}
      pageSize={100}
      t={(zh) => zh}
      formatUnixDateTime={(value) => (value ? String(value) : "--")}
      onRefresh={() => undefined}
    />,
  );

  assert.match(markup, /id="nni-history-rewards-panel"/);
  assert.match(markup, /原生智能奖励/);
  assert.match(markup, /data-nni-decimal-amount="7500\.00000000"/);
  assert.match(markup, /当前持有/);
  assert.match(markup, /data-nni-decimal-amount="6250\.12500000"/);
  assert.match(markup, /data-nni-decimal-amount="\+5000\.00000000"/);
  assert.match(markup, /data-nni-decimal-amount="\+5000\.00000000"[^>]*data-nni-decimal-fraction-size="normal"/);
  assert.match(markup, /3 次，按 1 台设备计奖/);
  assert.match(markup, /每次刷新都会由本机设备签署一次临时挑战/);
  assert.match(markup, /切换为原始十六进制公钥/);
  assert.match(markup, /（最近 100 条）/);
  assert.match(markup, /奖励开始倒计时/);
  assert.match(markup, /1 小时 1 分 1 秒/);
  assert.match(markup, /开始时间前的心跳只用于确认设备在线，不参与奖励/);
  assert.match(markup, /data-nni-reward-countdown="3661"/);
  assert.doesNotMatch(markup, /共 2 条/);
  assert.doesNotMatch(markup, /上一页|下一页/);
});

test("formats reward countdowns without hiding long durations", () => {
  assert.equal(formatNniRewardCountdown(90_061, (zh) => zh), "1 天 1 小时 1 分 1 秒");
  assert.equal(formatNniRewardCountdown(0, (_zh, en) => en), "0s");
});

test("explains that the first eligible heartbeat anchors rewards and halving", () => {
  const waitingRewards: NniRewardsResponse = {
    ...rewards,
    reward_policy: {
      ...rewards.reward_policy,
      phase: "waiting_first_heartbeat",
      accepting_reward_heartbeats: true,
      starts_in_seconds: 0,
    },
  };
  const markup = renderToStaticMarkup(
    <NniRewardsPanel
      rewards={waitingRewards}
      currentAicBalance="0.00000000"
      currentAicBalanceLoading={false}
      loading={false}
      error={null}
      pageSize={100}
      t={(zh) => zh}
      formatUnixDateTime={(value) => (value ? String(value) : "--")}
      onRefresh={() => undefined}
    />,
  );

  assert.match(markup, /等待全网首个有效心跳/);
  assert.match(markup, /同时确定奖励启动时间、首个十分钟结算窗口和减半周期起点/);
  assert.doesNotMatch(markup, /奖励开始倒计时/);
});
