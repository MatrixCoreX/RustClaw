import test from "node:test";
import assert from "node:assert/strict";

import {
  buildReplaySummary,
  buildTaskOutcome,
  buildTaskPermissionView,
  buildTaskTraceEventView,
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

test("extracts task lifecycle event meta for UI progress cards", () => {
  const result: TaskQueryResponse = {
    task_id: "task-events",
    status: "running",
    result_json: {
      task_journal: {
        trace: {
          event_stream: [
            {
              seq: 1,
              event_type: "task_transition",
              payload: {
                task_id: "task-events",
                transition_ref: "task_transition:1",
                evidence_ref: "task_transition:1",
                state_from: "executing",
                state_to: "finalizing",
                reason_code: "agent_loop_ready_to_finalize",
                round_no: 2,
                at_ms: 1781800001000,
              },
            },
            {
              seq: 2,
              event_type: "checkpoint_created",
              payload: {
                checkpoint_id: "ckpt-1",
                checkpoint_ref: "task_checkpoint:ckpt-1",
                evidence_ref: "task_checkpoint:ckpt-1",
                completed_side_effect_count: 1,
                pending_async_job_id: "job-1",
                poll_ref: "local_process:123",
                cancel_ref: "local_process:123",
                message_key: "async_job_running",
              },
            },
            {
              seq: 3,
              event_type: "tool_started",
              payload: {
                phase: "started",
                step_id: "step_1",
                step_ref: "step_1",
                evidence_ref: "step_1",
                skill: "run_cmd",
                requested_capability: "terminal.run_command",
                started_at: 1781800002000,
              },
            },
            {
              seq: 4,
              event_type: "tool_finished",
              payload: {
                phase: "finished",
                step_id: "step_1",
                step_ref: "step_1",
                evidence_ref: "step_1",
                skill: "run_cmd",
                status: "ok",
                finished_at: 1781800003000,
              },
            },
            {
              seq: 5,
              event_type: "coding_evidence",
              payload: {
                evidence_ref: "coding_evidence:summary",
                changed_file_count: 1,
                command_count: 2,
                verification_command_count: 2,
                test_count: 1,
                diff_summary_count: 1,
                failure_count: 1,
                retry_count: 1,
                unverified_risk: "tests_not_observed",
              },
            },
          ],
        },
      },
    },
  };

  const events = taskTraceEvents(result);

  assert.ok(traceEventMeta(events[0]).includes("transition_ref=task_transition:1"));
  assert.ok(traceEventMeta(events[0]).includes("state_to=finalizing"));
  assert.ok(traceEventMeta(events[1]).includes("checkpoint_ref=task_checkpoint:ckpt-1"));
  assert.ok(traceEventMeta(events[1]).includes("pending_async_job_id=job-1"));
  assert.ok(traceEventMeta(events[2]).includes("phase=started"));
  assert.ok(traceEventMeta(events[2]).includes("started_at=1781800002000"));
  assert.ok(traceEventMeta(events[3]).includes("phase=finished"));
  assert.ok(traceEventMeta(events[3]).includes("finished_at=1781800003000"));
  assert.ok(traceEventMeta(events[4]).includes("changed_file_count=1"));
  assert.ok(traceEventMeta(events[4]).includes("verification_command_count=2"));
  assert.ok(traceEventMeta(events[4]).includes("test_count=1"));
  assert.ok(traceEventMeta(events[4]).includes("retry_count=1"));
  assert.ok(traceEventMeta(events[4]).includes("unverified_risk=tests_not_observed"));
  assert.equal(buildTaskTraceEventView(events[1], "en").title, "Checkpoint saved");
  assert.equal(buildTaskTraceEventView(events[1], "en").tone, "attention");
  assert.equal(buildTaskTraceEventView(events[2], "en").detail, "run_cmd is running.");
  assert.equal(buildTaskTraceEventView(events[3], "zh").title, "工具执行结束");
  assert.equal(buildTaskTraceEventView(events[4], "en").tone, "failed");
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
