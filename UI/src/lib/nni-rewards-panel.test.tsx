import assert from "node:assert/strict";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";

import { NniRewardsPanel } from "../components/NniRewardsPanel";
import type { NniRewardsResponse } from "../types/api";

const rewards: NniRewardsResponse = {
  schema_version: 1,
  status: "heartbeat_rewards",
  device_pubkey:
    "2b9c9d84fa15f4e178ce58d0a40a9f5e150e9c502e689a24d0c0f221337870c" +
    "726f0e463d730a75401c425bfde0db0c442e314027d83885a84c535eaa35460a0",
  node_url: "https://nni.example.test",
  reward_point_scale: 10000,
  reward_decimal_places: 4,
  total_reward_units: "75000000",
  total_reward_points: "7500.0000",
  reward_grant_count: 2,
  first_period_start_unix: 1_800_000_000,
  latest_period_end_unix: 1_800_001_200,
  page: 1,
  per_page: 10,
  total: 2,
  total_pages: 1,
  records: [
    {
      id: 2,
      period_start_unix: 1_800_000_600,
      period_end_unix: 1_800_001_200,
      heartbeat_count_in_period: 3,
      eligibility_units: 1,
      reward_points_units: "50000000",
      reward_point_scale: 10000,
      reward_points: "5000.0000",
      rounding_adjustment_units: 0,
      awarded_at_unix: 1_800_001_201,
    },
  ],
};

test("renders the signed device reward total and period record", () => {
  const markup = renderToStaticMarkup(
    <NniRewardsPanel
      rewards={rewards}
      currentPointBalance="6250.1250"
      currentPointBalanceLoading={false}
      loading={false}
      error={null}
      pageSize={10}
      t={(zh) => zh}
      formatUnixDateTime={(value) => (value ? String(value) : "--")}
      onFetch={() => undefined}
      onRefresh={() => undefined}
    />,
  );

  assert.match(markup, /id="nni-history-rewards-panel"/);
  assert.match(markup, /原生智能奖励/);
  assert.match(markup, /7500\.0000/);
  assert.match(markup, /当前持有/);
  assert.match(markup, /6250\.1250/);
  assert.match(markup, /\+5000\.0000/);
  assert.match(markup, /3 次，按 1 台设备计奖/);
  assert.match(markup, /每次刷新都会由本机设备签署一次临时挑战/);
  assert.match(markup, /切换为原始十六进制公钥/);
});
