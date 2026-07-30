import test from "node:test";
import assert from "node:assert/strict";

import type { WorkspaceUpdateStatus } from "../types/api";
import {PRODUCT_DISPLAY_NAME} from "./product-identity.ts";
import {
  buildWorkspaceVersionDisplay,
  buildWorkspaceUpdateView,
  formatWorkspaceUpdateApiError,
  formatWorkspaceUpdateNextStep,
  formatWorkspaceUpdateStatus,
  formatWorkspaceUpdateStep,
  formatWorkspaceUpdateTime,
  shouldReloadAfterWorkspaceBuild,
} from "./workspace-update.ts";

function status(overrides: Partial<WorkspaceUpdateStatus>): WorkspaceUpdateStatus {
  return {
    status: "idle",
    step: "idle",
    stdout_tail: "",
    stderr_tail: "",
    ...overrides,
  };
}

test("shows only Git revisions for source checkouts", () => {
  assert.deepEqual(
    buildWorkspaceVersionDisplay(
      status({
        installation_kind: "source_checkout",
        old_commit: "203884fb",
        new_commit: "203884fb",
        remote_commit: "203884fb",
        current_version: "0.1.8",
        current_release_version: "release-old",
        latest_release_tag: "release-new",
      }),
    ),
    { kind: "git", current: "203884fb", latest: "203884fb" },
  );
});

test("shows only Release versions for release packages", () => {
  assert.deepEqual(
    buildWorkspaceVersionDisplay(
      status({
        installation_kind: "release_package",
        old_commit: "git-old",
        new_commit: "git-new",
        remote_commit: "git-remote",
        current_version: "0.1.8",
        current_release_version: "ubuntu-x86_64-20260727-1",
        latest_release_tag: "ubuntu-x86_64-20260727-2",
      }),
    ),
    {
      kind: "release",
      current: "ubuntu-x86_64-20260727-1",
      latest: "ubuntu-x86_64-20260727-2",
    },
  );
});

test("formats workspace update steps and statuses", () => {
  assert.equal(formatWorkspaceUpdateStep("building_ui", "en"), "Building UI");
  assert.equal(formatWorkspaceUpdateStep("building_clawd", "zh"), "正在编译 clawd");
  assert.equal(formatWorkspaceUpdateStep("custom_step", "en"), "custom_step");
  assert.equal(formatWorkspaceUpdateStatus("running", "release_deploy", "en"), "Deploying");
  assert.equal(formatWorkspaceUpdateStatus("running", "source_checkout", "en"), "Switching");
  assert.equal(formatWorkspaceUpdateStatus("running", "ui_only", "zh"), "编译中");
  assert.equal(formatWorkspaceUpdateStatus("running", "full_preserve_nginx", "zh"), "更新中");
  assert.equal(formatWorkspaceUpdateStatus("running", "nginx_enable", "zh"), "配置中");
  assert.equal(formatWorkspaceUpdateStatus("running", "nginx_disable", "en"), "Disabling");
  assert.equal(formatWorkspaceUpdateStatus("failed", undefined, "en"), "Failed");
  assert.equal(formatWorkspaceUpdateStatus("idle", undefined, "zh"), "待更新");
  assert.equal(formatWorkspaceUpdateApiError("workspace_update_already_running", "en"), "An update is already running.");
  assert.equal(formatWorkspaceUpdateApiError("workspace_update_admin_required", "zh"), "只有管理员可以执行这个操作。");
  assert.equal(
    formatWorkspaceUpdateApiError("workspace_update_source_checkout_required", "en"),
    "This installation uses a Release package and can only be updated through Releases.",
  );
  assert.equal(
    formatWorkspaceUpdateApiError("workspace_update_release_platform_unsupported", "zh"),
    "当前系统或架构没有可用的预编译 Release 包，请继续使用源码模式。",
  );
  assert.equal(formatWorkspaceUpdateApiError("custom_code", "en"), "custom_code");
});

test("builds running workspace update view", () => {
  const view = buildWorkspaceUpdateView(status({ status: "running", step: "building_workspace" }), "en");
  assert.equal(view.running, true);
  assert.equal(view.progressVisible, true);
  assert.equal(view.restarting, false);
  assert.equal(view.progressPercent, 82);
  assert.equal(view.progressActive, true);
  assert.equal(view.progressLabel, "Building; duration depends on device performance.");
  assert.equal(view.notice?.tone, "info");
  assert.equal(view.notice?.title, "Running full build/deploy");
});

