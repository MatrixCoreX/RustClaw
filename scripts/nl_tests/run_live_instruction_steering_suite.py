#!/usr/bin/env python3
"""Run live in-flight conversation steering cases against an existing clawd."""

from __future__ import annotations

import argparse
import json
import os
import sys
import time
import urllib.error
import urllib.request
import uuid
from pathlib import Path
from typing import Any, Iterator


TERMINAL_STATUSES = {"succeeded", "failed", "canceled", "cancelled"}


def percentile(values: list[float], percentage: float) -> float | None:
    if not values:
        return None
    ordered = sorted(values)
    position = (len(ordered) - 1) * percentage / 100.0
    lower = int(position)
    upper = min(lower + 1, len(ordered) - 1)
    fraction = position - lower
    return round(ordered[lower] + (ordered[upper] - ordered[lower]) * fraction, 3)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--base-url", default=os.environ.get("BASE_URL", "http://127.0.0.1:8787"))
    parser.add_argument("--auth-key", default=os.environ.get("APP_USER_KEY", ""))
    parser.add_argument("--cases", type=Path, required=True)
    parser.add_argument("--model-io-log", type=Path, default=Path("logs/model_io.log"))
    parser.add_argument("--run-root", type=Path, default=Path("scripts/nl_suite_logs/live_instruction_steering"))
    parser.add_argument("--timeout-seconds", type=float, default=240.0)
    parser.add_argument("--poll-seconds", type=float, default=0.1)
    parser.add_argument("--max-print-chars", type=int, default=2400)
    parser.add_argument("--selected-only", action="store_true")
    parser.add_argument("--exclude-selected", action="store_true")
    parser.add_argument("--case-limit", type=int)
    parser.add_argument("--case-id", action="append", default=[])
    return parser.parse_args()


class ApiClient:
    def __init__(self, base_url: str, auth_key: str, timeout_seconds: float) -> None:
        self.base_url = base_url.rstrip("/") + "/v1"
        self.auth_key = auth_key.strip()
        self.timeout_seconds = timeout_seconds

    def request(self, method: str, path: str, body: dict[str, Any] | None = None) -> tuple[int, dict[str, Any]]:
        encoded = None if body is None else json.dumps(body, ensure_ascii=False).encode("utf-8")
        headers = {"Content-Type": "application/json"}
        if self.auth_key:
            headers["X-Agent-Key"] = self.auth_key
        request = urllib.request.Request(
            self.base_url + path,
            data=encoded,
            method=method,
            headers=headers,
        )
        try:
            with urllib.request.urlopen(request, timeout=self.timeout_seconds) as response:
                return response.status, json.loads(response.read())
        except urllib.error.HTTPError as error:
            raw = error.read().decode("utf-8", errors="replace")
            return error.code, json.loads(raw) if raw else {"ok": False, "error": str(error)}

    def task_events(self, task_id: str, timeout_seconds: float) -> Iterator[dict[str, Any]]:
        headers = {"Accept": "text/event-stream", "Last-Event-ID": "0"}
        if self.auth_key:
            headers["X-Agent-Key"] = self.auth_key
        request = urllib.request.Request(
            f"{self.base_url}/tasks/{task_id}/events?cursor=0",
            method="GET",
            headers=headers,
        )
        data_lines: list[str] = []
        with urllib.request.urlopen(request, timeout=timeout_seconds) as response:
            for raw_line in response:
                line = raw_line.decode("utf-8", errors="replace").rstrip("\r\n")
                if line.startswith("data:"):
                    value = line[5:]
                    data_lines.append(value[1:] if value.startswith(" ") else value)
                    continue
                if line or not data_lines:
                    continue
                event = json.loads("\n".join(data_lines))
                data_lines.clear()
                if isinstance(event, dict):
                    yield event


def cancel_fixture_task(client: ApiClient, task_id: str) -> None:
    try:
        client.request(
            "POST",
            "/tasks/cancel-by-task-id",
            {
                "task_id": task_id,
                "idempotency_key": f"live-steering-timeout:{task_id}",
            },
        )
    except Exception:
        pass


def client_task_body(text: str, conversation_id: str, message_id: str) -> dict[str, Any]:
    return {
        "input": {
            "schema_version": 1,
            "client_message_id": message_id,
            "scope": {
                "conversation_id": conversation_id,
                "agent_id": "main",
                "channel": "ui",
                "channel_account_id": "live-steering-suite",
            },
            "content": [{"kind": "text", "text": text}],
            "delivery_mode": "auto",
            "source": {},
        },
        "task": {
            "user_id": 1,
            "chat_id": 2,
            "channel": "ui",
            "external_user_id": "live-steering-suite",
            "external_chat_id": conversation_id,
            "kind": "ask",
            "payload": {"text": text},
        },
    }


