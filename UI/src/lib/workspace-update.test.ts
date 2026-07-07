import test from "node:test";
import assert from "node:assert/strict";

import type { WorkspaceUpdateStatus } from "../types/api";
import {
  buildWorkspaceUpdateView,
  formatWorkspaceUpdateStatus,
  formatWorkspaceUpdateStep,
  formatWorkspaceUpdateTime,
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

test("formats workspace update steps and statuses", () => {
  assert.equal(formatWorkspaceUpdateStep("building_ui", "en"), "Building UI");
  assert.equal(formatWorkspaceUpdateStep("building_clawd", "zh"), "正在编译 clawd");
  assert.equal(formatWorkspaceUpdateStep("custom_step", "en"), "custom_step");
  assert.equal(formatWorkspaceUpdateStatus("running", "release_deploy", "en"), "Deploying");
  assert.equal(formatWorkspaceUpdateStatus("running", "ui_only", "zh"), "编译中");
  assert.equal(formatWorkspaceUpdateStatus("failed", undefined, "en"), "Failed");
});

test("builds running workspace update view", () => {
  const view = buildWorkspaceUpdateView(status({ status: "running", step: "building_workspace" }), "en");
  assert.equal(view.running, true);
  assert.equal(view.restarting, false);
  assert.equal(view.progressPercent, 82);
  assert.equal(view.progressActive, true);
  assert.equal(view.progressLabel, "Building; duration depends on device performance.");
  assert.equal(view.notice?.tone, "info");
  assert.equal(view.notice?.title, "Running full build");
});

test("builds release deployment progress view", () => {
  const view = buildWorkspaceUpdateView(status({ status: "running", mode: "release_deploy", step: "deploying_release" }), "en");
  assert.equal(view.progressPercent, 78);
  assert.equal(view.progressLabel, "Deploying the Release package; configs will be preserved and clawd will restart.");
  assert.equal(view.notice?.detail, "Release deployment is running. Logs will keep refreshing below.");
});

test("builds failed and canceled notices", () => {
  const failed = buildWorkspaceUpdateView(status({ status: "failed", error: "compile_failed", mode: "ui_only" }), "en");
  assert.equal(failed.notice?.tone, "error");
  assert.equal(failed.notice?.title, "compile_failed");
  assert.match(failed.notice?.detail ?? "", /Git, network, or build/);

  const canceled = buildWorkspaceUpdateView(status({ status: "canceled" }), "zh");
  assert.equal(canceled.notice?.tone, "info");
  assert.equal(canceled.notice?.title, "编译已停止。");
});

test("explains oversized git path list failures", () => {
  const view = buildWorkspaceUpdateView(
    status({ status: "failed", error: "git path list output is too large to process safely" }),
    "zh",
  );
  assert.equal(view.notice?.tone, "error");
  assert.equal(view.notice?.title, "Git 文件列表过大，无法安全处理");
  assert.match(view.notice?.detail ?? "", /未跟踪文件太多/);
});

test("recognizes remote up-to-date status", () => {
  const view = buildWorkspaceUpdateView(
    status({ status: "idle", old_commit: "abc", remote_commit: "abc" }),
    "en",
  );
  assert.equal(view.knownUpToDate, true);
  assert.equal(view.displayStatus, "up_to_date");
  assert.equal(view.notice?.tone, "success");
});

test("formats log preview and timestamps", () => {
  const view = buildWorkspaceUpdateView(status({ stdout_tail: "ok", stderr_tail: "warn" }), "en");
  assert.equal(view.logPreview, "Build output\nok\n\nBuild log (stderr, not necessarily errors)\nwarn");
  assert.equal(formatWorkspaceUpdateTime(null, "en"), "--");
  assert.match(formatWorkspaceUpdateTime(1782197321, "en"), /2026|6|23/);
});
