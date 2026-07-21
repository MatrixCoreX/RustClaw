#!/usr/bin/env python3
"""Build one manual NL-suite result row from structured task evidence."""
from __future__ import annotations

import json
import re
import sys
from pathlib import Path
from typing import Any

try:
    import yaml
except ImportError as exc:  # pragma: no cover - environment preflight
    raise RuntimeError(
        "NL structural assertions require PyYAML; install scripts/nl_tests/requirements.txt"
    ) from exc


_MISSING = object()
_CALL_ACTION_TYPES = {"call_capability", "call_tool", "call_skill"}


def json_pointer_get(root: Any, pointer: str) -> Any:
    if not pointer.startswith("/"):
        return _MISSING
    value = root
    for raw_part in pointer.split("/")[1:]:
        part = raw_part.replace("~1", "/").replace("~0", "~")
        if isinstance(value, dict):
            if part not in value:
                return _MISSING
            value = value[part]
        elif isinstance(value, list):
            try:
                index = int(part)
            except ValueError:
                return _MISSING
            if index < 0 or index >= len(value):
                return _MISSING
            value = value[index]
        else:
            return _MISSING
    return value


def value_to_compare_text(value: Any) -> str | None:
    if value is _MISSING:
        return None
    if isinstance(value, bool):
        return "true" if value else "false"
    if value is None:
        return "null"
    if isinstance(value, (int, float, str)):
        return str(value)
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"))


def boolean_tag(tags: str, name: str) -> bool | None:
    match = re.search(
        rf"(?:^|[,;])\s*{re.escape(name)}\s*=\s*(true|false)\s*(?=$|[,;])",
        tags,
        flags=re.IGNORECASE,
    )
    if match is None:
        return None
    return match.group(1).lower() == "true"


def has_tag(tags: str, name: str) -> bool:
    return (
        re.search(
            rf"(?:^|[,;])\s*{re.escape(name)}\s*(?=$|[,;])",
            tags,
            flags=re.IGNORECASE,
        )
        is not None
    )


def token_tags(tags: str, name: str) -> list[str]:
    return re.findall(
        rf"(?:^|[,;])\s*{re.escape(name)}:([a-z0-9_.-]+)\s*(?=$|[,;])",
        tags,
        flags=re.IGNORECASE,
    )


def task_journal(result: dict[str, Any]) -> dict[str, Any]:
    journal = result.get("task_journal")
    return journal if isinstance(journal, dict) else {}


def step_results(result: dict[str, Any]) -> list[dict[str, Any]]:
    trace = task_journal(result).get("trace")
    if not isinstance(trace, dict):
        return []
    steps = trace.get("step_results")
    if not isinstance(steps, list):
        return []
    return [step for step in steps if isinstance(step, dict)]


def actual_call_steps(result: dict[str, Any]) -> list[dict[str, Any]]:
    calls = []
    for step in step_results(result):
        action_type = str(step.get("requested_action_type") or step.get("action_kind") or "")
        if action_type in _CALL_ACTION_TYPES:
            calls.append(step)
    return calls


def step_has_structured_dry_run(step: dict[str, Any]) -> bool:
    action_ref = str(
        step.get("requested_action_ref")
        or step.get("requested_capability")
        or step.get("resolved_capability")
        or ""
    )
    action = action_ref.rsplit(".", maxsplit=1)[-1]
    if action == "preview" or action.startswith("preview_"):
        return True
    observed = step.get("observed_evidence")
    if not isinstance(observed, dict):
        return False
    items = observed.get("items")
    if not isinstance(items, list):
        return False
    for item in items:
        if not isinstance(item, dict):
            continue
        field = str(item.get("field") or "")
        if field != "dry_run" and not field.endswith(".dry_run"):
            continue
        excerpt = item.get("excerpt")
        if excerpt is True or str(excerpt).lower() == "true":
            return True
    return False


def completed_side_effect_count(result: dict[str, Any]) -> int:
    summary = task_journal(result).get("summary")
    if not isinstance(summary, dict):
        return 0
    workflow = summary.get("coding_workflow")
    if not isinstance(workflow, dict):
        return 0
    value = workflow.get("completed_side_effect_count")
    return value if isinstance(value, int) and not isinstance(value, bool) else 0


