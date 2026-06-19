import test from "node:test";
import assert from "node:assert/strict";

import { buildTaskLifecycleView } from "./task-lifecycle.ts";

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
  assert.ok(view.meta.some((item) => item === "Resume wait: 0s"));
  assert.ok(view.meta.some((item) => item === "Checkpoint: ckpt-ready"));
});

test("falls back to database status when lifecycle is absent", () => {
  const view = buildTaskLifecycleView(null, "canceled", "en");

  assert.equal(view.stateLabel, "Cancelled");
  assert.equal(view.tone, "failed");
  assert.equal(view.detail, "The task will not continue.");
});
