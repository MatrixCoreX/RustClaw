import assert from "node:assert/strict";
import test from "node:test";

import {
  type FeishuBindSessionResponse,
  getFeishuBindStatusCopy,
  getFeishuStepStatus,
  getFeishuSetupGuidance,
  isFeishuBindTerminalStatus,
} from "./feishu-bind";

test("waiting status copy guides user to scan", () => {
  const copy = getFeishuBindStatusCopy("pending");
  assert.equal(copy.tone, "pending");
  assert.equal(copy.zhLabel, "等待扫码");
  assert.match(copy.zhDescription, /扫码|飞书/);
});

test("detected status copy confirms account recognition", () => {
  const copy = getFeishuBindStatusCopy("detected");
  assert.equal(copy.tone, "attention");
  assert.equal(copy.enLabel, "Recognized");
  assert.match(copy.enDescription, /recognized|binding/i);
});

test("bound status copy is terminal and successful", () => {
  const copy = getFeishuBindStatusCopy("bound");
  assert.equal(copy.tone, "success");
  assert.equal(copy.zhLabel, "绑定成功");
  assert.equal(isFeishuBindTerminalStatus("bound"), true);
});

test("failed and expired statuses are terminal", () => {
  const failed = getFeishuBindStatusCopy("failed");
  const expired = getFeishuBindStatusCopy("expired");
  assert.equal(failed.tone, "error");
  assert.equal(expired.tone, "error");
  assert.equal(isFeishuBindTerminalStatus("failed"), true);
  assert.equal(isFeishuBindTerminalStatus("expired"), true);
});

test("setup guidance lets users scan to configure when deployment config is missing", () => {
  const guidance = getFeishuSetupGuidance({
    bindReady: false,
    hasUnsavedConfigChanges: false,
    serviceHealthy: false,
    hasActiveSession: false,
    bound: false,
  });
  assert.equal(guidance.status, "scan_to_setup");
  assert.match(guidance.zhSummary, /二维码|扫码|自动/);
  assert.equal(guidance.canStartService, false);
  assert.equal(guidance.canStartBind, true);
});

test("setup guidance lets users start service after deployment is ready", () => {
  const guidance = getFeishuSetupGuidance({
    bindReady: true,
    hasUnsavedConfigChanges: false,
    serviceHealthy: false,
    hasActiveSession: false,
    bound: false,
  });
  assert.equal(guidance.status, "ready_to_start");
  assert.equal(guidance.canStartService, true);
  assert.equal(guidance.canStartBind, false);
});

test("failed session does not keep the top-level step in progress", () => {
  const session: FeishuBindSessionResponse = {
    session_id: 1,
    channel: "feishu",
    bind_token: "bind-token",
    status: "failed",
    created_at: "2026-04-02T12:00:00Z",
    updated_at: "2026-04-02T12:00:05Z",
    expires_at: "2026-04-02T12:10:00Z",
    error_text: "service failed",
  };
  assert.equal(
    getFeishuStepStatus({
      bindReady: false,
      serviceHealthy: false,
      session,
      currentKeyBound: false,
    }),
    "todo",
  );
});
