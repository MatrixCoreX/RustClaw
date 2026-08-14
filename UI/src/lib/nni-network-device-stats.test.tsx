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
        window_seconds: 600,
      }}
      loading={false}
      joined
      t={(zh) => zh}
      formatUnixDateTime={(value) => String(value ?? "--")}
    />,
  );

  assert.match(markup, /网络设备概览/);
  assert.match(markup, /注册设备/);
  assert.match(markup, />12</);
  assert.doesNotMatch(markup, /服务端白名单中的设备/);
  assert.match(markup, /活跃设备/);
  assert.match(markup, />8</);
  assert.doesNotMatch(markup, /上个 10 分钟窗口内提交过心跳/);
  assert.match(markup, /1800000000.*1800000600/);
  assert.equal((markup.match(/flex items-center justify-between gap-3/g) ?? []).length, 2);
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
  assert.equal((markup.match(/未加入/g) ?? []).length, 2);
  assert.match(markup, /加入网络后可查看/);
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

  assert.equal((markup.match(/暂不可用/g) ?? []).length, 2);
  assert.match(markup, /刷新状态后重试/);
  assert.doesNotMatch(markup, /未加入/);
});
