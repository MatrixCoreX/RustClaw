import assert from "node:assert/strict";
import test from "node:test";
import { buildTaskStatusFacts } from "./task-status-facts";
import { buildTaskOutcome } from "./task-result";
import type { TaskQueryResponse } from "../types/api";

function task(verification: string, delivery: string): TaskQueryResponse {
  return { task_id: "fixture", status: "running", result_json: { task_journal: { summary: {
    execution_status: "succeeded", answer_verification: { status: verification, retryable: true },
    delivery: { schema_version: 1, status: delivery },
  } } } };
}

test("execution, verification and delivery remain independent facts", () => {
  const view = buildTaskStatusFacts(task("unavailable", "partial"), "en");
  assert.deepEqual(view.map(item => item.status), ["succeeded", "unavailable", "partial"]);
  assert.equal(view[0].tone, "ok");
  assert.equal(view[1].tone, "attention");
  assert.equal(view[2].text, "Partially accepted");
});

test("legacy missing fields never fabricate verification success", () => {
  assert.deepEqual(buildTaskStatusFacts({ task_id: "old", status: "succeeded" }, "zh"), []);
  assert.deepEqual(buildTaskStatusFacts(task("unknown-state", "<script>"), "en").map(v => v.kind), ["execution"]);
});

test("received is not inferred from overall task success", () => {
  const input = task("passed", "query_required");
  input.status = "succeeded";
  const view = buildTaskStatusFacts(input, "zh");
  assert.equal(view[2].text, "需要核查");
  assert.equal(view[2].tone, "attention");
});

test("successful execution with verifier outage is not presented as execution failure", () => {
  const view = buildTaskOutcome(task("unavailable", "pending"), "en");
  assert.equal(view.title, "Execution completed; verification pending");
  assert.equal(view.tone, "running");
});

test("a canceled task is not presented as an automatic verification retry", () => {
  const input = task("unavailable", "pending");
  input.status = "canceled";
  assert.notEqual(buildTaskOutcome(input, "en").title, "Execution completed; verification pending");
});

test("rejected answer verification does not rewrite completed execution", () => {
  const input = task("failed", "not_required");
  input.status = "failed";
  const facts = buildTaskStatusFacts(input, "en");
  assert.deepEqual(facts.map(fact => fact.status), ["succeeded", "failed", "not_required"]);
  assert.equal(facts[0].text, "Completed");
  assert.equal(facts[1].text, "Not passed");
});

test("manual receipt confirmation is visibly distinct from provider delivery", () => {
  const facts = buildTaskStatusFacts(task("passed", "user_confirmed"), "en");
  assert.equal(facts[2].text, "Confirmed received by you");
  assert.notEqual(facts[2].text, "Delivered");
});
