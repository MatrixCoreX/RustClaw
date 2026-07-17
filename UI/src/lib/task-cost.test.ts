import assert from "node:assert/strict";
import test from "node:test";

import { buildTaskCostGovernance, formatUsdNanos } from "./task-cost";
import type { TaskQueryResponse } from "../types/api";

test("projects task cost and budget from structured journal fields", () => {
  const task: TaskQueryResponse = {
    task_id: "task-cost",
    status: "succeeded",
    result_json: {
      task_journal: {
        summary: {
          task_metrics: {
            llm_cost: {
              status: "unknown",
              estimated_cost_usd_nanos: 1_250_000,
              unknown_record_count: 1,
            },
            llm_cost_budget: {
              status: "soft_exceeded",
              enforcement: "checkpoint",
              task_known_cost_usd_nanos: 1_250_000,
              soft_task_limit_usd_nanos: 1_000_000,
              hard_task_limit_usd_nanos: 5_000_000,
              hard_exceeded: false,
              signals: ["soft_task_cost_exceeded"],
            },
          },
        },
      },
    },
  };

  assert.deepEqual(buildTaskCostGovernance(task), {
    costStatus: "unknown",
    budgetStatus: "soft_exceeded",
    enforcement: "checkpoint",
    estimatedCostUsdNanos: 1_250_000,
    taskKnownCostUsdNanos: 1_250_000,
    softTaskLimitUsdNanos: 1_000_000,
    hardTaskLimitUsdNanos: 5_000_000,
    unknownRecordCount: 1,
    hardExceeded: false,
    signals: ["soft_task_cost_exceeded"],
  });
  assert.equal(formatUsdNanos(1_250_000), "$0.001250");
});

test("returns no view when the task has no monetary cost contract", () => {
  assert.equal(
    buildTaskCostGovernance({
      task_id: "task-no-cost",
      status: "running",
      result_json: { task_journal: { summary: {} } },
    }),
    null,
  );
  assert.equal(formatUsdNanos(undefined), null);
});
