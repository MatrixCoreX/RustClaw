import test from "node:test";
import assert from "node:assert/strict";

import {
  areRequiredDashboardStepsComplete,
  countCompletedDashboardSteps,
  getDefaultDashboardSection,
  getDashboardOverviewItems,
  getSuggestedDashboardAction,
  isDashboardCategoryPage,
} from "./dashboard-home.ts";

test("keeps model, communication setup, and tools under home categories", () => {
  assert.equal(isDashboardCategoryPage("models"), true);
  assert.equal(isDashboardCategoryPage("services"), true);
  assert.equal(isDashboardCategoryPage("skills"), true);
  assert.equal(isDashboardCategoryPage("skill_store"), false);
});

test("opens quick setup until required setup is complete", () => {
  assert.equal(
    getDefaultDashboardSection([{ required: true, status: "attention" }]),
    "setup",
  );
  assert.equal(
    getDefaultDashboardSection([
      { required: true, status: "done" },
      { required: false, status: "todo" },
    ]),
    "overview",
  );
});

test("considers only required onboarding steps when deciding whether setup is complete", () => {
  assert.equal(
    areRequiredDashboardStepsComplete([
      { required: true, status: "done" },
      { required: true, status: "done" },
      { required: false, status: "todo" },
    ]),
    true,
  );
  assert.equal(
    areRequiredDashboardStepsComplete([
      { required: true, status: "done" },
      { required: true, status: "attention" },
      { required: false, status: "done" },
    ]),
    false,
  );
  assert.equal(areRequiredDashboardStepsComplete([{ required: false, status: "done" }]), false);
});

test("collapses after model setup while message and communication stay optional", () => {
  assert.equal(
    areRequiredDashboardStepsComplete([
      { required: true, status: "done" },
      { required: false, status: "attention" },
      { required: false, status: "todo" },
    ]),
    true,
  );
  assert.equal(
    areRequiredDashboardStepsComplete([
      { required: true, status: "attention" },
      { required: false, status: "done" },
      { required: false, status: "done" },
    ]),
    false,
  );
});

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
