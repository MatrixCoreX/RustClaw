import test from "node:test";
import assert from "node:assert/strict";

import {
  appendLiveTaskEvent,
  buildReplaySummary,
  buildTaskApprovalRequest,
  buildTaskGoalView,
  buildTaskOutcome,
  buildTaskPermissionView,
  buildTaskTraceEventView,
  extractTaskText,
  taskArtifactRefs,
  taskTraceEvents,
  traceEventMeta,
} from "./task-result.ts";
import type { TaskQueryResponse } from "../types/api.ts";

test("appends progressive model events into the live task trace", () => {
  const first = appendLiveTaskEvent(null, "task-live", {
    schema_version: 1,
    seq: 1,
    task_id: "task-live",
    event_kind: "model_turn",
    payload: {
      type: "tool_call_delta",
      model_event_index: 2,
      tool_name: "call_capability",
      arguments_delta_bytes: 12,
    },
  });
  const duplicate = appendLiveTaskEvent(first, "task-live", {
    schema_version: 1,
    seq: 1,
    task_id: "task-live",
    event_kind: "model_turn",
    payload: { type: "tool_call_delta" },
  });

  const events = taskTraceEvents(duplicate);
  assert.equal(events.length, 1);
  assert.equal(events[0].event_type, "model_turn");
  assert.equal(
    events[0].payload && (events[0].payload as Record<string, unknown>).tool_name,
    "call_capability",
  );
});

test("renders safe model turn lifecycle fields", () => {
  const view = buildTaskTraceEventView(
    {
      event_type: "model_turn",
      payload: {
        type: "tool_call",
        provider: "vendor-minimax:MiniMax-M3",
        tool_name: "call_capability",
      },
    },
    "en",
  );

  assert.equal(view.title, "Model turn");
  assert.equal(view.detail, "The model selected call_capability.");
  assert.ok(view.meta.includes("provider=vendor-minimax:MiniMax-M3"));
  assert.ok(view.meta.includes("tool_name=call_capability"));
});

test("renders a live tool event before the persisted step completes", () => {
  const view = buildTaskTraceEventView(
    {
      event_type: "tool_active",
      payload: {
        phase: "active",
        round_no: 2,
        step_in_round: 1,
        global_step: 3,
        action_kind: "call_capability",
        action_ref: "terminal.run_command",
        requested_capability: "terminal.run_command",
        status: "running",
      },
    },
    "en",
  );

  assert.equal(view.title, "Tool active");
  assert.equal(view.detail, "terminal.run_command is running.");
  assert.equal(view.tone, "running");
  assert.ok(view.meta.includes("action_kind=call_capability"));
  assert.ok(view.meta.includes("action_ref=terminal.run_command"));
});

test("summarizes persisted subagent graph events", () => {
  const graphView = buildTaskTraceEventView(
    {
      event_type: "subagent_graph",
      payload: {
        schema_version: 1,
        status: "active",
        nodes: [
          { child_task_id: "child-1", readiness: "running" },
          { child_task_id: "child-2", readiness: "blocked_dependency" },
        ],
        edges: [
          {
            predecessor_task_id: "child-1",
            successor_task_id: "child-2",
          },
        ],
      },
    },
    "en",
  );
  assert.equal(graphView.title, "Subagent task graph");
  assert.equal(graphView.detail, "2 node(s), 1 dependency edge(s); status active.");

  const nodeView = buildTaskTraceEventView(
    {
      event_type: "subagent_node",
      payload: {
        child_task_id: "child-2",
        graph: {
          nodes: [
            { child_task_id: "child-2", readiness: "failed" },
          ],
        },
      },
    },
    "en",
  );
  assert.equal(nodeView.title, "Subagent task node");
  assert.equal(nodeView.detail, "child-2 · failed");
  assert.equal(nodeView.tone, "failed");

  const steeringView = buildTaskTraceEventView(
    {
      event_type: "subagent_steering",
      payload: {
        parent_task_id: "parent-1",
        child_task_id: "child-2",
        steering_version: 2,
        resume_trigger: "user_followup",
        has_user_message: true,
        has_new_constraints: true,
      },
    },
    "en",
  );
  assert.equal(steeringView.title, "Subagent steering updated");
  assert.equal(steeringView.detail, "child-2 · v2 · user_followup");
  assert.equal(steeringView.tone, "attention");
});