def final_text_has_machine_field(text: str, field: str) -> bool:
    return final_text_machine_field_value(text, field) is not None


def unwrap_markdown_scalar(value: str) -> str:
    normalized = value.strip()
    for wrapper in ("`", "**", "__"):
        if (
            normalized.startswith(wrapper)
            and normalized.endswith(wrapper)
            and len(normalized) > len(wrapper) * 2
        ):
            return normalized[len(wrapper) : -len(wrapper)].strip()
    return normalized


def structured_mapping_from_text(text: str) -> dict[str, Any] | None:
    candidates = [text.strip()]
    candidates.extend(
        match.group(1).strip()
        for match in re.finditer(
            r"```(?:json|ya?ml)?\s*\n(.*?)\n```",
            text,
            flags=re.IGNORECASE | re.DOTALL,
        )
    )
    for candidate in candidates:
        if not candidate:
            continue
        try:
            parsed = json.loads(candidate)
        except (json.JSONDecodeError, TypeError):
            try:
                parsed = yaml.safe_load(candidate)
            except yaml.YAMLError:
                continue
        if isinstance(parsed, dict):
            return parsed
    return None


def final_text_machine_field_value(text: str, field: str) -> str | None:
    parsed = structured_mapping_from_text(text)
    if parsed is not None and field in parsed:
        return value_to_compare_text(parsed[field])
    match = re.search(
        rf"(?m)^\s*(?:[-*+]\s+)?(?:\*\*|__|`)?{re.escape(field)}(?:\*\*|__|`)?\s*:\s*(.*?)\s*$",
        text,
    )
    if match is not None:
        return unwrap_markdown_scalar(match.group(1))
    match = re.search(
        rf"(?:^|[;\s]){re.escape(field)}\s*=\s*(.*?)(?=\s+[a-zA-Z0-9_.-]+\s*=|$)",
        text,
    )
    return unwrap_markdown_scalar(match.group(1)) if match is not None else None


def observed_machine_field_matches(
    result: dict[str, Any],
    field: str,
    actual: str | None,
) -> bool:
    if actual is None:
        return False
    if actual in observed_machine_field_values(result, field):
        return True
    try:
        actual_value = json.loads(actual)
    except (json.JSONDecodeError, TypeError):
        return False
    field_aliases = {
        "path": {"path", "resolved_path", "effective_path"},
    }
    accepted_fields = field_aliases.get(field, {field})
    for step in actual_call_steps(result):
        observed = step.get("observed_evidence")
        items = observed.get("items") if isinstance(observed, dict) else None
        if not isinstance(items, list):
            continue
        for item in items:
            if not isinstance(item, dict):
                continue
            observed_leaf = str(item.get("field") or "").rsplit(".", maxsplit=1)[-1]
            if observed_leaf not in accepted_fields:
                continue
            if item.get("kind") == "object" and isinstance(actual_value, dict):
                keys = item.get("keys")
                if isinstance(keys, list) and all(key in actual_value for key in keys):
                    return True
            if item.get("kind") == "array" and isinstance(actual_value, list):
                count = item.get("count")
                if isinstance(count, int) and not isinstance(count, bool) and count == len(actual_value):
                    return True
    return False


def observed_machine_field_values(
    result: dict[str, Any],
    field: str,
) -> list[str]:
    field_aliases = {
        "path": {"path", "resolved_path", "effective_path"},
    }
    accepted_fields = field_aliases.get(field, {field})
    values: list[str] = []
    for step in actual_call_steps(result):
        observed = step.get("observed_evidence")
        items = observed.get("items") if isinstance(observed, dict) else None
        if not isinstance(items, list):
            continue
        for item in items:
            if not isinstance(item, dict):
                continue
            observed_field = str(item.get("field") or "")
            observed_leaf = observed_field.rsplit(".", maxsplit=1)[-1]
            if observed_leaf not in accepted_fields:
                continue
            value = value_to_compare_text(item.get("excerpt", _MISSING))
            if value is not None and value not in values:
                values.append(value)
    return values