def resume_task_body(task_id: str, steering: dict[str, Any], request_id: str) -> dict[str, Any]:
    body: dict[str, Any] = {
        "task_id": task_id,
        "resume_reason": str(steering.get("resume_reason") or "explicit_user_resume"),
        "user_message": steering.get("text"),
        "idempotency_key": request_id,
    }
    if "checkpoint_id" in steering:
        body["checkpoint_id"] = steering["checkpoint_id"]
    if "new_constraints" in steering:
        body["new_constraints"] = steering["new_constraints"]
    return body


def response_data(response: dict[str, Any], label: str) -> dict[str, Any]:
    data = response.get("data")
    if not response.get("ok") or not isinstance(data, dict):
        raise RuntimeError(f"{label}_failed:{response.get('error') or 'missing_data'}")
    return data


def wait_for_status(
    client: ApiClient,
    task_id: str,
    wanted: set[str],
    timeout_seconds: float,
    poll_seconds: float,
) -> tuple[dict[str, Any], int]:
    deadline = time.monotonic() + timeout_seconds
    polls = 0
    while time.monotonic() < deadline:
        polls += 1
        status, response = client.request("GET", f"/tasks/{task_id}")
        data = response_data(response, "task_poll")
        task_status = str(data.get("status") or "")
        if status == 200 and task_status in wanted:
            return data, polls
        if task_status in TERMINAL_STATUSES and not TERMINAL_STATUSES.intersection(wanted):
            raise RuntimeError(f"task_terminal_before_injection:{task_status}")
        time.sleep(poll_seconds)
    cancel_fixture_task(client, task_id)
    raise TimeoutError(f"task_status_timeout:wanted={sorted(wanted)}")


def task_lifecycle_state(task: dict[str, Any]) -> str:
    lifecycle = task.get("lifecycle")
    if not isinstance(lifecycle, dict):
        result = task.get("result_json")
        lifecycle = result.get("task_lifecycle") if isinstance(result, dict) else None
    return str(lifecycle.get("state") or "") if isinstance(lifecycle, dict) else ""


def expected_completion_outcomes(expected: dict[str, Any]) -> list[dict[str, Any]]:
    outcomes = expected.get("accepted_outcomes")
    if isinstance(outcomes, list) and outcomes and all(isinstance(item, dict) for item in outcomes):
        return outcomes
    return [expected]


def outcome_statuses(outcome: dict[str, Any]) -> set[str]:
    raw_status = outcome.get("status", "succeeded")
    return (
        {str(value) for value in raw_status}
        if isinstance(raw_status, list)
        else {str(raw_status)}
    )


def task_matches_completion_outcome(task: dict[str, Any], outcome: dict[str, Any]) -> bool:
    if str(task.get("status") or "") not in outcome_statuses(outcome):
        return False
    wanted_lifecycle = str(outcome.get("lifecycle_state") or "")
    return not wanted_lifecycle or task_lifecycle_state(task) == wanted_lifecycle


def wait_for_expected_completion(
    client: ApiClient,
    task_id: str,
    expected: dict[str, Any],
    timeout_seconds: float,
    poll_seconds: float,
) -> tuple[dict[str, Any], int]:
    outcomes = expected_completion_outcomes(expected)
    wanted_statuses = set().union(*(outcome_statuses(outcome) for outcome in outcomes))
    deadline = time.monotonic() + timeout_seconds
    polls = 0
    while time.monotonic() < deadline:
        polls += 1
        status, response = client.request("GET", f"/tasks/{task_id}")
        task = response_data(response, "task_poll")
        task_status = str(task.get("status") or "")
        if status == 200 and any(
            task_matches_completion_outcome(task, outcome) for outcome in outcomes
        ):
            return task, polls
        if task_status in TERMINAL_STATUSES and task_status not in wanted_statuses:
            return task, polls
        time.sleep(poll_seconds)
    cancel_fixture_task(client, task_id)
    raise TimeoutError(
        f"task_completion_timeout:statuses={sorted(wanted_statuses)} "
        f"lifecycle={sorted({str(outcome.get('lifecycle_state') or '*') for outcome in outcomes})}"
    )


def task_step_results(task: dict[str, Any]) -> list[dict[str, Any]]:
    trace = journal_trace(task)
    candidates = trace.get("step_results")
    if not isinstance(candidates, list):
        candidates = trace.get("executed_step_results")
    return [item for item in candidates or [] if isinstance(item, dict)]


def injection_step_observed(
    task: dict[str, Any],
    required_step_skills: set[str],
    required_step_actions: set[str],
) -> bool:
    results = task_step_results(task)
    observed_skills = {str(result.get("skill") or "") for result in results}
    observed_actions = {
        str(result.get("requested_action_ref") or "") for result in results
    }
    skill_matches = not required_step_skills or bool(
        required_step_skills.intersection(observed_skills)
    )
    action_matches = not required_step_actions or bool(
        required_step_actions.intersection(observed_actions)
    )
    return bool(results) and skill_matches and action_matches


