import assert from "node:assert/strict";
import test from "node:test";

import {
  taskLlmDebugCallEntry,
  taskLlmDebugCallMetaTokens,
  taskLlmDebugRawFields,
  taskLlmDebugRequestData,
  taskLlmDebugResponseData,
  taskLlmTraceAvailabilityStatus,
} from "./task-llm-debug-display";

test("reads teaching trace payloads from entry-wrapped backend calls", () => {
  const request = { messages: [{ role: "user", content: "plan" }] };
  const call = {
    call_index: 1,
    flow: { flow_stage: "agent_loop.planner" },
    entry: {
      ts: 10,
      call_id: "task-1:planner",
      status: "ok",
      model: "MiniMax-M3",
      prompt_source: "layered:planner",
      request_payload: request,
      raw_response: "{\"choices\":[]}",
      usage: { prompt_tokens: 11, completion_tokens: 7, total_tokens: 18 },
    },
  };

  assert.equal(taskLlmDebugCallEntry(call).call_id, "task-1:planner");
  assert.deepEqual(taskLlmDebugRequestData(call), request);
  assert.equal(taskLlmDebugResponseData(call), "{\"choices\":[]}");
  assert.ok(taskLlmDebugCallMetaTokens(call).includes("status=ok"));
  assert.ok(taskLlmDebugCallMetaTokens(call).includes("prompt=11"));
  assert.ok(taskLlmDebugRawFields(call).includes("entry.request_payload"));
});

test("keeps compatibility with flat teaching trace calls", () => {
  const call = {
    call_index: 2,
    status: "error",
    prompt: "verify answer",
    error: "provider_timeout",
  };

  assert.equal(taskLlmDebugRequestData(call), "verify answer");
  assert.equal(taskLlmDebugResponseData(call), "provider_timeout");
  assert.ok(taskLlmDebugCallMetaTokens(call).includes("status=error"));
  assert.ok(taskLlmDebugRawFields(call).includes("prompt"));
});

test("projects structured teaching trace availability across refresh recovery states", () => {
  assert.equal(
    taskLlmTraceAvailabilityStatus({
      task_id: "task-available",
      trace_availability: { status: "available" },
    }),
    "available",
  );
  assert.equal(
    taskLlmTraceAvailabilityStatus({
      task_id: "task-metadata",
      trace_availability: { status: "metadata_only" },
    }),
    "metadata_only",
  );
  assert.equal(
    taskLlmTraceAvailabilityStatus({
      task_id: "task-pending",
      trace_availability: { status: "pending" },
    }),
    "pending",
  );
  assert.equal(
    taskLlmTraceAvailabilityStatus({
      task_id: "task-expired",
      trace_availability: {
        status: "unavailable",
        reason_code: "provider_io_not_recorded_or_expired",
      },
    }),
    "unavailable",
  );
  assert.equal(
    taskLlmTraceAvailabilityStatus({
      task_id: "legacy-task",
      calls: [{ call_index: 1 }],
    }),
    "available",
  );
});
