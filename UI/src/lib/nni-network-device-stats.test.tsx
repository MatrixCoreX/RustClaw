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
  shouldOfferNniOwnerRecovery,
} from "../components/NniPage";

test("keeps the unauthorized-device guidance user-facing", () => {
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
        total_distributed_reward_aic: "12500.00000000",
        settled_period_count: 3,
        first_period_start_unix: 1_800_000_000,
        latest_period_end_unix: 1_800_001_800,
      }}
      rewardPolicy={{
        interval_seconds: 600,
        initial_reward_pool_aic: 500000,
        current_reward_pool_units: "50000000000000",
        current_reward_pool_aic: "500000.00000000",
        distribution: "equal_per_eligible_device",
        halving_epoch_unix: 1_799_000_000,
        halving_interval_seconds: 126_144_000,
        halving_era: 0,
        rewards_ended: false,
        next_halving_at_unix: 1_925_144_000,
      }}
      localPreviousRewardAic="62.50000000"
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
  assert.match(markup, /总奖励/);
  assert.match(markup, /本机上个窗口/);
  assert.match(markup, /data-nni-window-reward-layout="stacked"/);
  assert.equal((markup.match(/font-mono text-xs font-semibold text-white\/90/g) ?? []).length, 2);
  assert.doesNotMatch(markup, /当前每 10 分钟总奖励/);
  assert.match(markup, />500000<\/span>/);
  assert.match(markup, />62\.50000000<\/span>/);
  assert.doesNotMatch(markup, /500000\.00000000/);
  assert.doesNotMatch(markup, /AIC/);
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
  const dialogSource = readFileSync(new URL("../components/NniAssetAccountDialog.tsx", import.meta.url), "utf8");
  assert.match(source, /t\("资产账户", "Asset account"\)/);
  assert.match(source, /硬件芯片只识别设备并证明当前授权/);
  assert.doesNotMatch(source, /资产账户 A|芯片公钥 H|资产公钥 A/);
  assert.match(dialogSource, /data-nni-copy-owner-private-key="true"/);
  assert.match(dialogSource, /data-nni-download-owner-key-pair="true"/);
  assert.match(dialogSource, /t\("下载 JSON 备份", "Download JSON backup"\)/);
  assert.match(dialogSource, /privateKeyCopied \? t\("已复制", "Copied"\)/);
  assert.match(dialogSource, /data-nni-discard-owner-key-pair="true"/);
});

test("offers device recovery only before an asset account is bound or generated", () => {
  assert.equal(shouldOfferNniOwnerRecovery(false, null, false), true);
  assert.equal(shouldOfferNniOwnerRecovery(true, null, false), false);
  assert.equal(shouldOfferNniOwnerRecovery(false, "bound-asset-public-key", false), false);
  assert.equal(shouldOfferNniOwnerRecovery(false, null, true), false);
});

test("asset account UI keeps recovery while adding custom bind, replacement, and unbind controls", () => {
  const source = readFileSync(new URL("../components/NniPage.tsx", import.meta.url), "utf8");
  const dialogSource = readFileSync(new URL("../components/NniAssetAccountDialog.tsx", import.meta.url), "utf8");
  assert.match(source, /t\("换机恢复", "Recover on this device"\)/);
  assert.match(source, /t\("重新绑定资产账户", "Rebind asset account"\)/);
  assert.match(source, /t\("更换资产账户", "Replace asset account"\)/);
  assert.match(source, /t\("解绑资产密钥", "Unbind asset key"\)/);
  assert.match(source, /onStartOwnerUnbind/);
  assert.match(dialogSource, /data-nni-asset-account-dialog/);
  assert.match(dialogSource, /t\("输入私钥", "Use private key"\)/);
  assert.match(dialogSource, /t\("外部签名", "External signature"\)/);
  assert.match(dialogSource, /t\("前往 HTTPS 设置", "Open HTTPS settings"\)/);
  assert.match(dialogSource, /data-nni-insecure-http-risk-acceptance="true"/);
  assert.match(dialogSource, /本浏览器会话通过 HTTP 操作资产私钥的风险/);
  assert.match(dialogSource, /data-nni-created-owner-bind/);
  assert.match(dialogSource, /t\("确认已保存私钥", "Confirm private-key backup"\)/);
  assert.match(dialogSource, /不会发送给本机服务、远程节点或写入存储/);
  assert.match(dialogSource, /data-nni-recovery-warning="true"/);
  assert.match(dialogSource, /仅当原设备已损坏或无法再使用时/);
  assert.match(dialogSource, /撤销该资产账户当前关联的其他设备授权/);
  assert.equal(dialogSource.match(/autoComplete="one-time-code"/g)?.length, 2);
  assert.equal(dialogSource.match(/data-1p-ignore="true"/g)?.length, 2);
  assert.equal(dialogSource.match(/data-lpignore="true"/g)?.length, 2);
  assert.doesNotMatch(dialogSource, /autoComplete="new-password"/);
  assert.doesNotMatch(source, /previousOwnerAuthorizationSignature/);
});

test("non-admin console sessions cannot display NNI, Bancor, or asset pages", () => {
  const source = readFileSync(new URL("../App.tsx", import.meta.url), "utf8");
  assert.match(source, /ADMIN_ONLY_UI_PAGES = new Set<ConsolePage>\(\["nni", "nni_apr", "bancor", "assets"\]\)/);
  assert.match(source, /navItems\.filter\(\(item\) => !ADMIN_ONLY_UI_PAGES\.has\(item\.id\)\)/);
  assert.match(source, /!isAdminIdentity && ADMIN_ONLY_UI_PAGES\.has\(currentPage\)/);
  assert.match(source, /isAdminIdentity && currentPage === "nni"/);
  assert.match(source, /isAdminIdentity && currentPage === "bancor"/);
  assert.match(source, /isAdminIdentity && currentPage === "assets"/);
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
  assert.equal((markup.match(/暂不可用/g) ?? []).length, 7);
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
        total_distributed_reward_aic: "0.00000000",
        settled_period_count: 0,
        first_period_start_unix: null,
        latest_period_end_unix: null,
      }}
      rewardPolicy={{
        interval_seconds: 600,
        initial_reward_pool_aic: 5000,
        current_reward_pool_units: "500000000000",
        current_reward_pool_aic: "5000.00000000",
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