def task_event_matches_step(
    event: dict[str, Any],
    required_step_skills: set[str],
    required_step_actions: set[str],
) -> bool:
    if event.get("event_kind") != "tool_finished":
        return False
    payload = event.get("payload")
    if not isinstance(payload, dict) or payload.get("status") != "ok":
        return False
    skill = str(payload.get("skill") or payload.get("executed_skill") or "")
    action = str(payload.get("requested_action_ref") or "")
    skill_matches = not required_step_skills or skill in required_step_skills
    action_matches = not required_step_actions or action in required_step_actions
    return skill_matches and action_matches


def task_event_is_terminal(event: dict[str, Any]) -> bool:
    if event.get("event_kind") == "task_final":
        return True
    if event.get("event_kind") != "task_state":
        return False
    payload = event.get("payload")
    return isinstance(payload, dict) and payload.get("status") in TERMINAL_STATUSES


def wait_for_step_event_phase(
    client: ApiClient,
    task_id: str,
    phase: str,
    minimum_step_results: int,
    required_step_skills: set[str],
    required_step_actions: set[str],
    timeout_seconds: float,
) -> tuple[dict[str, Any], int]:
    observed_steps: set[str] = set()
    event_count = 0
    for event in client.task_events(task_id, timeout_seconds):
        event_count += 1
        if event.get("event_kind") == "tool_finished":
            payload = event.get("payload")
            if isinstance(payload, dict) and payload.get("status") == "ok":
                observed_steps.add(str(payload.get("step_id") or event.get("seq") or ""))
        if phase == "after_step_results" and len(observed_steps) >= minimum_step_results:
            return {"status": "running", "matched_event": event}, event_count
        if phase == "after_step_skill" and task_event_matches_step(
            event, required_step_skills, required_step_actions
        ):
            return {"status": "running", "matched_event": event}, event_count
        if task_event_is_terminal(event):
            raise RuntimeError("task_terminal_before_injection:event_stream")
    raise TimeoutError(
        f"injection_event_timeout:phase={phase} min_step_results={minimum_step_results} "
        f"required_step_skills={sorted(required_step_skills)} "
        f"required_step_actions={sorted(required_step_actions)}"
    )


def wait_for_injection_phase(
    client: ApiClient,
    task_id: str,
    phase: str,
    minimum_step_results: int,
    required_step_skills: set[str],
    required_step_actions: set[str],
    timeout_seconds: float,
    poll_seconds: float,
) -> tuple[dict[str, Any], int]:
    if phase in {"after_step_results", "after_step_skill"}:
        return wait_for_step_event_phase(
            client,
            task_id,
            phase,
            minimum_step_results,
            required_step_skills,
            required_step_actions,
            timeout_seconds,
        )
    deadline = time.monotonic() + timeout_seconds
    polls = 0
    while time.monotonic() < deadline:
        polls += 1
        status, response = client.request("GET", f"/tasks/{task_id}")
        task = response_data(response, "task_phase_poll")
        task_status = str(task.get("status") or "")
        if status == 200:
            if phase == "running" and task_status == "running":
                return task, polls
            if phase == "paused" and task_lifecycle_state(task) == "needs_user":
                return task, polls
        if task_status in TERMINAL_STATUSES:
            raise RuntimeError(f"task_terminal_before_injection:{task_status}")
        time.sleep(poll_seconds)
    cancel_fixture_task(client, task_id)
    raise TimeoutError(
        f"injection_phase_timeout:phase={phase} min_step_results={minimum_step_results} "
        f"required_step_skills={sorted(required_step_skills)} "
        f"required_step_actions={sorted(required_step_actions)}"
    )


def visible_text(task: dict[str, Any]) -> str:
    result = task.get("result_json")
    if not isinstance(result, dict):
        return "" if result is None else str(result)
    text = result.get("text")
    if isinstance(text, str) and text:
        return text
    messages = result.get("messages")
    if isinstance(messages, list):
        return "\n".join(str(message) for message in messages if isinstance(message, str))
    return ""


def journal_trace(task: dict[str, Any]) -> dict[str, Any]:
    result = task.get("result_json")
    if not isinstance(result, dict):
        return {}
    journal = result.get("task_journal")
    if isinstance(journal, dict):
        trace = journal.get("trace")
        if isinstance(trace, dict) and trace:
            return trace
    checkpoint = result.get("task_checkpoint")
    boundary = checkpoint.get("boundary_context") if isinstance(checkpoint, dict) else None
    resume_state = (
        boundary.get("agent_loop_resume_state") if isinstance(boundary, dict) else None
    )
    if not isinstance(resume_state, dict):
        return {}
    trace = dict(resume_state)
    if isinstance(boundary.get("task_llm_metrics"), dict):
        trace["task_metrics"] = boundary["task_llm_metrics"]
    return trace


def model_rows(path: Path, task_id: str) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    with path.open("rb") as handle:
        while True:
            offset = handle.tell()
            raw = handle.readline()
            if not raw:
                break
            try:
                row = json.loads(raw)
            except (UnicodeDecodeError, json.JSONDecodeError):
                continue
            if row.get("task_id") == task_id:
                row["_log_offset"] = offset
                rows.append(row)
    return rows