test("distinguishes archive recovery from an irrecoverable event gap", () => {
  const recovered = buildTaskTraceEventView(
    {
      event_type: "archive_replay",
      payload: {
        requested_cursor: 4,
        oldest_available_seq: 1,
        newest_available_seq: 2048,
        replay_mode: "archive_recovery",
      },
    },
    "en",
  );
  assert.equal(recovered.title, "Archived events restored");
  assert.equal(recovered.detail, "Replay range 1 to 2048.");
  assert.equal(recovered.tone, "ok");

  const expired = buildTaskTraceEventView(
    {
      event_type: "cursor_expired",
      payload: {
        requested_cursor: 4,
        oldest_available_seq: 20,
        newest_available_seq: 2048,
        replay_mode: "available_suffix",
        replay_source: "archive",
      },
    },
    "en",
  );
  assert.equal(expired.title, "Event history gap");
  assert.equal(expired.detail, "The oldest available event is 20.");
  assert.equal(expired.tone, "attention");
});

test("extracts visible task text before falling back to error text", () => {
  const result: TaskQueryResponse = {
    task_id: "task-1",
    status: "succeeded",
    result_json: { text: "dry_run=true\noutput_path=/tmp/a.png" },
    error_text: "ignored",
  };

  assert.equal(extractTaskText(result), "dry_run=true\noutput_path=/tmp/a.png");
});

test("builds a task-bound approval request from structured resume context", () => {
  const result: TaskQueryResponse = {
    task_id: "task-approval",
    status: "failed",
    result_json: {
      resume_context: {
        approval_request: {
          schema_version: 1,
          request_id: "approval-1",
          task_id: "task-approval",
          status: "pending",
          targets: ["run_cmd", "fs_basic"],
          action_count: 2,
          expires_at: 1_800_000_000,
          reversible: false,
          effect: "mutating_or_external_action",
          reason_code: "explicit_approval_required",
          allowed_decisions: ["approve_once", "always_for_scope", "deny"],
          scope_grant: {
            available: true,
            scope_kind: "session",
            max_ttl_seconds: 3600,
            entries: [{
              capability: "filesystem.write_file",
              action: "write_text",
              resource_kind: "workspace_path",
              resources: ["run/example.txt"],
            }],
          },
        },
      },
    },
  };

  assert.deepEqual(buildTaskApprovalRequest(result), {
    requestId: "approval-1",
    status: "pending",
    targets: ["run_cmd", "fs_basic"],
    actionCount: 2,
    expiresAt: 1_800_000_000,
    reversible: false,
    effect: "mutating_or_external_action",
    reasonCode: "explicit_approval_required",
    allowedDecisions: ["approve_once", "always_for_scope", "deny"],
    scopeGrant: {
      available: true,
      scopeKind: "session",
      maxTtlSeconds: 3600,
      entries: [{
        capability: "filesystem.write_file",
        action: "write_text",
        resourceKind: "workspace_path",
        resources: ["run/example.txt"],
      }],
    },
  });

  result.task_id = "another-task";
  assert.equal(buildTaskApprovalRequest(result), null);
});

