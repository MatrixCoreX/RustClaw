import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";

import {
  NniNetworkDeviceStats,
  formatNniRewardMetric,
} from "../components/NniNetworkDeviceStats";
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
        total_distributed_reward_units: "1250000000000",
        total_distributed_reward_points: "12500.00000000",
        settled_period_count: 3,
        first_period_start_unix: 1_800_000_000,
        latest_period_end_unix: 1_800_001_800,
      }}
      rewardPolicy={{
        interval_seconds: 600,
        initial_reward_pool_points: 5000,
        current_reward_pool_units: "500000000000",
        current_reward_pool_points: "5000.00000000",
        distribution: "equal_per_eligible_device",
        halving_epoch_unix: 1_799_000_000,
        halving_interval_seconds: 126_144_000,
        halving_era: 0,
        rewards_ended: false,
        next_halving_at_unix: 1_925_144_000,
      }}
      loading={false}
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
  assert.match(markup, /累计产出/);
  assert.doesNotMatch(markup, /全网累计产出/);
  assert.match(markup, />12500<\/span>/);
  assert.doesNotMatch(markup, /12500\.00000000/);
  assert.match(markup, /窗口奖励/);
  assert.doesNotMatch(markup, /当前每 10 分钟总奖励/);
  assert.match(markup, />5000<\/span>/);
  assert.doesNotMatch(markup, /5000\.00000000/);
  assert.doesNotMatch(markup, /POINT/);
  assert.doesNotMatch(markup, /由本周期有效心跳设备平分/);
  assert.match(markup, /首跳/);
  assert.doesNotMatch(markup, /全网首次心跳时间/);
  assert.match(markup, /1799000000/);
  assert.match(markup, /减半/);
  assert.doesNotMatch(markup, /下次减半时间/);
  assert.match(markup, /1925144000/);
  assert.match(markup, /sm:grid-cols-2 md:grid-cols-3 xl:grid-cols-6/);
  assert.equal((markup.match(/whitespace-nowrap text-xs font-medium/g) ?? []).length, 2);
  assert.equal((markup.match(/rounded-lg border border-white\/10/g) ?? []).length, 6);
});

test("network reward metrics hide an all-zero fraction and retain real decimals", () => {
  assert.equal(formatNniRewardMetric("5000.00000000"), "5000");
  assert.equal(formatNniRewardMetric("5000"), "5000");
  assert.equal(formatNniRewardMetric("312.50000000"), "312.50000000");
});

test("places remote node configuration at the bottom of the runtime entry card", () => {
  const source = readFileSync(new URL("../components/NniPage.tsx", import.meta.url), "utf8");
  const nodeSettings = source.indexOf('t("远程 NNI 节点", "Remote NNI nodes")');
  assert.ok(nodeSettings > source.indexOf("nni-runtime-board"));
  assert.ok(nodeSettings > source.indexOf('t("心跳请求次数", "Heartbeat requests")'));
  assert.ok(nodeSettings > source.indexOf('"点击加入会向远程服务端请求一次随机挑战'));
});

test("asset account setup avoids key-letter jargon and protects key actions", () => {
  const source = readFileSync(new URL("../components/NniPage.tsx", import.meta.url), "utf8");
  assert.match(source, /t\("资产账户", "Asset account"\)/);
  assert.match(source, /硬件芯片只识别设备并证明当前授权/);
  assert.doesNotMatch(source, /资产账户 A|芯片公钥 H|资产公钥 A/);
  assert.match(source, /data-nni-copy-owner-private-key="true"/);
  assert.match(source, /ownerPrivateKeyCopied \? t\("已复制", "Copied"\)/);
  assert.match(source, /className="flex justify-end pt-1"/);
  assert.match(source, /data-nni-discard-owner-key-pair="true"/);
});

test("network counters never imply that public aggregate data requires joining", () => {
  const markup = renderToStaticMarkup(
    <NniNetworkDeviceStats
      stats={null}
      loading={false}
      t={(zh) => zh}
      formatUnixDateTime={() => "--"}
    />,
  );

  assert.match(markup, /注册设备/);
  assert.match(markup, /活跃设备/);
  assert.equal((markup.match(/暂不可用/g) ?? []).length, 6);
  assert.doesNotMatch(markup, /未加入/);
  assert.doesNotMatch(markup, />--</);
});

test("shows explicit first-heartbeat placeholders for a fresh public network", () => {
  const markup = renderToStaticMarkup(
    <NniNetworkDeviceStats
      stats={{
        registered_device_count: 153,
        active_device_count: 0,
        active_period_start_unix: null,
        active_period_end_unix: null,
        first_heartbeat_unix: null,
        window_seconds: 600,
      }}
      networkRewards={{
        total_distributed_reward_units: "0",
        total_distributed_reward_points: "0.00000000",
        settled_period_count: 0,
        first_period_start_unix: null,
        latest_period_end_unix: null,
      }}
      rewardPolicy={{
        interval_seconds: 600,
        initial_reward_pool_points: 5000,
        current_reward_pool_units: "500000000000",
        current_reward_pool_points: "5000.00000000",
        distribution: "equal_per_eligible_device",
        halving_epoch_unix: null,
        halving_interval_seconds: 126_144_000,
        halving_era: null,
        rewards_ended: false,
        next_halving_at_unix: null,
      }}
      loading={false}
      t={(zh) => zh}
      formatUnixDateTime={() => "--"}
    />,
  );

  assert.match(markup, />153</);
  assert.match(markup, />0</);
  assert.match(markup, /等待首跳/);
  assert.match(markup, /首跳后计算/);
  assert.doesNotMatch(markup, /未加入/);
});