def parsed_response_fields(row: dict[str, Any]) -> list[str]:
    clean = row.get("clean_response")
    if not isinstance(clean, str):
        return []
    try:
        value = json.loads(clean)
    except json.JSONDecodeError:
        return []
    return sorted(value) if isinstance(value, dict) else []


def finish_reason(row: dict[str, Any]) -> str | None:
    clean = row.get("clean_response")
    if isinstance(clean, str):
        try:
            value = json.loads(clean)
        except json.JSONDecodeError:
            value = None
        if isinstance(value, dict) and isinstance(value.get("finish_reason"), str):
            return value["finish_reason"]
    response = row.get("response")
    if isinstance(response, str):
        try:
            response = json.loads(response)
        except json.JSONDecodeError:
            response = None
    if isinstance(response, dict):
        choices = response.get("choices")
        if isinstance(choices, list) and choices and isinstance(choices[0], dict):
            reason = choices[0].get("finish_reason")
            return reason if isinstance(reason, str) else None
    raw_response = row.get("raw_response")
    if isinstance(raw_response, str):
        try:
            raw_response = json.loads(raw_response)
        except json.JSONDecodeError:
            raw_response = None
    if isinstance(raw_response, dict):
        choices = raw_response.get("choices")
        if isinstance(choices, list) and choices and isinstance(choices[0], dict):
            reason = choices[0].get("finish_reason")
            return reason if isinstance(reason, str) else None
    return None


def response_relation(row: dict[str, Any]) -> str | None:
    clean = row.get("clean_response")
    if not isinstance(clean, str):
        return None
    try:
        value = json.loads(clean)
    except json.JSONDecodeError:
        return None
    if not isinstance(value, dict):
        return None
    for tool_call in value.get("tool_calls") or []:
        if not isinstance(tool_call, dict):
            continue
        arguments = tool_call.get("arguments")
        if isinstance(arguments, dict) and isinstance(arguments.get("conversation_relation"), str):
            return arguments["conversation_relation"]
    return None


def accepted_planner_relations(task: dict[str, Any]) -> list[str]:
    trace = journal_trace(task)
    result = task.get("result_json")
    journal = result.get("task_journal") if isinstance(result, dict) else None
    summary = journal.get("summary") if isinstance(journal, dict) else None
    rounds = trace.get("rounds")
    if isinstance(rounds, str):
        try:
            rounds = json.loads(rounds)
        except json.JSONDecodeError:
            rounds = []
    if not isinstance(rounds, list):
        rounds = []
    relations: list[str] = []
    for container in (trace, summary):
        if not isinstance(container, dict):
            continue
        relation = container.get("latest_planner_conversation_relation")
        if isinstance(relation, str) and relation and relation not in relations:
            relations.append(relation)
    for round_record in rounds:
        if not isinstance(round_record, dict):
            continue
        envelope = round_record.get("decision_envelope")
        relation = (
            envelope.get("conversation_relation")
            if isinstance(envelope, dict)
            else None
        )
        if isinstance(relation, str) and relation and relation not in relations:
            relations.append(relation)
    return relations


def parsed_tool_calls(row: dict[str, Any]) -> list[dict[str, Any]]:
    clean = row.get("clean_response")
    if not isinstance(clean, str):
        return []
    try:
        value = json.loads(clean)
    except json.JSONDecodeError:
        return []
    calls = value.get("tool_calls") if isinstance(value, dict) else None
    if not isinstance(calls, list):
        return []
    return [
        {
            "name": str(call.get("name") or ""),
            "arguments": call.get("arguments")
            if isinstance(call.get("arguments"), dict)
            else {},
        }
        for call in calls
        if isinstance(call, dict)
    ]