test("projects workspace patch evidence from task events", () => {
  const event = {
    seq: 8,
    event_type: "tool_finished",
    payload: {
      status: "ok",
      checkpoint_id: "patch_checkpoint_1",
      patch_id: "sha256:patch-1",
      mutation_id: "sha256:mutation-1",
      compensates_checkpoint_id: "mutation_checkpoint_1",
      compensates_mutation_id: "sha256:mutation-0",
      target_path: "src/lib.rs",
      isolation_root: "workspace://current",
      reversible: true,
      additions: 4,
      deletions: 2,
      changed_hunks: 2,
    },
  };

  const meta = traceEventMeta(event);

  assert.ok(meta.includes("checkpoint_id=patch_checkpoint_1"));
  assert.ok(meta.includes("patch_id=sha256:patch-1"));
  assert.ok(meta.includes("mutation_id=sha256:mutation-1"));
  assert.ok(meta.includes("compensates_mutation_id=sha256:mutation-0"));
  assert.ok(meta.includes("target_path=src/lib.rs"));
  assert.ok(meta.includes("isolation_root=workspace://current"));
  assert.ok(meta.includes("reversible=true"));
  assert.ok(meta.includes("changed_hunks=2"));
});

test("projects untracked shell reversibility from task events", () => {
  const meta = traceEventMeta({
    seq: 9,
    event_type: "tool_finished",
    payload: {
      skill: "run_cmd",
      status: "ok",
      reversible: false,
      reversibility_status: "not_rewindable",
      reversibility_reason_code: "shell_side_effects_not_tracked",
    },
  });

  assert.ok(meta.includes("reversible=false"));
  assert.ok(meta.includes("reversibility_status=not_rewindable"));
  assert.ok(meta.includes("reversibility_reason_code=shell_side_effects_not_tracked"));
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
            final_answer_shape: "single_path",
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
  assert.equal(view.finalShape, "single_path");
  assert.deepEqual(view.doneConditions, ["tests_pass"]);
  assert.ok(view.constraints.includes("scope=workspace"));
  assert.ok(view.constraints.includes("writes_allowed=true"));
  assert.ok(view.verification.includes("command=cargo test -p clawd"));
  assert.ok(view.verification.includes("verification_status=verified"));
  assert.ok(view.currentProgress.includes("changed_file_count=1"));
  assert.ok(view.remainingWork.includes("summarize"));
});

test("uses the highest coding projection revision after checkpoint continuation", () => {
  const result: TaskQueryResponse = {
    task_id: "task-coding-projection",
    status: "succeeded",
    result_json: {
      task_journal: {
        summary: {
          final_status: "success",
        },
        trace: {
          event_stream: [
            {
              seq: 14,
              event_type: "coding_evidence",
              payload: {
                projection_revision: 6,
                verification_status: "verified",
                verification_failure_kind_count: 0,
              },
            },
            {
              seq: 8,
              event_type: "coding_evidence",
              payload: {
                projection_revision: 2,
                verification_status: "failed",
                verification_failure_kind_count: 1,
              },
            },
          ],
        },
      },
    },
  };

  const view = buildTaskOutcome(result, "en");
  assert.ok(view.verification.includes("verification_status=verified"));
  assert.ok(view.verification.includes("verification_failure_kind_count=0"));
  assert.ok(!view.verification.includes("verification_status=failed"));
});

test("uses authoritative coding summary when compact trace omits late projections", () => {
  const result: TaskQueryResponse = {
    task_id: "task-coding-summary-fallback",
    status: "succeeded",
    result_json: {
      task_journal: {
        summary: {
          final_status: "success",
          coding_workflow: {
            schema_version: 2,
            projection_revision: 8,
            latest_verification_step_ref: "step_7",
            verification_status: "verified",
            verification_failure_kind_count: 0,
            historical_verification_failure_kind_count: 1,
          },
        },
        trace: {
          event_stream: [
            {
              seq: 3,
              event_type: "coding_evidence",
              payload: {
                projection_revision: 2,
                verification_status: "failed",
                verification_failure_kind_count: 1,
              },
            },
          ],
        },
      },
    },
  };

  const view = buildTaskOutcome(result, "en");
  assert.ok(view.verification.includes("verification_status=verified"));
  assert.ok(view.verification.includes("verification_failure_kind_count=0"));
  assert.ok(!view.verification.includes("verification_status=failed"));
});

