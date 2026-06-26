import test from "node:test";
import assert from "node:assert/strict";

import {
  buildReplaySummary,
  buildTaskOutcome,
  buildTaskPermissionView,
  extractTaskText,
  taskArtifactRefs,
  taskTraceEvents,
  traceEventMeta,
} from "./task-result.ts";
import type { TaskQueryResponse } from "../types/api.ts";

test("extracts visible task text before falling back to error text", () => {
  const result: TaskQueryResponse = {
    task_id: "task-1",
    status: "succeeded",
    result_json: { text: "dry_run=true\noutput_path=/tmp/a.png" },
    error_text: "ignored",
  };

  assert.equal(extractTaskText(result), "dry_run=true\noutput_path=/tmp/a.png");
});

test("builds completed task outcome from machine task_outcome fields", () => {
  const result: TaskQueryResponse = {
    task_id: "task-2",
    status: "succeeded",
    result_json: {
      task_journal: {
        summary: {
          final_status: "success",
          task_outcome: {
            state: "done",
            message_en: "Completed from machine state",
            next_step_en: "Review the result.",
          },
        },
        trace: {
          contract_matrix: {
            final_answer_shape: "generated_file_path_report",
          },
        },
      },
    },
  };

  const view = buildTaskOutcome(result, "en");

  assert.equal(view.title, "Completed from machine state");
  assert.equal(view.tone, "ok");
  assert.equal(view.nextStep, "Review the result.");
  assert.equal(view.finalShape, "generated_file_path_report");
});

test("builds permission view from structured decision fields", () => {
  const result: TaskQueryResponse = {
    task_id: "task-3",
    status: "failed",
    result_json: {
      permission_decision: {
        allowed: false,
        needs_confirmation: false,
        denied_by_policy: true,
        external_provider_blocked: true,
        risk_level: "high",
        owner_layer: "plan_verifier",
        status_code: "risk_budget_exceeded",
        message_key: "clawd.verify.risk_budget_exceeded",
      },
    },
  };

  const view = buildTaskPermissionView(result, "en");

  assert.equal(view?.tone, "failed");
  assert.equal(view?.title, "Permission decision");
  assert.ok(view?.meta.includes("allowed=false"));
  assert.ok(view?.meta.includes("external_provider_blocked=true"));
  assert.ok(view?.meta.includes("message_key=clawd.verify.risk_budget_exceeded"));
});

test("extracts trace events and stable machine meta", () => {
  const result: TaskQueryResponse = {
    task_id: "task-4",
    status: "running",
    result_json: {
      task_journal: {
        trace: {
          event_stream: [
            {
              seq: 1,
              event_type: "provider_call",
              payload: {
                prompt_label: "plan",
                llm_call_count: 1,
                child_run_summary: {
                  trace_merge_status: "merged",
                },
              },
            },
          ],
        },
      },
    },
  };

  const events = taskTraceEvents(result);
  assert.equal(events.length, 1);
  assert.deepEqual(traceEventMeta(events[0]), [
    "seq=1",
    "type=provider_call",
    "prompt_label=plan",
    "llm_call_count=1",
    "child_trace_merge_status=merged",
  ]);
});

test("collects artifact refs recursively without duplicate mirrored arrays", () => {
  const result: TaskQueryResponse = {
    task_id: "task-artifacts",
    status: "succeeded",
    result_json: {
      extra: {
        artifact_refs: [
          { ref: "artifact:summary", path: "out/summary.json", role: "summary" },
        ],
        artifacts: [
          { ref: "artifact:summary", path: "out/summary.json", role: "summary" },
          { output_path: "out/report.md", kind: "report" },
        ],
      },
    },
  };

  const refs = taskArtifactRefs(result);

  assert.equal(refs.length, 2);
  assert.equal(refs[0].summary, "ref=artifact:summary · path=out/summary.json · role=summary");
  assert.equal(refs[1].summary, "output_path=out/report.md · kind=report");
});

test("summarizes recorded replay machine fields", () => {
  const result: TaskQueryResponse = {
    task_id: "task-replay",
    status: "succeeded",
    result_json: {
      replay_mode: "recorded_only",
      result_source: "recorded_bundle",
      execution_replay: {
        strategy: "recorded_outputs_first",
        deterministic: true,
        live_provider: false,
        step_count: 4,
      },
      coverage: {
        has_task_checkpoint: true,
        event_types: ["route", "tool_result"],
      },
    },
  };

  const summary = buildReplaySummary(result);

  assert.ok(summary?.meta.includes("replay_mode=recorded_only"));
  assert.ok(summary?.meta.includes("strategy=recorded_outputs_first"));
  assert.ok(summary?.coverage.includes("has_task_checkpoint=true"));
  assert.ok(summary?.coverage.includes("event_types=route,tool_result"));
});
