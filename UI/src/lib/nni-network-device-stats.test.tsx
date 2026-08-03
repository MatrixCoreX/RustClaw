import assert from "node:assert/strict";
import test from "node:test";
import { renderToStaticMarkup } from "react-dom/server";

import { NniNetworkDeviceStats } from "../components/NniNetworkDeviceStats";

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

test("keeps the network counters understandable before the first settlement", () => {
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
  assert.match(markup, /等待首个窗口结算/);
});