test("builds task goal view from query goal projection", () => {
  const result: TaskQueryResponse = {
    task_id: "task-goal-view",
    status: "running",
    goal: {
      schema_version: 1,
      task_id: "task-goal-view",
      goal_id: "task:task-goal-view",
      objective: "ship feature",
      goal_status: "background",
      goal_status_source: "lifecycle",
      done_conditions: ["tests_pass"],
      constraints: [{ scope: "workspace" }],
      verification_commands: ["cargo test -p clawcli"],
      current_progress: ["changed_file_count=1"],
      remaining_work: ["summarize"],
    },
  };

  const view = buildTaskGoalView(result, "en");

  assert.equal(view?.title, "Goal progress");
  assert.equal(view?.tone, "running");
  assert.equal(view?.status, "background");
  assert.equal(view?.objective, "ship feature");
  assert.ok(view?.meta.includes("goal_id=task:task-goal-view"));
  assert.ok(view?.meta.includes("goal_status_source=lifecycle"));
  assert.deepEqual(view?.doneConditions, ["tests_pass"]);
  assert.ok(view?.constraints.includes("scope=workspace"));
  assert.deepEqual(view?.verificationCommands, ["cargo test -p clawcli"]);
  assert.deepEqual(view?.currentProgress, ["changed_file_count=1"]);
  assert.deepEqual(view?.remainingWork, ["summarize"]);
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

test("projects agent hook execution fields for teaching traces", () => {
  const event = {
    seq: 4,
    event_type: "agent_hook",
    payload: {
      status: "error",
      owner_layer: "agent_hooks",
      stage: "permission_request",
      decision: "deny",
      reason_code: "fixture_denied",
      status_code: "fixture_denied",
      error_code: "hook_handler_timeout",
      handler_id: "workspace_policy_guard",
      handler_kind: "command",
      blocking: true,
      failure_policy: "deny",
      trust_status: "trusted",
      content_sha256: "sha256:fixture",
      duration_ms: 120,
      attempts: 1,
      output_truncated: false,
    },
  };

  const meta = traceEventMeta(event);
  assert.ok(meta.includes("handler_id=workspace_policy_guard"));
  assert.ok(meta.includes("handler_kind=command"));
  assert.ok(meta.includes("trust_status=trusted"));
  assert.ok(meta.includes("error_code=hook_handler_timeout"));
  assert.ok(meta.includes("duration_ms=120"));
  assert.ok(meta.includes("attempts=1"));
  assert.ok(meta.includes("output_truncated=false"));

  const view = buildTaskTraceEventView(event, "en");
  assert.equal(view.title, "Agent lifecycle hook");
  assert.equal(view.detail, "workspace_policy_guard · permission_request · deny");
  assert.equal(view.tone, "failed");
  assert.ok(view.meta.includes("failure_policy=deny"));
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
            {
              seq: 8,
              event_type: "agent_team_started",
              payload: {
                team_id: "subagent-batch:1:1",
                max_parallel: 2,
                write_permission: "read_only",
                conflict_policy: "parent_loop_resolution_required",
                status: "started",
              },
            },
            {
              seq: 9,
              event_type: "subagent_finished",
              payload: {
                team_id: "subagent-batch:1:1",
                child_task_id: "subagent-batch:1:1:explorer",
                child_run_id: "subagent-batch:1:1:explorer",
                role: "explorer",
                required: true,
                status: "completed",
                write_permission: "read_only",
              },
            },
            {
              seq: 10,
              event_type: "agent_team_conflict_detected",
              payload: {
                team_id: "subagent-batch:1:1",
                status: "needs_conflict_resolution",
                reason_code: "subagent_conflict_detected",
                recommended_next_action: "resolve_child_conflicts",
              },
            },
            {
              seq: 11,
              event_type: "task_goal",
              payload: {
                task_id: "task-events",
                goal_status: "verified",
                goal_status_source: "validation_result",
                validation_status: "passed",
              },
            },
            {
              seq: 12,
              event_type: "context_budget",
              payload: {
                budget_tier: "light",
                included_ref_count: 2,
                excluded_ref_count: 1,
                token_estimate: 96,
                compaction_source: "deterministic_context_builder",
              },
            },
            {
              seq: 13,
              event_type: "context_compaction",
              payload: {
                record_count: 1,
                summary_kind: "deterministic_context_budget",
                compaction_id: "context_compaction:1",
              },
            },
            {
              seq: 14,
              event_type: "budget_decision",
              payload: {
                decision: "checkpoint_requeue",
                profile: "multi_step_workspace",
                soft_slice_ms: 900000,
                continuation_index: 2,
                cumulative_model_turns: 7,
                cumulative_tool_calls: 15,
                observed_progress: true,
                soft_slice_exhausted: true,
                resumable: true,
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
  assert.ok(traceEventMeta(events[7]).includes("team_id=subagent-batch:1:1"));
  assert.ok(traceEventMeta(events[7]).includes("max_parallel=2"));
  assert.ok(traceEventMeta(events[7]).includes("write_permission=read_only"));
  assert.ok(traceEventMeta(events[8]).includes("child_run_id=subagent-batch:1:1:explorer"));
  assert.ok(traceEventMeta(events[8]).includes("role=explorer"));
  assert.ok(traceEventMeta(events[8]).includes("required=true"));
  assert.ok(traceEventMeta(events[9]).includes("recommended_next_action=resolve_child_conflicts"));
  assert.ok(traceEventMeta(events[10]).includes("goal_status=verified"));
  assert.ok(traceEventMeta(events[10]).includes("validation_status=passed"));
  assert.ok(traceEventMeta(events[11]).includes("budget_tier=light"));
  assert.ok(traceEventMeta(events[11]).includes("included_ref_count=2"));
  assert.ok(traceEventMeta(events[12]).includes("record_count=1"));
  assert.ok(traceEventMeta(events[12]).includes("summary_kind=deterministic_context_budget"));
  assert.ok(traceEventMeta(events[13]).includes("decision=checkpoint_requeue"));
  assert.ok(traceEventMeta(events[13]).includes("profile=multi_step_workspace"));
  assert.ok(traceEventMeta(events[13]).includes("cumulative_model_turns=7"));
  assert.ok(traceEventMeta(events[13]).includes("cumulative_tool_calls=15"));
  assert.ok(traceEventMeta(events[13]).includes("continuation_index=2"));
  assert.ok(traceEventMeta(events[13]).includes("soft_slice_exhausted=true"));
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
  assert.equal(buildTaskTraceEventView(events[7], "en").title, "Agent team started");
  assert.equal(buildTaskTraceEventView(events[8], "en").title, "Subagent finished");
  assert.equal(buildTaskTraceEventView(events[9], "en").title, "Agent team conflict");
  assert.equal(buildTaskTraceEventView(events[9], "en").tone, "attention");
  assert.equal(buildTaskTraceEventView(events[10], "en").title, "Goal state");
  assert.equal(buildTaskTraceEventView(events[10], "en").tone, "ok");
  assert.equal(buildTaskTraceEventView(events[11], "en").title, "Context budget");
  assert.equal(buildTaskTraceEventView(events[12], "en").title, "Context compaction");
  assert.equal(buildTaskTraceEventView(events[13], "en").title, "Task budget decision");
  assert.equal(
    buildTaskTraceEventView(events[13], "en").detail,
    "multi_step_workspace · checkpoint_requeue · continuation 2",
  );
  assert.equal(buildTaskTraceEventView(events[13], "en").tone, "attention");
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
