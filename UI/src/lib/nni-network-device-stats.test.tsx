import assert from "node:assert/strict";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";

import { NniNetworkDeviceStats } from "../components/NniNetworkDeviceStats";
import {
  NNI_DEVICE_AUTHORIZATION_DENIED_COPY,
  NNI_DEVICE_MANAGEMENT_COPY,
} from "../components/NniPage";

test("describes NNI as a hardware-device capability instead of a Pi App feature", () => {
  assert.equal(NNI_DEVICE_MANAGEMENT_COPY.zh, "这里管理硬件设备的 NNI 入口和设备签名能力。");
  assert.match(NNI_DEVICE_MANAGEMENT_COPY.en, /hardware device/);
  assert.doesNotMatch(Object.values(NNI_DEVICE_MANAGEMENT_COPY).join(" "), /Pi App/);
  assert.equal(NNI_DEVICE_AUTHORIZATION_DENIED_COPY.zh, "你不是合法设备，不能参与 NNI 网络。");
  assert.match(NNI_DEVICE_AUTHORIZATION_DENIED_COPY.en, /not an authorized device/);
});

test("shows registered allowlist devices and active devices from the previous heartbeat window", () => {
  const markup = renderToStaticMarkup(
    <NniNetworkDeviceStats
      stats={{
        registered_device_count: 12,
        active_device_count: 8,
        active_period_start_unix: 1_800_000_000,
        active_period_end_unix: 1_800_000_600,
        first_heartbeat_unix: 1_799_000_000,
        window_seconds: 600,
      }}
      networkRewards={{
        total_distributed_reward_units: "125000000",
        total_distributed_reward_points: "12500.0000",
        settled_period_count: 3,
        first_period_start_unix: 1_800_000_000,
        latest_period_end_unix: 1_800_001_800,
      }}
      rewardPolicy={{
        interval_seconds: 600,
        initial_reward_pool_points: 5000,
        current_reward_pool_units: "50000000",
        current_reward_pool_points: "5000.0000",
        distribution: "equal_per_eligible_device",
        halving_epoch_unix: 1_799_000_000,
        halving_interval_seconds: 126_144_000,
        halving_era: 0,
        rewards_ended: false,
        next_halving_at_unix: 1_925_144_000,
      }}
      loading={false}
      joined
      t={(zh) => zh}
      formatUnixDateTime={(value) => String(value ?? "--")}
    />,
  );

  assert.match(markup, /网络概览/);
  assert.match(markup, /注册设备/);
  assert.match(markup, />12</);
  assert.doesNotMatch(markup, /服务端白名单中的设备/);
  assert.match(markup, /活跃设备/);
  assert.match(markup, />8</);
  assert.doesNotMatch(markup, /上个 10 分钟窗口内提交过心跳/);
  assert.doesNotMatch(markup, /1800000000.*1800000600/);
  assert.match(markup, /全网累计产出/);
  assert.match(markup, /12500\.0000/);
  assert.match(markup, /当前每 10 分钟总奖励/);
  assert.match(markup, /5000\.0000/);
  assert.doesNotMatch(markup, /POINT/);
  assert.doesNotMatch(markup, /由本周期有效心跳设备平分/);
  assert.match(markup, /全网首次心跳时间/);
  assert.match(markup, /1799000000/);
  assert.match(markup, /下次减半时间/);
  assert.match(markup, /1925144000/);
  assert.match(markup, /sm:grid-cols-2 md:grid-cols-3/);
  assert.equal((markup.match(/rounded-lg border border-white\/10/g) ?? []).length, 6);
});

test("explains why network counters are unavailable before joining", () => {
  const markup = renderToStaticMarkup(
    <NniNetworkDeviceStats
      stats={null}
      loading={false}
      joined={false}
      t={(zh) => zh}
      formatUnixDateTime={() => "--"}
    />,
  );

  assert.match(markup, /注册设备/);
  assert.match(markup, /活跃设备/);
  assert.equal((markup.match(/未加入/g) ?? []).length, 6);
  assert.doesNotMatch(markup, />--</);
});

test("distinguishes a joined network whose counters are temporarily unavailable", () => {
  const markup = renderToStaticMarkup(
    <NniNetworkDeviceStats
      stats={null}
      loading={false}
      joined
      t={(zh) => zh}
      formatUnixDateTime={() => "--"}
    />,
  );

  assert.equal((markup.match(/暂不可用/g) ?? []).length, 6);
  assert.doesNotMatch(markup, /未加入/);
});