test("shows active stage progress only while a UI-only build is running", () => {
  const view = buildWorkspaceUpdateView(
    status({
      status: "running",
      step: "building_ui",
      mode: "ui_only",
      started_ts: 1,
    }),
    "en",
  );

  assert.equal(view.progressVisible, true);
  assert.equal(view.progressPercent, 82);
  assert.equal(view.progressActive, true);
  assert.equal(
    view.progressLabel,
    "Building the UI; it will be published to the current deployment when finished.",
  );
});

test("builds release deployment progress view", () => {
  const view = buildWorkspaceUpdateView(status({ status: "running", mode: "release_deploy", step: "deploying_release" }), "en");
  assert.equal(view.progressPercent, 78);
  assert.equal(view.progressLabel, "Deploying the Release package; configs will be preserved and clawd will restart.");
  assert.equal(view.notice?.detail, "Release deployment is running. Logs will keep refreshing below.");
});

test("builds nginx repair and UI deployment progress view", () => {
  const enabling = buildWorkspaceUpdateView(
    status({ status: "running", mode: "nginx_enable", step: "enabling_nginx" }),
    "en",
  );
  assert.equal(enabling.progressPercent, 45);
  assert.match(enabling.progressLabel, /installing, or updating nginx/);
  assert.match(enabling.progressLabel, /deploying the current UI/);
});

test("builds nginx disable warning and completion views", () => {
  const disabling = buildWorkspaceUpdateView(
    status({ status: "running", mode: "nginx_disable", step: "disabling_nginx" }),
    "en",
  );
  assert.equal(disabling.progressPercent, 45);
  assert.match(
    disabling.progressLabel,
    new RegExp(`removing the ${PRODUCT_DISPLAY_NAME.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")} site`),
  );
  assert.match(disabling.notice?.detail ?? "", /may disconnect immediately/);

  const completed = buildWorkspaceUpdateView(
    status({ status: "succeeded", mode: "nginx_disable", step: "nginx_disabled" }),
    "en",
  );
  assert.equal(completed.notice?.title, "nginx was disabled and the deployed UI was removed.");
  assert.match(completed.notice?.detail ?? "", /remote entry is unavailable/);
});

test("builds source checkout migration progress view", () => {
  const view = buildWorkspaceUpdateView(
    status({
      status: "running",
      mode: "source_checkout",
      step: "cloning_source_checkout",
      next_step_key: "workspace_update.source_checkout_cloning",
    }),
    "en",
  );
  assert.equal(view.progressPercent, 45);
  assert.match(view.progressLabel, /Validating source/);
  assert.equal(view.notice?.title, "Fetching complete source");
  assert.match(view.notice?.detail ?? "", /current Release installation remains unchanged/);
});

test("builds failed and canceled notices", () => {
  const failed = buildWorkspaceUpdateView(status({ status: "failed", error: "compile_failed", mode: "ui_only" }), "en");
  assert.equal(failed.notice?.tone, "error");
  assert.equal(failed.notice?.title, "compile_failed");
  assert.match(failed.notice?.detail ?? "", /Git, network, or build/);

  const canceled = buildWorkspaceUpdateView(status({ status: "canceled" }), "zh");
  assert.equal(canceled.notice?.tone, "info");
  assert.equal(canceled.notice?.title, "编译已停止。");

  const sourceFailed = buildWorkspaceUpdateView(
    status({
      status: "failed",
      mode: "source_checkout",
      error: "source_checkout_migration_failed",
      next_step_key: "workspace_update.source_checkout_failed",
    }),
    "zh",
  );
  assert.equal(sourceFailed.notice?.title, "源码模式切换失败");
  assert.match(sourceFailed.notice?.detail ?? "", /Release 安装保持不变/);
});

