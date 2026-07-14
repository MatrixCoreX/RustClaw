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
            done_conditions: ["tests_pass"],
            constraints: [{ scope: "workspace", writes_allowed: true }],
            verification: { command: "cargo test -p clawd" },
            current_progress: ["changed_file_count=1"],
            remaining_work: ["summarize"],
          },
        },
        trace: {
          contract_matrix: {
            final_answer_shape: "generated_file_path_report",
          },
          event_stream: [
            {
              event_type: "coding_evidence",
              payload: {
                current_phase_hint: "summarize",
                changed_file_count: 1,
                command_count: 2,
                test_count: 1,
                verification_command_count: 1,
                verification_status: "verified",
              },
            },
          ],
        },
      },
    },
  };

  const view = buildTaskOutcome(result, "en");

  assert.equal(view.title, "Completed from machine state");
  assert.equal(view.tone, "ok");
  assert.equal(view.nextStep, "Review the result.");
  assert.equal(view.finalShape, "generated_file_path_report");
  assert.deepEqual(view.doneConditions, ["tests_pass"]);
  assert.ok(view.constraints.includes("scope=workspace"));
  assert.ok(view.constraints.includes("writes_allowed=true"));
  assert.ok(view.verification.includes("command=cargo test -p clawd"));
  assert.ok(view.verification.includes("verification_status=verified"));
  assert.ok(view.currentProgress.includes("changed_file_count=1"));
  assert.ok(view.remainingWork.includes("summarize"));
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
        steps: [
          {
            step_id: "step_1",
            action_type: "call_skill",
            executable: true,
            decision: "require_confirmation",
            skill: "run_cmd",
            action: "execute",
            action_effect: {
              observes: false,
              mutates: true,
              validates: true,
            },
            risk_level: "high",
            requires_confirmation: true,
            sandbox_profile: "local_temp_workspace",
            sandbox: {
              profile: "local_temp_workspace",
              source: "registry_capability_policy",
              filesystem_write: true,
              network_access: false,
              external_publish: false,
              credential_access: false,
            },
            workspace_scope: {
              scope: "workspace_scoped",
              path_arg_count: 1,
              cwd_present: true,
              untrusted_path_present: false,
              external_workspace: false,
            },
            registry_policy: {
              capability: "terminal.run_command",
              effect: "mutate",
              risk_level: "high",
              isolation_profile: "local_temp_workspace",
              filesystem_write: true,
              network_access: false,
              once_per_task: true,
              dedup_scope: "args",
              idempotent: false,
            },
          },
        ],
      },
    },
  };

  const view = buildTaskPermissionView(result, "en");

  assert.equal(view?.tone, "failed");
  assert.equal(view?.title, "Permission decision");
  assert.ok(view?.meta.includes("allowed=false"));
  assert.ok(view?.meta.includes("external_provider_blocked=true"));
  assert.ok(view?.meta.includes("message_key=clawd.verify.risk_budget_exceeded"));
  assert.equal(view?.steps.length, 1);
  assert.ok(view?.steps[0].meta.includes("skill=run_cmd"));
  assert.ok(view?.steps[0].meta.includes("effect.mutates=true"));
  assert.ok(view?.steps[0].sandbox.includes("profile=local_temp_workspace"));
  assert.ok(view?.steps[0].workspace.includes("scope=workspace_scoped"));
  assert.ok(view?.steps[0].registryPolicy.includes("capability=terminal.run_command"));
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
                requires_idempotency_guard: true,
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
              event_type: "coding_checkpoint",
              payload: {
                checkpoint_kind: "verification_command",
                checkpoint_ref: "coding_checkpoint:verification_command:1",
                evidence_ref: "coding_checkpoint:verification_command:1",
                command_index: 1,
                verification_command: "cargo test -p clawd",
                verification_command_count: 2,
                verification_status: "failed",
                verification_failure_kind_count: 1,
              },
            },
            {
              seq: 6,
              event_type: "coding_task_contract",
              payload: {
                contract_ref: "coding_task_contract:summary",
                files_read_count: 1,
                files_changed_count: 1,
                commands_run_count: 2,
                tests_run_count: 1,
                verification_command_count: 2,
                verification_status: "failed",
                retry_count: 1,
              },
            },
            {
              seq: 7,
              event_type: "coding_evidence",
              payload: {
                evidence_ref: "coding_evidence:summary",
                changed_file_count: 1,
                command_count: 2,
                verification_command_count: 2,
                test_count: 1,
                diff_summary_count: 1,
                failure_count: 1,
                verification_status: "failed",
                verification_failure_kind_count: 1,
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
  assert.ok(traceEventMeta(events[1]).includes("requires_idempotency_guard=true"));
  assert.ok(traceEventMeta(events[2]).includes("phase=started"));
  assert.ok(traceEventMeta(events[2]).includes("started_at=1781800002000"));
  assert.ok(traceEventMeta(events[3]).includes("phase=finished"));
  assert.ok(traceEventMeta(events[3]).includes("finished_at=1781800003000"));
  assert.ok(traceEventMeta(events[4]).includes("checkpoint_kind=verification_command"));
  assert.ok(traceEventMeta(events[4]).includes("command_index=1"));
  assert.ok(traceEventMeta(events[4]).includes("verification_command=cargo test -p clawd"));
  assert.ok(traceEventMeta(events[5]).includes("contract_ref=coding_task_contract:summary"));
  assert.ok(traceEventMeta(events[5]).includes("files_read_count=1"));
  assert.ok(traceEventMeta(events[5]).includes("files_changed_count=1"));
  assert.ok(traceEventMeta(events[5]).includes("commands_run_count=2"));
  assert.ok(traceEventMeta(events[5]).includes("tests_run_count=1"));
  assert.ok(traceEventMeta(events[6]).includes("changed_file_count=1"));
  assert.ok(traceEventMeta(events[6]).includes("verification_command_count=2"));
  assert.ok(traceEventMeta(events[6]).includes("test_count=1"));
  assert.ok(traceEventMeta(events[6]).includes("verification_status=failed"));
  assert.ok(traceEventMeta(events[6]).includes("verification_failure_kind_count=1"));
  assert.ok(traceEventMeta(events[6]).includes("retry_count=1"));
  assert.ok(traceEventMeta(events[6]).includes("unverified_risk=tests_not_observed"));
  assert.equal(buildTaskTraceEventView(events[1], "en").title, "Checkpoint saved");
  assert.equal(buildTaskTraceEventView(events[1], "en").tone, "attention");
  assert.equal(buildTaskTraceEventView(events[2], "en").detail, "run_cmd is running.");
  assert.equal(buildTaskTraceEventView(events[3], "zh").title, "工具执行结束");
  assert.equal(buildTaskTraceEventView(events[4], "en").title, "Verification checkpoint");
  assert.equal(
    buildTaskTraceEventView(events[4], "en").detail,
    "Verification command: cargo test -p clawd",
  );
  assert.equal(buildTaskTraceEventView(events[5], "en").title, "Coding task contract");
  assert.equal(
    buildTaskTraceEventView(events[5], "en").detail,
    "1 file(s) read, 1 changed file(s), 2 command(s), 1 test record(s).",
  );
  assert.equal(
    buildTaskTraceEventView(events[6], "en").detail,
    "1 changed file(s), 2 verification command(s), 1 test record(s).",
  );
  assert.equal(buildTaskTraceEventView(events[6], "en").tone, "failed");
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
