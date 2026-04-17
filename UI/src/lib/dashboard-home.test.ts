import test from "node:test";
import assert from "node:assert/strict";

import {
  countCompletedDashboardSteps,
  getDashboardOverviewItems,
  getSuggestedDashboardAction,
} from "./dashboard-home.ts";

test("suggests models first when the llm is not configured", () => {
  assert.deepEqual(
    getSuggestedDashboardAction({
      isOnline: true,
      llmStepStatus: "todo",
      testMessageStepStatus: "todo",
      wechatStepStatus: "todo",
    }),
    { kind: "llm_setup", page: "models" },
  );
});

test("suggests restart when model changes are pending", () => {
  assert.deepEqual(
    getSuggestedDashboardAction({
      isOnline: true,
      llmStepStatus: "attention",
      testMessageStepStatus: "todo",
      wechatStepStatus: "todo",
    }),
    { kind: "llm_restart", page: "models" },
  );
});

test("suggests a test message after llm is ready", () => {
  assert.deepEqual(
    getSuggestedDashboardAction({
      isOnline: true,
      llmStepStatus: "done",
      testMessageStepStatus: "attention",
      wechatStepStatus: "todo",
    }),
    { kind: "chat_test", page: "chat" },
  );
});

test("suggests wechat after the test message is done", () => {
  assert.deepEqual(
    getSuggestedDashboardAction({
      isOnline: true,
      llmStepStatus: "done",
      testMessageStepStatus: "done",
      wechatStepStatus: "attention",
    }),
    { kind: "wechat_setup", page: "services" },
  );
});

test("suggests chat after llm and wechat are ready", () => {
  assert.deepEqual(
    getSuggestedDashboardAction({
      isOnline: true,
      llmStepStatus: "done",
      testMessageStepStatus: "done",
      wechatStepStatus: "done",
    }),
    { kind: "chat_test", page: "chat" },
  );
});

test("counts only completed steps", () => {
  assert.equal(countCompletedDashboardSteps(["done", "attention", "done"]), 2);
});

test("builds lightweight dashboard overview items", () => {
  assert.deepEqual(
    getDashboardOverviewItems({
      isOnline: true,
      memoryLabel: "128.00 MB",
      uptimeLabel: "3h 12m 4s",
    }),
    [
      {
        key: "status",
        label: "服务状态",
        value: "可访问",
        tone: "good",
      },
      {
        key: "memory",
        label: "内存占用",
        value: "128.00 MB",
        tone: "neutral",
      },
      {
        key: "uptime",
        label: "运行时长",
        value: "3h 12m 4s",
        tone: "neutral",
      },
    ],
  );
});
