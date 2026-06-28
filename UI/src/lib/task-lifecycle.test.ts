import test from "node:test";
import assert from "node:assert/strict";

import { buildTaskLifecycleView, buildTaskPollingView, buildTaskStatusSummary } from "./task-lifecycle.ts";

test("builds a pollable running lifecycle view", () => {
  const view = buildTaskLifecycleView(
    {
      state: "running",
      can_poll: true,
      can_cancel: true,
      last_heartbeat_ts: 1781796641,
    },
    "running",
    "en",
  );

  assert.equal(view.stateLabel, "Running");
  assert.equal(view.tone, "running");
  assert.match(view.detail, /refresh/i);
  assert.equal(view.recommendedAction.actionKind, "running");
  assert.equal(view.recommendedAction.label, "Keep monitoring");
  assert.ok(view.meta.some((item) => item === "Pollable: Yes"));
  assert.ok(view.meta.some((item) => item === "Cancelable: Yes"));
  assert.ok(view.meta.some((item) => item.startsWith("Last heartbeat:")));
});

test("surfaces waiting checkpoint details without raw json", () => {
  const view = buildTaskLifecycleView(
    {
      state: "waiting",
      can_poll: true,
      can_cancel: true,
      resume_reason: "provider_gap_retry_window",
      next_check_after: 1781800300,
      resume_due: false,
      resume_wait_seconds: 120,
      checkpoint_id: "ckpt-1",
      pending_job_ref: "job-1",
    },
    "running",
    "zh",
  );

  assert.equal(view.stateLabel, "等待中");
  assert.equal(view.tone, "attention");
  assert.equal(view.detail, "恢复原因: provider_gap_retry_window");
  assert.equal(view.recommendedAction.label, "保持等待");
  assert.ok(view.meta.some((item) => item === "恢复等待: 120s"));
  assert.ok(view.meta.some((item) => item === "检查点: ckpt-1"));
  assert.ok(view.meta.some((item) => item === "后台任务: job-1"));
});

test("surfaces due resume window without exposing checkpoint json", () => {
  const view = buildTaskLifecycleView(
    {
      state: "background",
      can_poll: true,
      can_cancel: true,
      resume_due: true,
      resume_wait_seconds: 0,
      checkpoint_id: "ckpt-ready",
    },
    "running",
    "en",
  );

  assert.equal(view.stateLabel, "Background");
  assert.equal(view.detail, "The resume window is due and the system can continue.");
  assert.equal(view.recommendedAction.label, "Ready to resume");
  assert.ok(view.meta.some((item) => item === "Resume wait: 0s"));
  assert.ok(view.meta.some((item) => item === "Checkpoint: ckpt-ready"));
});

test("uses next action fields without exposing them as primary meta", () => {
  const view = buildTaskLifecycleView(
    {
      state: "background",
      can_poll: true,
      can_cancel: true,
      last_heartbeat_ts: 1781796641,
      next_check_after: 1781800300,
      waiting_reason_code: "provider_backoff",
      next_action_kind: "poll_async_job",
      next_action_ref: "job-9",
      resume_wait_seconds: 45,
      checkpoint_id: "ckpt-9",
    },
    "running",
    "en",
  );

  assert.deepEqual(view.meta.slice(0, 2), ["Wait reason: provider_backoff", "Resume wait: 45s"]);
  assert.ok(view.meta.every((item) => !item.includes("poll_async_job")));
  assert.ok(view.meta.every((item) => !item.includes("Next action ref")));
  assert.ok(view.meta.some((item) => item === "Checkpoint: ckpt-9"));
  assert.ok(view.meta.some((item) => item === "Pollable: Yes"));
  assert.ok(view.meta.some((item) => item === "Cancelable: Yes"));
  assert.equal(view.recommendedAction.actionKind, "poll_async_job");
  assert.equal(view.recommendedAction.label, "Waiting for background result");
});

test("falls back to database status when lifecycle is absent", () => {
  const view = buildTaskLifecycleView(null, "canceled", "en");

  assert.equal(view.stateLabel, "Cancelled");
  assert.equal(view.tone, "failed");
  assert.equal(view.detail, "The task will not continue.");
  assert.equal(view.recommendedAction.label, "Stopped");
});

test("renders timeout as a clear stopped state", () => {
  const view = buildTaskLifecycleView({ terminal_reason: "worker_task_timeout" }, "timeout", "en");

  assert.equal(view.stateLabel, "Timed out");
  assert.equal(view.tone, "failed");
  assert.equal(view.recommendedAction.actionKind, "timeout");
  assert.equal(view.recommendedAction.label, "Review reason");
});

test("summarizes task states for dashboard cards", () => {
  const summary = buildTaskStatusSummary(
    [
      { status: "queued" },
      { status: "running", lifecycle: { state: "background" } },
      { status: "running", lifecycle: { state: "waiting" } },
      { status: "running", lifecycle: { state: "needs_user" } },
      { status: "failed" },
      { status: "running", lifecycle: { state: "canceled" } },
    ],
    "en",
  );

  assert.deepEqual(
    summary.map((item) => [item.kind, item.count]),
    [
      ["active", 2],
      ["waiting", 1],
      ["needs_user", 1],
      ["failed", 2],
    ],
  );
});

test("builds async polling hints from machine lifecycle fields", () => {
  const view = buildTaskPollingView(
    {
      state: "background",
      can_poll: true,
      can_cancel: true,
      pending_job_ref: "job-123",
      poll_ref: "poll-123",
      next_poll_after: 1_800_000_000,
      cancel_ref: "cancel-123",
    },
    "en",
  );

  assert.ok(view);
  assert.equal(
    view.detail,
    "This task can wait in the background and continue polling through machine fields.",
  );
  assert.ok(view.meta.includes("Background job: job-123"));
  assert.ok(view.meta.includes("Poll ref: poll-123"));
  assert.ok(view.meta.includes("Pollable: Yes"));
  assert.ok(view.meta.includes("Cancelable: Yes"));
  assert.ok(view.meta.includes("Cancel ref: cancel-123"));
});
