import assert from "node:assert/strict";
import test from "node:test";

import {
  agentFlowPhaseToken,
  agentFlowTimelineRows,
  flowStageMachineTokens,
  flowSummaryMachineTokens,
  modelCatalogTraceMachineTokens,
  resumeTraceMachineTokens,
} from "./task-llm-trace";

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

test("builds classroom timeline rows in agent-loop order", () => {
  const rows = agentFlowTimelineRows({
    task_id: "task-1",
    flow_summary: {
      call_count: 4,
      stage_count: 4,
      retry_count: 0,
      verifier_call_count: 1,
      finalizer_call_count: 1,
      provider_error_count: 0,
      modules: [],
      status_counts: {},
      trigger_counts: {},
      stages: [
        {
          flow_stage: "finalizer.response_composer",
          call_count: 1,
          prompt_labels: ["user_response_composer"],
          flow_nodes: ["user_response_composer"],
          code_modules: ["crates/clawd/src/fallback.rs"],
          code_entrypoints: ["compose_user_response_from_contract_impl"],
          trigger_counts: { normal: 1 },
          status_counts: { ok: 1 },
          provider_error_count: 0,
        },
        {
          flow_stage: "agent_loop.planner",
          call_count: 1,
          prompt_labels: ["plan"],
          flow_nodes: ["planner_round"],
          code_modules: ["crates/clawd/src/agent_engine/planning.rs"],
          code_entrypoints: ["plan_round_actions"],
          trigger_counts: { normal: 1 },
          status_counts: { ok: 1 },
          provider_error_count: 0,
        },
        {
          flow_stage: "boundary.normalizer",
          call_count: 1,
          prompt_labels: ["normalizer"],
          flow_nodes: ["intent_normalizer"],
          code_modules: ["crates/clawd/src/intent_router_normalizer_model.rs"],
          code_entrypoints: ["run_intent_normalizer_model_step"],
          trigger_counts: { normal: 1 },
          status_counts: { ok: 1 },
          provider_error_count: 0,
        },
        {
          flow_stage: "agent_loop.answer_verifier",
          call_count: 1,
          prompt_labels: ["verifier"],
          flow_nodes: ["answer_verifier"],
          code_modules: ["crates/clawd/src/answer_verifier_runtime.rs"],
          code_entrypoints: ["verify_answer_observe_only"],
          trigger_counts: { normal: 1 },
          status_counts: { ok: 1 },
          provider_error_count: 0,
        },
      ],
    },
    calls: [
      { call_index: 1, flow: { flow_stage: "boundary.normalizer" } },
      { call_index: 2, flow: { flow_stage: "agent_loop.planner" } },
      { call_index: 3, flow: { flow_stage: "agent_loop.answer_verifier" } },
      { call_index: 4, flow: { flow_stage: "finalizer.response_composer" } },
    ],
  });

  assert.deepEqual(
    rows.map((row) => row.flowStage),
    [
      "boundary.normalizer",
      "agent_loop.planner",
      "agent_loop.answer_verifier",
      "finalizer.response_composer",
    ],
  );
  assert.deepEqual(rows.map((row) => row.callIndexes), [[1], [2], [3], [4]]);
  assert.deepEqual(rows.map((row) => row.phaseToken), [
    "boundary",
    "planner",
    "answer_verifier",
    "finalizer",
  ]);
});

test("classifies unknown flow stages as provider fallback phase", () => {
  assert.equal(agentFlowPhaseToken("provider.llm_call"), "provider");
  assert.equal(agentFlowPhaseToken("custom.future_stage"), "provider");
});

test("builds model catalog trace machine tokens", () => {
  const tokens = modelCatalogTraceMachineTokens({
    trace_kind: "model_catalog_decision",
    status: "ok",
    selected_provider: "minimax",
    selected_model: "MiniMax-M3",
    observed_provider_count: 1,
    entry_count: 1,
    catalog_guard_status: {
      status: "ok",
      finding_count: 0,
    },
  });

  assert.ok(tokens.includes("trace_kind=model_catalog_decision"));
  assert.ok(tokens.includes("selected_provider=minimax"));
  assert.ok(tokens.includes("selected_model=MiniMax-M3"));
  assert.ok(tokens.includes("catalog_guard_status.status=ok"));
});

test("builds resume trace machine tokens", () => {
  const tokens = resumeTraceMachineTokens({
    trace_kind: "task_resume_decision",
    state: "waiting",
    execution_state: "waiting",
    checkpoint_id: "ckpt-1",
    resume_entrypoint: "next_planner_round",
    resume_due: false,
    resume_wait_seconds: 30,
    recommended_user_action_kind: "wait_until_next_check",
    completed_side_effect_count: 1,
    requires_idempotency_guard: true,
    provider_blocker_status_code: "provider_rate_limited",
  });

  assert.ok(tokens.includes("trace_kind=task_resume_decision"));
  assert.ok(tokens.includes("state=waiting"));
  assert.ok(tokens.includes("checkpoint_id=ckpt-1"));
  assert.ok(tokens.includes("requires_idempotency_guard=true"));
  assert.ok(tokens.includes("provider_blocker_status_code=provider_rate_limited"));
});