test("formats workspace update next-step keys and legacy fallback", () => {
  assert.equal(
    formatWorkspaceUpdateNextStep(
      status({
        next_step_key: "workspace_update.config_saved_retrying_pull",
      }),
      "en",
    ),
    "The current runtime configuration is saved in memory while source is pulled, and will be restored automatically afterward.",
  );
  assert.equal(
    formatWorkspaceUpdateNextStep(
      status({
        next_step_key: "workspace_update.restart_wait",
      }),
      "zh",
    ),
    `${PRODUCT_DISPLAY_NAME} 正在重启，请等待 10-20 秒后刷新页面。`,
  );
  assert.equal(
    formatWorkspaceUpdateNextStep(status({ next_step: "legacy next step" }), "en"),
    "legacy next step",
  );
  assert.equal(
    formatWorkspaceUpdateNextStep(status({ next_step_key: "workspace_update.unknown" }), "en"),
    "workspace_update.unknown",
  );
});

test("uses workspace update next-step keys in notices", () => {
  const running = buildWorkspaceUpdateView(
    status({
      status: "running",
      step: "building_ui",
      next_step_key: "workspace_update.build_logs_refreshing",
    }),
    "en",
  );
  assert.equal(running.notice?.detail, "Building. Build logs will keep refreshing.");

  const failed = buildWorkspaceUpdateView(
    status({
      status: "failed",
      error: "git fetch failed",
      next_step_key: "workspace_update.remote_fetch_required_failed",
    }),
    "zh",
  );
  assert.match(failed.notice?.detail ?? "", /远端检查失败/);

  const canceled = buildWorkspaceUpdateView(
    status({
      status: "canceled",
      next_step_key: "workspace_update.canceled",
    }),
    "en",
  );
  assert.equal(canceled.notice?.detail, "The build stopped. Fix any issues, then build again.");
});

test("recognizes remote up-to-date status", () => {
  const view = buildWorkspaceUpdateView(
    status({ status: "idle", old_commit: "abc", remote_commit: "abc" }),
    "en",
  );
  assert.equal(view.knownUpToDate, true);
  assert.equal(view.displayStatus, "up_to_date");
  assert.equal(view.notice?.tone, "success");
  assert.equal(view.progressVisible, false);
  assert.equal(view.progressPercent, 0);
});

test("keeps progress hidden after checks and completed UI builds", () => {
  const checked = buildWorkspaceUpdateView(
    status({ status: "up_to_date", step: "already_latest" }),
    "en",
  );
  assert.equal(checked.progressVisible, false);

  const completed = buildWorkspaceUpdateView(
    status({
      status: "succeeded",
      step: "ui_build_succeeded",
      mode: "ui_only",
      started_ts: 1,
      finished_ts: 2,
    }),
    "en",
  );
  assert.equal(completed.progressVisible, false);
  assert.equal(completed.progressPercent, 100);
  assert.equal(completed.notice?.tone, "success");
  assert.equal(completed.notice?.title, "UI build and deployment completed.");
});

test("reloads once after compile modes complete but not after release deployment", () => {
  assert.equal(shouldReloadAfterWorkspaceBuild(true, "ui_only", "succeeded"), true);
  assert.equal(shouldReloadAfterWorkspaceBuild(true, "full", "up_to_date"), true);
  assert.equal(shouldReloadAfterWorkspaceBuild(true, "clawd_only", "idle"), true);
  assert.equal(shouldReloadAfterWorkspaceBuild(true, "release_deploy", "up_to_date"), false);
  assert.equal(shouldReloadAfterWorkspaceBuild(true, "nginx_enable", "succeeded"), false);
  assert.equal(shouldReloadAfterWorkspaceBuild(true, "nginx_disable", "succeeded"), false);
  assert.equal(shouldReloadAfterWorkspaceBuild(false, "ui_only", "succeeded"), false);
  assert.equal(shouldReloadAfterWorkspaceBuild(true, "ui_only", "failed"), false);
});

test("formats log preview and timestamps", () => {
  const view = buildWorkspaceUpdateView(status({ stdout_tail: "ok", stderr_tail: "warn" }), "en");
  assert.equal(view.logPreview, "Operation output\nok\n\nOperation log (stderr, not necessarily errors)\nwarn");
  assert.equal(formatWorkspaceUpdateTime(null, "en"), "--");
  assert.match(formatWorkspaceUpdateTime(1782197321, "en"), /2026|6|23/);
});
