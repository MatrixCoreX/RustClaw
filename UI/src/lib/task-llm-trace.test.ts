import assert from "node:assert/strict";
import test from "node:test";

import { flowStageMachineTokens, flowSummaryMachineTokens } from "./task-llm-trace";

test("builds task flow summary machine tokens", () => {
  const tokens = flowSummaryMachineTokens({
    call_count: 4,
    stage_count: 3,
    retry_count: 1,
    verifier_call_count: 1,
    finalizer_call_count: 1,
    provider_error_count: 1,
    modules: ["planner", "finalizer"],
    status_counts: { ok: 3, error: 1 },
    trigger_counts: { normal: 2, json_retry: 1, provider_error: 1 },
    stages: [],
  });

  assert.deepEqual(tokens.slice(0, 7), [
    "call_count=4",
    "stage_count=3",
    "retry_count=1",
    "verifier_call_count=1",
    "finalizer_call_count=1",
    "provider_error_count=1",
    "module_count=2",
  ]);
  assert.ok(tokens.includes("status=ok:3"));
  assert.ok(tokens.includes("trigger=normal:2"));
});

test("builds stage-level flow machine tokens", () => {
  const tokens = flowStageMachineTokens({
    flow_stage: "agent_loop.planner",
    call_count: 2,
    provider_error_count: 0,
    prompt_labels: ["plan", "plan_repair"],
    flow_nodes: ["planner_round"],
    code_modules: ["crates/clawd/src/agent_engine/planning.rs"],
    code_entrypoints: ["plan_round_actions"],
    status_counts: { ok: 2 },
    trigger_counts: { normal: 1, json_retry: 1 },
  });

  assert.deepEqual(tokens.slice(0, 5), [
    "flow_stage=agent_loop.planner",
    "call_count=2",
    "provider_error_count=0",
    "prompt_label=plan",
    "prompt_label=plan_repair",
  ]);
  assert.ok(tokens.includes("flow_node=planner_round"));
  assert.ok(tokens.includes("trigger=json_retry:1"));
});