def structural_assertions(
    tags: str,
    text: str,
    result: dict[str, Any],
) -> list[dict[str, Any]]:
    details: list[dict[str, Any]] = []
    calls = actual_call_steps(result)
    successful_calls = [step for step in calls if step.get("status") == "ok"]
    requires_tool_call = boolean_tag(tags, "requires_tool_call")

    if requires_tool_call is not None:
        ok = bool(calls) if requires_tool_call else not calls
        details.append(
            {
                "kind": "tag",
                "tag": "requires_tool_call",
                "expected": requires_tool_call,
                "actual_call_count": len(calls),
                "successful_call_count": len(successful_calls),
                "ok": ok,
            }
        )

    required_capabilities = token_tags(tags, "capability")
    for required_capability in required_capabilities:
        matched_steps = [
            step
            for step in calls
            if required_capability
            in {
                str(step.get("requested_capability") or ""),
                str(step.get("resolved_capability") or ""),
            }
        ]
        details.append(
            {
                "kind": "tag",
                "tag": "capability",
                "expected": required_capability,
                "matched_call_count": len(matched_steps),
                "ok": bool(matched_steps),
            }
        )

    requires_dry_run_evidence = has_tag(tags, "dry_run") and requires_tool_call is True
    if requires_dry_run_evidence:
        dry_run_calls = [step for step in calls if step_has_structured_dry_run(step)]
        details.append(
            {
                "kind": "tag",
                "tag": "dry_run",
                "expected": True,
                "structured_dry_run_call_count": len(dry_run_calls),
                "actual_call_count": len(calls),
                "ok": bool(dry_run_calls),
            }
        )

    if has_tag(tags, "no_external_side_effect"):
        side_effect_count = completed_side_effect_count(result)
        mutation_steps = [
            step
            for step in calls
            if step.get("structured_workspace_mutation") is not None
            or step.get("mutation_id") is not None
        ]
        details.append(
            {
                "kind": "tag",
                "tag": "no_external_side_effect",
                "expected": True,
                "completed_side_effect_count": side_effect_count,
                "mutation_step_count": len(mutation_steps),
                "ok": side_effect_count == 0 and not mutation_steps,
            }
        )

    required_final_fields = token_tags(tags, "final_field")
    for required_field in required_final_fields:
        details.append(
            {
                "kind": "tag",
                "tag": "final_field",
                "expected": required_field,
                "ok": final_text_has_machine_field(text, required_field),
            }
        )

    observed_final_fields = token_tags(tags, "final_observed_field")
    for required_field in observed_final_fields:
        actual = final_text_machine_field_value(text, required_field)
        observed_values = observed_machine_field_values(result, required_field)
        details.append(
            {
                "kind": "tag",
                "tag": "final_observed_field",
                "expected": required_field,
                "actual": actual,
                "observed_values": observed_values,
                "ok": observed_machine_field_matches(result, required_field, actual),
            }
        )

    return details


def evaluate_expectations(
    spec_text: str,
    tags: str,
    obj: dict[str, Any],
    final_status: str,
    text: str,
    result: dict[str, Any],
) -> tuple[str, list[dict[str, Any]]]:
    spec_text = (spec_text or "").strip()
    details = structural_assertions(tags, text, result)
    has_assertions = bool(spec_text or details)
    if not has_assertions:
        return "-", []

    all_ok = final_status == "succeeded"
    if final_status != "succeeded":
        details.insert(
            0,
            {
                "kind": "status",
                "expected": "succeeded",
                "actual": final_status,
                "ok": False,
            },
        )

    for raw in [part.strip() for part in spec_text.split(";") if part.strip()]:
        if raw.startswith("contains:"):
            needle = raw[len("contains:") :]
            ok = needle in text
            details.append({"kind": "contains", "value": needle, "ok": ok})
        elif raw.startswith("json_exists:"):
            pointer = raw[len("json_exists:") :]
            ok = json_pointer_get(obj, pointer) is not _MISSING
            details.append({"kind": "json_exists", "pointer": pointer, "ok": ok})
        elif raw.startswith("json_eq:"):
            expr = raw[len("json_eq:") :]
            pointer, sep, expected = expr.partition("=")
            actual = json_pointer_get(obj, pointer)
            actual_text = value_to_compare_text(actual)
            ok = bool(sep) and actual_text == expected
            details.append(
                {
                    "kind": "json_eq",
                    "pointer": pointer,
                    "expected": expected,
                    "actual": actual_text,
                    "ok": ok,
                }
            )
        else:
            ok = raw in text
            details.append({"kind": "contains", "value": raw, "ok": ok})
        all_ok = all_ok and ok

    all_ok = all_ok and all(bool(detail.get("ok")) for detail in details)
    return ("pass" if all_ok else "fail"), details