def compact_field(value: Any, limit: int) -> dict[str, Any]:
    if value is None:
        return {"chars": 0, "head": None, "tail": None}
    text = value if isinstance(value, str) else json.dumps(value, ensure_ascii=False, sort_keys=True)
    if len(text) <= limit:
        return {"chars": len(text), "head": text, "tail": None}
    half = max(1, limit // 2)
    return {"chars": len(text), "head": text[:half], "tail": text[-half:]}


def merged_llm_calls(task: dict[str, Any], rows: list[dict[str, Any]], max_chars: int) -> list[dict[str, Any]]:
    trace = journal_trace(task)
    metrics = trace.get("task_metrics") if isinstance(trace.get("task_metrics"), dict) else {}
    cost_records = metrics.get("llm_cost_records") if isinstance(metrics.get("llm_cost_records"), list) else []
    by_index = {
        int(row["logical_call_index"]): row
        for row in rows
        if isinstance(row.get("logical_call_index"), int)
    }
    indexes = set(by_index)
    indexes.update(
        int(record["logical_call_index"])
        for record in cost_records
        if isinstance(record, dict) and isinstance(record.get("logical_call_index"), int)
    )
    if indexes:
        indexes.update(range(1, max(indexes) + 1))
    calls: list[dict[str, Any]] = []
    for ordinal, logical_index in enumerate(sorted(indexes), 1):
        row = by_index.get(logical_index, {})
        cost = next(
            (
                record
                for record in cost_records
                if isinstance(record, dict) and record.get("logical_call_index") == logical_index
            ),
            {},
        )
        inferred_interruption = not row and not cost
        calls.append(
            {
                "llm_call_ref": f"LLM#{ordinal}",
                "logical_call_index": logical_index,
                "prompt_label": row.get("prompt_source") or cost.get("prompt_label"),
                "logical_prompt_path": row.get("prompt_source"),
                "provider": row.get("provider") or cost.get("provider"),
                "model": row.get("model") or cost.get("model"),
                "status": row.get("status")
                or cost.get("provider_status")
                or ("interrupted" if inferred_interruption else None),
                "provider_status": cost.get("provider_status")
                or ("interrupted" if inferred_interruption else None),
                "status_evidence": (
                    "inferred_from_contiguous_logical_call_gap"
                    if inferred_interruption
                    else "recorded"
                ),
                "finish_reason": finish_reason(row),
                "usage": row.get("usage") or cost.get("usage"),
                "error": row.get("error"),
                "raw_fields": sorted(key for key in row if not key.startswith("_")),
                "parsed_json_fields": parsed_response_fields(row),
                "response_text": compact_field(row.get("clean_response"), max_chars),
                "raw_response": compact_field(row.get("raw_response"), max_chars),
                "request_payload": compact_field(row.get("request_payload"), max_chars),
                "log_offset": row.get("_log_offset"),
            }
        )
    return calls


def evaluate(
    case: dict[str, Any],
    task: dict[str, Any],
    followup_receipts: list[dict[str, Any]],
    handoff_states: list[str],
    llm_calls: list[dict[str, Any]],
    relations: list[str],
    tool_calls: list[dict[str, Any]],
    operation_records: list[dict[str, Any]],
) -> dict[str, bool]:
    expected = case.get("expected") or {}
    text = visible_text(task)
    lines = [line for line in text.splitlines() if line.strip()]
    outcomes = expected_completion_outcomes(expected)
    wanted_statuses = set().union(*(outcome_statuses(outcome) for outcome in outcomes))
    control_actions = [
        call.get("arguments", {}).get("action")
        for call in tool_calls
        if call.get("name") == "control_active_turn"
    ]
    checks = {
        "terminal_status": task.get("status") in wanted_statuses,
        "lifecycle_state": (
            any(task_matches_completion_outcome(task, outcome) for outcome in outcomes)
        ),
        "followups_bound_existing": all(state == "bound_existing_task" for state in handoff_states),
        "followups_applied": all(receipt.get("disposition") == "applied" for receipt in followup_receipts),
        "revision_advanced": all(
            receipt.get("instruction_revision") == index + 2
            for index, receipt in enumerate(followup_receipts)
        ),
        "epoch_advanced": all(
            receipt.get("execution_epoch") == index + 2
            for index, receipt in enumerate(followup_receipts)
        ),
        "required_text_present": all(token in text for token in expected.get("contains", [])),
        "forbidden_text_absent": all(token not in text for token in expected.get("absent", [])),
        "planner_relation": not expected.get("relations") or relations == expected.get("relations"),
        "planner_relation_allowed": (
            not expected.get("allowed_relations")
            or bool(relations)
            and all(relation in expected["allowed_relations"] for relation in relations)
        ),
        "required_relations": all(
            relation in relations for relation in expected.get("required_relations", [])
        ),
        "control_actions": (
            not expected.get("control_actions")
            or control_actions == expected.get("control_actions")
        ),
        "forbidden_control_actions": all(
            action not in control_actions
            for action in expected.get("forbidden_control_actions", [])
        ),
        "interrupted_model_call": (
            not expected.get("require_interrupted_model_call", True)
            or any(call.get("provider_status") == "interrupted" for call in llm_calls)
        ),
        "resume_operations": (
            "resume_count" not in expected
            or sum(record.get("transport") == "resume" for record in operation_records)
            == int(expected["resume_count"])
        ),
    }
    if "accepted_outcomes" in expected:
        checks["accepted_outcome"] = any(
            task_matches_completion_outcome(task, outcome)
            and all(token in text for token in outcome.get("contains", []))
            and all(token not in text for token in outcome.get("absent", []))
            and (
                "min_llm_calls" not in outcome
                or len(llm_calls) >= int(outcome["min_llm_calls"])
            )
            and (
                "max_llm_calls" not in outcome
                or len(llm_calls) <= int(outcome["max_llm_calls"])
            )
            for outcome in outcomes
        )
    if "exact_nonblank_lines" in expected:
        checks["exact_nonblank_lines"] = len(lines) == int(expected["exact_nonblank_lines"])
    if "min_nonblank_lines" in expected:
        checks["min_nonblank_lines"] = len(lines) >= int(expected["min_nonblank_lines"])
    if "max_chars" in expected:
        checks["max_chars"] = len(text) <= int(expected["max_chars"])
    if "json_object_keys" in expected:
        try:
            parsed_json = json.loads(text)
        except (TypeError, ValueError):
            parsed_json = None
        checks["json_object_keys"] = (
            isinstance(parsed_json, dict)
            and set(parsed_json) == set(expected["json_object_keys"])
        )
    if "min_nonblank_line_chars" in expected:
        checks["min_nonblank_line_chars"] = bool(lines) and all(
            len(line.strip()) >= int(expected["min_nonblank_line_chars"])
            for line in lines
        )
    if "min_llm_calls" in expected:
        checks["min_llm_calls"] = len(llm_calls) >= int(expected["min_llm_calls"])
    if "max_llm_calls" in expected:
        checks["max_llm_calls"] = len(llm_calls) <= int(expected["max_llm_calls"])
    return checks


def trace_evidence(task: dict[str, Any], max_chars: int) -> dict[str, Any]:
    trace = journal_trace(task)
    evidence: dict[str, Any] = {"trace_fields": sorted(trace)}
    for key in (
        "executed_step_results",
        "task_observations",
        "action_dispatch_claims",
        "mutation_ledger",
        "delivery_receipts",
        "task_metrics",
    ):
        if key in trace:
            evidence[key] = compact_field(trace[key], max_chars)
    return evidence


def evaluate_workspace_assertions(
    case: dict[str, Any], max_chars: int
) -> tuple[dict[str, bool], list[dict[str, Any]]]:
    assertions = case.get("workspace_assertions") or []
    if not assertions:
        return {}, []
    root_raw = os.environ.get("NL_ISOLATED_WORKSPACE", "").strip()
    if not root_raw:
        return {"workspace_root_available": False}, []
    root = Path(root_raw).resolve()
    checks: dict[str, bool] = {"workspace_root_available": True}
    evidence: list[dict[str, Any]] = []
    for index, assertion in enumerate(assertions, 1):
        relative = Path(str(assertion.get("path") or ""))
        path = (root / relative).resolve()
        within_root = path == root or root in path.parents
        checks[f"workspace_{index}_within_root"] = within_root
        exists = within_root and path.is_file()
        checks[f"workspace_{index}_exists"] = exists == bool(assertion.get("exists", True))
        text = path.read_text(encoding="utf-8") if exists else ""
        checks[f"workspace_{index}_contains"] = all(
            token in text for token in assertion.get("contains", [])
        )
        checks[f"workspace_{index}_absent"] = all(
            token not in text for token in assertion.get("absent", [])
        )
        evidence.append(
            {
                "path": str(relative),
                "exists": exists,
                "content": compact_field(text, max_chars),
            }
        )
    return checks, evidence


def print_llm_calls(calls: list[dict[str, Any]], log_path: Path) -> None:
    for call in calls:
        print(f"  [{call['llm_call_ref']}] raw_fields={json.dumps(call, ensure_ascii=False)}")
        print(f"    full_log={log_path.resolve()} offset={call.get('log_offset')}")


def run_case(
    client: ApiClient,
    suite: dict[str, Any],
    case: dict[str, Any],
    ordinal: int,
    run_id: str,
    model_io_log: Path,
    timeout_seconds: float,
    poll_seconds: float,
    max_print_chars: int,
) -> dict[str, Any]:
    conversation_id = f"live-steering-{case['id']}-{uuid.uuid4().hex[:10]}"
    case_started = time.monotonic()
    print(f"[CASE {ordinal}] run_id={run_id} suite={suite['suite']} id={case['id']} family={case['family']} language={case['language']}")
    print(f"  provider={suite.get('provider')} model={suite.get('model')} dry_run=false skip=false")
    print(f"  NL#1={case['initial_input']}")
    submit_started = time.monotonic()
    status, response = client.request(
        "POST",
        "/conversation-inputs/client-task",
        client_task_body(case["initial_input"], conversation_id, f"{run_id}-{case['id']}-1"),
    )
    initial_accept_ms = round((time.monotonic() - submit_started) * 1000, 3)
    first = response_data(response, "initial_input")
    if status != 202:
        raise RuntimeError(f"initial_input_http_status:{status}")
    task_id = first["input"]["target_task_id"]
    _, running_polls = wait_for_status(
        client, task_id, {"running"}, timeout_seconds, poll_seconds
    )
    followup_ids: list[str] = []
    handoff_states: list[str] = []
    steering_accept_ms: list[float] = []
    operation_records: list[dict[str, Any]] = []
    for input_index, steering in enumerate(case["steering_inputs"], 2):
        if isinstance(steering, dict):
            text = str(steering["text"])
            injection_phase = str(steering.get("phase") or "running")
            minimum_step_results = int(steering.get("minimum_step_results") or 1)
            required_step_skills = {
                str(value) for value in steering.get("required_step_skills", [])
            }
            required_step_actions = {
                str(value) for value in steering.get("required_step_actions", [])
            }
            transport = str(steering.get("transport") or "conversation_input")
        else:
            text = str(steering)
            injection_phase = "running" if input_index == 2 else "immediate"
            minimum_step_results = 1
            required_step_skills = set()
            required_step_actions = set()
            transport = "conversation_input"
        phase_polls = running_polls
        if injection_phase != "immediate":
            _, phase_polls = wait_for_injection_phase(
                client,
                task_id,
                injection_phase,
                minimum_step_results,
                required_step_skills,
                required_step_actions,
                timeout_seconds,
                poll_seconds,
            )
        print(
            f"  injection_phase={injection_phase} transport={transport} "
            f"poll_count={phase_polls} "
            f"NL#{input_index}={text}"
        )
        submit_started = time.monotonic()
        request_id = f"{run_id}-{case['id']}-{input_index}"
        if transport == "conversation_input":
            status, response = client.request(
                "POST",
                "/conversation-inputs/client-task",
                client_task_body(text, conversation_id, request_id),
            )
            handoff = response_data(response, f"steering_input_{input_index}")
            if status != 202:
                raise RuntimeError(f"steering_input_http_status:{status}")
            if handoff["input"].get("target_task_id") != task_id:
                raise RuntimeError("steering_input_bound_to_different_task")
            followup_ids.append(handoff["input"]["input_id"])
            handoff_states.append(handoff["handoff_state"])
            operation_result = {
                "input_id": handoff["input"]["input_id"],
                "handoff_state": handoff["handoff_state"],
            }
        elif transport == "resume":
            status, response = client.request(
                "POST",
                "/tasks/resume-by-task-id",
                resume_task_body(task_id, steering, request_id),
            )
            operation_result = response_data(response, f"resume_input_{input_index}")
            if status != 200:
                raise RuntimeError(f"resume_input_http_status:{status}")
        else:
            raise RuntimeError(f"unsupported_steering_transport:{transport}")
        accept_ms = round((time.monotonic() - submit_started) * 1000, 3)
        steering_accept_ms.append(accept_ms)
        operation_records.append(
            {
                "input_index": input_index,
                "transport": transport,
                "injection_phase": injection_phase,
                "phase_poll_count": phase_polls,
                "accept_ms": accept_ms,
                "result": operation_result,
            }
        )
    task, terminal_polls = wait_for_expected_completion(
        client,
        task_id,
        case.get("expected") or {},
        timeout_seconds,
        poll_seconds,
    )
    receipts: list[dict[str, Any]] = []
    for input_id in followup_ids:
        status, response = client.request("GET", f"/conversation-inputs/{input_id}")
        record = response_data(response, "conversation_input_record")
        if status != 200:
            raise RuntimeError(f"conversation_input_get_http_status:{status}")
        receipts.append(record["receipt"])
    rows = model_rows(model_io_log, task_id)
    calls = merged_llm_calls(task, rows, max_print_chars)
    raw_relations = [relation for row in rows if (relation := response_relation(row))]
    relations = accepted_planner_relations(task)
    tool_calls = [call for row in rows for call in parsed_tool_calls(row)]
    checks = evaluate(
        case,
        task,
        receipts,
        handoff_states,
        calls,
        relations,
        tool_calls,
        operation_records,
    )
    workspace_checks, workspace_evidence = evaluate_workspace_assertions(
        case, max_print_chars
    )
    checks.update(workspace_checks)
    result = {
        "run_id": run_id,
        "suite": suite["suite"],
        "case_id": case["id"],
        "family": case["family"],
        "language": case["language"],
        "task_id": task_id,
        "conversation_id": conversation_id,
        "running_poll_count": running_polls,
        "terminal_poll_count": terminal_polls,
        "handoff_states": handoff_states,
        "followup_receipts": receipts,
        "operation_records": operation_records,
        "planner_relations": relations,
        "raw_planner_relations": raw_relations,
        "planner_tool_calls": tool_calls,
        "executed_action_refs": [
            str(step.get("requested_action_ref") or "")
            for step in task_step_results(task)
            if str(step.get("status") or "") == "ok"
        ],
        "trace_evidence": trace_evidence(task, max_print_chars),
        "workspace_evidence": workspace_evidence,
        "visible_text": visible_text(task),
        "task_status": task.get("status"),
        "checks": checks,
        "passed": all(checks.values()),
        "llm_request_count": len(calls),
        "llm_calls": calls,
        "metrics": {
            "initial_accept_ms": initial_accept_ms,
            "steering_accept_ms": steering_accept_ms,
            "elapsed_ms": round((time.monotonic() - case_started) * 1000, 3),
        },
        "model_io_log": str(model_io_log.resolve()),
    }
    print(f"  task_id={task_id} input_ids={json.dumps(followup_ids)} revisions={json.dumps([r.get('instruction_revision') for r in receipts])}")
    print(f"  final_status={task.get('status')} visible_text={json.dumps(result['visible_text'], ensure_ascii=False)}")
    print(f"  checks={json.dumps(checks, ensure_ascii=False, sort_keys=True)}")
    print(f"  llm_request_count={len(calls)}")
    print(f"  planner_relations={json.dumps(relations, ensure_ascii=False)} raw_planner_relations={json.dumps(raw_relations, ensure_ascii=False)}")
    print(f"  planner_tool_calls={json.dumps(tool_calls, ensure_ascii=False)}")
    print(f"  trace_evidence={json.dumps(result['trace_evidence'], ensure_ascii=False)}")
    print(f"  metrics={json.dumps(result['metrics'], ensure_ascii=False)}")
    print_llm_calls(calls, model_io_log)
    print(f"  result={'PASS' if result['passed'] else 'FAIL'}")
    return result


def main() -> int:
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(line_buffering=True)
    args = parse_args()
    if args.selected_only and args.exclude_selected:
        raise SystemExit("--selected-only and --exclude-selected are mutually exclusive")
    suite = json.loads(args.cases.read_text(encoding="utf-8"))
    if suite.get("schema_version") != 1 or not isinstance(suite.get("cases"), list):
        raise SystemExit("invalid live steering case schema")
    args.model_io_log.parent.mkdir(parents=True, exist_ok=True)
    args.model_io_log.touch(exist_ok=True)
    if not args.auth_key:
        raise SystemExit("--auth-key or APP_USER_KEY is required")
    run_id = time.strftime("run_%Y%m%d_%H%M%S")
    run_dir = args.run_root / run_id
    run_dir.mkdir(parents=True, exist_ok=False)
    client = ApiClient(args.base_url, args.auth_key, args.timeout_seconds)
    cases = [
        case
        for case in suite["cases"]
        if (not args.selected_only or case.get("selected") is True)
        and (not args.exclude_selected or case.get("selected") is not True)
        and (not args.case_id or case.get("id") in args.case_id)
    ]
    if args.case_limit is not None:
        cases = cases[: max(0, args.case_limit)]
    if not cases:
        raise SystemExit("no live steering cases selected")
    results: list[dict[str, Any]] = []
    for ordinal, case in enumerate(cases, 1):
        try:
            result = run_case(
                client,
                suite,
                case,
                ordinal,
                run_id,
                args.model_io_log,
                args.timeout_seconds,
                args.poll_seconds,
                args.max_print_chars,
            )
        except Exception as error:  # The run record must survive one failed case.
            result = {
                "run_id": run_id,
                "suite": suite["suite"],
                "case_id": case.get("id"),
                "passed": False,
                "error": f"{type(error).__name__}:{error}",
            }
            print(f"  result=FAIL error={result['error']}")
        results.append(result)
        (run_dir / f"{ordinal:03d}_{case['id']}.json").write_text(
            json.dumps(result, ensure_ascii=False, indent=2) + "\n",
            encoding="utf-8",
        )
    report = {
        "schema_version": 1,
        "run_id": run_id,
        "suite": suite["suite"],
        "case_count": len(results),
        "passed": sum(1 for result in results if result.get("passed")),
        "failed": sum(1 for result in results if not result.get("passed")),
        "results": results,
    }
    elapsed_values = [
        float(result["metrics"]["elapsed_ms"])
        for result in results
        if isinstance(result.get("metrics"), dict)
        and isinstance(result["metrics"].get("elapsed_ms"), (int, float))
    ]
    initial_accept_values = [
        float(result["metrics"]["initial_accept_ms"])
        for result in results
        if isinstance(result.get("metrics"), dict)
        and isinstance(result["metrics"].get("initial_accept_ms"), (int, float))
    ]
    steering_accept_values = [
        float(value)
        for result in results
        if isinstance(result.get("metrics"), dict)
        for value in result["metrics"].get("steering_accept_ms", [])
        if isinstance(value, (int, float))
    ]
    report["metrics"] = {
        "elapsed_ms_total": round(sum(elapsed_values), 3),
        "elapsed_ms_p50": percentile(elapsed_values, 50),
        "elapsed_ms_p95": percentile(elapsed_values, 95),
        "initial_accept_ms_p50": percentile(initial_accept_values, 50),
        "initial_accept_ms_p95": percentile(initial_accept_values, 95),
        "steering_accept_ms_p50": percentile(steering_accept_values, 50),
        "steering_accept_ms_p95": percentile(steering_accept_values, 95),
        "llm_request_count": sum(
            int(result.get("llm_request_count") or 0) for result in results
        ),
    }
    report_path = run_dir / "report.json"
    report_path.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"[SUMMARY] {json.dumps({key: report[key] for key in ('run_id','suite','case_count','passed','failed')}, ensure_ascii=False)}")
    print(f"[SUMMARY] report={report_path.resolve()}")
    return 0 if report["failed"] == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