def task_efficiency(result: dict[str, Any]) -> dict[str, Any]:
    journal = task_journal(result)
    summary = journal.get("summary")
    summary = summary if isinstance(summary, dict) else {}
    metrics = summary.get("task_metrics")
    metrics = metrics if isinstance(metrics, dict) else {}
    by_prompt = metrics.get("by_prompt")
    by_prompt = by_prompt if isinstance(by_prompt, dict) else {}

    prompt_bytes_after_max = 0
    provider_attempt_count = 0
    prompt_truncation_count = 0
    for prompt_metrics in by_prompt.values():
        if not isinstance(prompt_metrics, dict):
            continue
        prompt_bytes_after_max = max(
            prompt_bytes_after_max,
            int(prompt_metrics.get("prompt_bytes_after_max") or 0),
        )
        provider_attempt_count += int(prompt_metrics.get("provider_attempt_count") or 0)
        prompt_truncation_count += int(prompt_metrics.get("prompt_truncation_count") or 0)

    return {
        "round_count": summary.get("round_count"),
        "step_count": summary.get("step_count"),
        "llm_call_count": metrics.get("llm_calls_per_task"),
        "llm_elapsed_ms": metrics.get("llm_elapsed_ms_per_task"),
        "provider_attempt_count": provider_attempt_count,
        "prompt_bytes_after_max": prompt_bytes_after_max,
        "prompt_truncation_count": prompt_truncation_count,
    }


def build_summary_row(
    source_line: int,
    case_name: str,
    tags: str,
    prompt: str,
    task_id: str,
    final_json_path: str,
    effective_status: str,
    started_at: int,
    ended_at: int,
    expectation_spec: str,
    mode: str,
) -> dict[str, Any]:
    path = Path(final_json_path) if final_json_path else None
    if path is not None and path.is_file():
        obj = json.loads(path.read_text(encoding="utf-8"))
    else:
        obj = {}
    data = obj.get("data") or {}
    result = data.get("result_json") or {}
    text = str(result.get("text") or "")
    final_status = effective_status.strip() or str(data.get("status") or "")
    assertion, assertion_details = evaluate_expectations(
        expectation_spec,
        tags,
        obj,
        final_status,
        text,
        result,
    )

    return {
        "source_line": source_line,
        "case_name": case_name,
        "tags": tags,
        "mode": mode or "ask",
        "prompt": prompt,
        "task_id": task_id,
        "status": final_status,
        "text": text or None,
        "messages": result.get("messages"),
        "error_text": data.get("error_text"),
        "started_at": started_at,
        "ended_at": ended_at,
        "wall_seconds": max(0, ended_at - started_at) if ended_at and started_at else None,
        "efficiency": task_efficiency(result),
        "expect_substr": expectation_spec or None,
        "assertion": assertion,
        "assertion_details": assertion_details,
    }


def main(argv: list[str]) -> int:
    if len(argv) != 12:
        print(
            "usage: manual_case_assertions.py SOURCE_LINE CASE_NAME TAGS PROMPT "
            "TASK_ID FINAL_JSON STATUS STARTED_AT ENDED_AT EXPECT MODE",
            file=sys.stderr,
        )
        return 2
    row = build_summary_row(
        source_line=int(argv[1]),
        case_name=argv[2],
        tags=argv[3],
        prompt=argv[4],
        task_id=argv[5],
        final_json_path=argv[6],
        effective_status=argv[7],
        started_at=int(argv[8] or 0),
        ended_at=int(argv[9] or 0),
        expectation_spec=argv[10],
        mode=argv[11],
    )
    print(json.dumps(row, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
