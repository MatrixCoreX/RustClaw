#!/usr/bin/env python3
"""Build one manual NL-suite result row from structured task evidence."""
from __future__ import annotations

import json
import re
import sys
import tomllib
from functools import lru_cache
from pathlib import Path
from typing import Any

from manual_workspace_assertions import workspace_file_cycle_assertion
from manual_execution_assertions import step_contract_assertion
from manual_trace_evidence import restore_execution_streams

try:
    import yaml
except ImportError as exc:  # pragma: no cover - environment preflight
    raise RuntimeError(
        "NL structural assertions require PyYAML; install scripts/nl_tests/requirements.txt"
    ) from exc


_MISSING = object()
_CALL_ACTION_TYPES = {"call_capability", "call_tool", "call_skill"}
_PLANNER_INTERNAL_DISCOVERY_CALLS = {"load_capability_groups"}


@lru_cache(maxsize=1)
def registry_capability_aliases() -> dict[str, str]:
    path = Path(__file__).resolve().parents[2] / "configs/skills_registry.toml"
    aliases = {}
    for skill in tomllib.loads(path.read_text())["skills"]:
        for alias, target in skill.get("planner_capability_aliases", {}).items():
            if alias in aliases and aliases[alias] != target:
                raise ValueError("conflicting registry capability alias")
            aliases[alias] = target
    return aliases


def canonical_capability(name: str) -> str:
    aliases = registry_capability_aliases()
    seen = set()
    while name in aliases:
        if name in seen:
            raise ValueError("cyclic registry capability alias")
        seen.add(name)
        name = aliases[name]
    return name


def step_matches_capability(step: dict[str, Any], name: str) -> bool:
    expected = canonical_capability(name)
    return any(canonical_capability(str(step.get(field) or "")) == expected
               for field in ("requested_capability", "resolved_capability"))


def json_pointer_get(root: Any, pointer: str) -> Any:
    if pointer == "":
        return root
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


def json_value_matches_expected(actual: Any, expected: str) -> bool:
    if actual is _MISSING:
        return False
    try:
        expected_value = json.loads(expected)
    except json.JSONDecodeError:
        return isinstance(actual, str) and actual == expected
    try:
        # JSON object order is insignificant; array order and value types are not.
        options = {"ensure_ascii": False, "sort_keys": True,
                   "separators": (",", ":"), "allow_nan": False}
        return json.dumps(actual, **options) == json.dumps(expected_value, **options)
    except (TypeError, ValueError):
        return False


def boolean_tag(tags: str, name: str) -> bool | None:
    match = re.search(
        rf"(?:^|[,;])\s*{re.escape(name)}\s*[:=]\s*(true|false)\s*(?=$|[,;])",
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


def counted_capability_tags(tags: str, name: str) -> list[tuple[str, int]]:
    return [
        (capability, int(count))
        for capability, count in re.findall(
            rf"(?:^|[,;])\s*{re.escape(name)}:([a-z0-9_.-]+)=(\d+)\s*(?=$|[,;])",
            tags,
            flags=re.IGNORECASE,
        )
    ]


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


def task_observations(result: dict[str, Any]) -> list[dict[str, Any]]:
    trace = task_journal(result).get("trace")
    if not isinstance(trace, dict):
        return []
    observations = trace.get("task_observations")
    if not isinstance(observations, list):
        return []
    return [observation for observation in observations if isinstance(observation, dict)]


def actual_call_steps(result: dict[str, Any]) -> list[dict[str, Any]]:
    calls = []
    for step in step_results(result):
        action_type = str(step.get("requested_action_type") or step.get("action_kind") or "")
        subject = str(
            step.get("requested_capability")
            or step.get("resolved_capability")
            or step.get("resolved_tool_or_skill")
            or step.get("skill")
            or ""
        )
        executed = step.get("executed_skill") or step.get("resolved_tool_or_skill")
        if (action_type in _CALL_ACTION_TYPES
                and subject not in _PLANNER_INTERNAL_DISCOVERY_CALLS
                and executed not in _PLANNER_INTERNAL_DISCOVERY_CALLS
                and executed not in {"respond", "synthesize_answer"}):
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
    preview_true = False
    writes_false = False
    no_output_write_verified = False
    for item in items:
        if not isinstance(item, dict):
            continue
        field = str(item.get("field") or "")
        excerpt = item.get("excerpt")
        normalized_excerpt = str(excerpt).lower()
        if (field == "preview" or field.endswith(".preview")) and (
            excerpt is True or normalized_excerpt == "true"
        ):
            preview_true = True
        if (field == "writes_performed" or field.endswith(".writes_performed")) and (
            excerpt is False or normalized_excerpt in {"false", "0"}
        ):
            writes_false = True
        if field.endswith(".validation.checks") and isinstance(item.get("sample_values"), list):
            no_output_write_verified = "no_output_written" in item["sample_values"]
        if field != "dry_run" and not field.endswith(".dry_run"):
            continue
        if excerpt is True or normalized_excerpt == "true":
            return True
    return preview_true and (writes_false or no_output_write_verified)


def step_has_structured_mutation(step: dict[str, Any]) -> bool:
    if step.get("structured_workspace_mutation") is not None or step.get("mutation_id") is not None:
        return True
    action_ref = str(
        step.get("requested_action_ref")
        or step.get("requested_capability")
        or step.get("resolved_capability")
        or ""
    )
    if action_ref in {
        "filesystem.write_file",
        "filesystem.append_text",
        "filesystem.make_dir",
        "filesystem.remove_path",
        "workspace.apply_patch",
        "workspace.apply_child_patch",
        "workspace.revert_checkpoint",
    }:
        return True
    observed = step.get("observed_evidence")
    extractor = observed.get("extractor") if isinstance(observed, dict) else None
    source_action_ref = str(extractor.get("source_action_ref") or "") if isinstance(extractor, dict) else ""
    return source_action_ref in {
        "fs_basic.write_text",
        "fs_basic.make_dir",
        "fs_basic.remove_path",
        "workspace.apply_patch",
    }


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
                observed_field = str(item.get("field") or "")
                if composite_observed_value_matches(
                    items,
                    observed_field,
                    actual_value,
                ):
                    return True
            if item.get("kind") == "array" and isinstance(actual_value, list):
                count = item.get("count")
                if isinstance(count, int) and not isinstance(count, bool) and count == len(actual_value):
                    return True
    return False


def composite_observed_value_matches(
    items: list[Any],
    observed_field: str,
    actual_value: Any,
) -> bool:
    exact_items = [
        item
        for item in items
        if isinstance(item, dict) and str(item.get("field") or "") == observed_field
    ]
    if isinstance(actual_value, dict):
        if not actual_value:
            return False
        object_items = [item for item in exact_items if item.get("kind") == "object"]
        if not object_items:
            return False
        if not any(
            isinstance(item.get("keys"), list)
            and set(actual_value).issubset(set(item["keys"]))
            for item in object_items
        ):
            return False
        return all(
            composite_observed_descendant_matches(
                items,
                f"{observed_field}.{key}",
                value,
            )
            for key, value in actual_value.items()
        )
    if isinstance(actual_value, list):
        array_items = [item for item in exact_items if item.get("kind") == "array"]
        return any(
            isinstance(item.get("count"), int)
            and not isinstance(item.get("count"), bool)
            and item["count"] == len(actual_value)
            for item in array_items
        )
    return False


def composite_observed_descendant_matches(
    items: list[Any],
    observed_field: str,
    actual_value: Any,
) -> bool:
    exact_items = [
        item
        for item in items
        if isinstance(item, dict) and str(item.get("field") or "") == observed_field
    ]
    if isinstance(actual_value, dict):
        object_items = [item for item in exact_items if item.get("kind") == "object"]
        if object_items and not any(
            isinstance(item.get("keys"), list)
            and set(actual_value).issubset(set(item["keys"]))
            for item in object_items
        ):
            return False
        return all(
            composite_observed_descendant_matches(
                items,
                f"{observed_field}.{key}",
                value,
            )
            for key, value in actual_value.items()
        )
    if isinstance(actual_value, list):
        array_items = [item for item in exact_items if item.get("kind") == "array"]
        if array_items and not any(
            isinstance(item.get("count"), int)
            and not isinstance(item.get("count"), bool)
            and item["count"] == len(actual_value)
            for item in array_items
        ):
            return False
        return all(
            composite_observed_descendant_matches(
                items,
                f"{observed_field}[{index}]",
                value,
            )
            for index, value in enumerate(actual_value)
        )
    excerpts = [
        value_to_compare_text(item.get("excerpt", _MISSING))
        for item in exact_items
        if item.get("excerpt", _MISSING) is not _MISSING
    ]
    excerpts = [excerpt for excerpt in excerpts if excerpt is not None]
    return not excerpts or value_to_compare_text(actual_value) in excerpts


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
        try:
            complete_output = json.loads(step.get("output_excerpt") or "")
        except (json.JSONDecodeError, TypeError):
            complete_output = _MISSING
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
            # Human-readable evidence excerpts trim whitespace. Prefer the exact
            # JSON value when a complete structured output is available.
            parts = re.sub(r"\[(\d+)\]", r".\1", observed_field).split(".")
            pointer = "/" + "/".join(part.replace("~", "~0").replace("/", "~1") for part in parts)
            exact = json_pointer_get(complete_output, pointer)
            value = value_to_compare_text(
                item.get("excerpt", _MISSING) if exact is _MISSING else exact
            )
            if value is not None and value not in values:
                values.append(value)
    return values


def observed_machine_field_exists(result: dict[str, Any], field: str) -> bool:
    for step in actual_call_steps(result):
        observed = step.get("observed_evidence")
        items = observed.get("items") if isinstance(observed, dict) else None
        if not isinstance(items, list):
            continue
        for item in items:
            if not isinstance(item, dict):
                continue
            observed_leaf = str(item.get("field") or "").rsplit(".", maxsplit=1)[-1]
            if observed_leaf == field:
                return True
    return False


def successful_call_step(step: dict[str, Any]) -> bool:
    if step.get("status") != "ok" or step.get("error_code"):
        return False
    # Transport completion is not domain success. Inspect only envelope fields,
    # never status columns inside returned records or user-visible error prose.
    observed = step.get("observed_evidence")
    items = observed.get("items", []) if isinstance(observed, dict) else []
    for item in items:
        if not isinstance(item, dict):
            continue
        if item.get("field") in {"status", "extra.status", "data.extra.status"}:
            value = item.get("excerpt")
            if isinstance(value, str) and value in {"error", "failed", "canceled"}:
                return False
    return True


def structural_assertions(
    tags: str,
    text: str,
    result: dict[str, Any],
    harness_evidence: dict[str, Any],
) -> list[dict[str, Any]]:
    details: list[dict[str, Any]] = []
    calls = actual_call_steps(result)
    successful_calls = [step for step in calls if successful_call_step(step)]
    alternatives = token_tags(tags, "any_successful_capability")
    if alternatives:
        matches = [step for step in successful_calls if
                   any(step_matches_capability(step, name) for name in alternatives)]
        details.append({"kind": "tag", "tag": "any_successful_capability",
                        "expected": alternatives, "matched_call_count": len(matches), "ok": bool(matches)})
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
            if step_matches_capability(step, required_capability)
        ]
        matched_resolutions = []
        if requires_tool_call is False:
            matched_resolutions = [
                observation
                for observation in task_observations(result)
                if observation.get("observation_kind") == "capability_resolution"
                and observation.get("outcome") == "resolved"
                and step_matches_capability(observation, required_capability)
            ]
        details.append(
            {
                "kind": "tag",
                "tag": "capability",
                "expected": required_capability,
                "matched_call_count": len(matched_steps),
                "matched_resolution_count": len(matched_resolutions),
                "ok": bool(matched_steps or matched_resolutions),
            }
        )

    forbidden_capabilities = token_tags(tags, "forbid_capability")
    for forbidden_capability in forbidden_capabilities:
        matched_steps = [
            step
            for step in calls
            if step_matches_capability(step, forbidden_capability)
        ]
        details.append(
            {
                "kind": "tag",
                "tag": "forbid_capability",
                "expected_absent": forbidden_capability,
                "matched_call_count": len(matched_steps),
                "ok": not matched_steps,
            }
        )

    for required_capability, minimum_count in counted_capability_tags(
        tags,
        "min_successful_capability_calls",
    ):
        matched_steps = [
            step
            for step in successful_calls
            if step_matches_capability(step, required_capability)
        ]
        details.append(
            {
                "kind": "tag",
                "tag": "min_successful_capability_calls",
                "expected_capability": required_capability,
                "expected_minimum": minimum_count,
                "actual_successful_count": len(matched_steps),
                "ok": len(matched_steps) >= minimum_count,
            }
        )

    requires_dry_run_evidence = has_tag(tags, "dry_run") and requires_tool_call is True
    if requires_dry_run_evidence:
        dry_run_calls = [step for step in calls if step_has_structured_dry_run(step)]
        mutation_calls = [
            step for step in successful_calls if step_has_structured_mutation(step)
        ]
        details.append(
            {
                "kind": "tag",
                "tag": "dry_run",
                "expected": True,
                "structured_dry_run_call_count": len(dry_run_calls),
                "structured_mutation_call_count": len(mutation_calls),
                "actual_call_count": len(calls),
                "ok": bool(dry_run_calls) and not mutation_calls,
            }
        )

    if has_tag(tags, "no_external_side_effect"):
        side_effect_count = completed_side_effect_count(result)
        mutation_steps = [step for step in successful_calls if step_has_structured_mutation(step)]
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

    if boolean_tag(tags, "concurrent_health_probe") is True:
        health_ok = harness_evidence.get("health_ok") is True
        task_status = str(harness_evidence.get("task_status") or "")
        checkpoint_id = str(harness_evidence.get("checkpoint_id") or "")
        async_job_id = str(harness_evidence.get("async_job_id") or "")
        details.append(
            {
                "kind": "tag",
                "tag": "concurrent_health_probe",
                "expected": True,
                "health_ok": health_ok,
                "task_status": task_status,
                "checkpoint_id": checkpoint_id,
                "async_job_id": async_job_id,
                "elapsed_ms": harness_evidence.get("elapsed_ms"),
                "ok": (
                    harness_evidence.get("status") == "pass"
                    and health_ok
                    and task_status == "running"
                    and bool(checkpoint_id)
                    and bool(async_job_id)
                ),
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

    required_observed_fields = token_tags(tags, "observed_field")
    for required_field in required_observed_fields:
        details.append(
            {
                "kind": "tag",
                "tag": "observed_field",
                "expected": required_field,
                "ok": observed_machine_field_exists(result, required_field),
            }
        )

    return details


def skill_outcome_json_assertion(spec: str, text: str, result: dict[str, Any]) -> dict[str, Any]:
    detail: dict[str, Any] = {"kind": "skill_outcome_json", "ok": False}
    try:
        contract = json.loads(spec)
        final = json.loads(text)
    except (json.JSONDecodeError, TypeError):
        return detail
    if not isinstance(contract, dict) or not isinstance(final, dict):
        return detail
    if set(contract) != {"skill", "success_fields"}:
        return detail
    skill = contract["skill"]
    fields = contract["success_fields"]
    if not isinstance(skill, str) or not skill or not isinstance(fields, list) or not fields:
        return detail
    if not all(isinstance(field, str) and field for field in fields):
        return detail
    calls = [step for step in actual_call_steps(result)
             if (step.get("executed_skill") or step.get("resolved_tool_or_skill")) == skill]
    detail["skill"] = skill
    if final.get("status") != "error":
        detail["acceptance_path"] = "successful_execution"
        detail["ok"] = (all(field in final and final[field] is not None for field in fields)
                        and any(successful_call_step(step) for step in calls))
        return detail

    detail["acceptance_path"] = "provider_error"
    if not isinstance(final.get("error_code"), str) or not final["error_code"]:
        return detail
    if final.get("failure_phase") not in {"provider_request", "provider_rejected"}:
        return detail
    # Match every error field to one actual failed invocation, never to a
    # response-only step or fields assembled from unrelated observations.
    required = {"error_code", "failure_phase"}
    required.update(field for field in ("provider", "status_code") if field in final)
    for step in calls:
        if successful_call_step(step):
            continue
        single = {"task_journal": {"trace": {"step_results": [step]}}}
        if all(observed_machine_field_matches(single, field, value_to_compare_text(final[field]))
               for field in required):
            detail["ok"] = True
            break
    return detail


def evaluate_expectations(
    spec_text: str,
    tags: str,
    obj: dict[str, Any],
    final_status: str,
    text: str,
    result: dict[str, Any],
    harness_evidence: dict[str, Any],
) -> tuple[str, list[dict[str, Any]]]:
    spec_text = (spec_text or "").strip()
    details = structural_assertions(tags, text, result, harness_evidence)
    has_assertions = bool(spec_text or details)
    if not has_assertions:
        return "-", []

    allow_terminal_failure = has_tag(tags, "allow_terminal_failure")
    expected_statuses = {"succeeded", "failed"} if allow_terminal_failure else {"succeeded"}
    all_ok = final_status in expected_statuses
    if final_status not in expected_statuses:
        details.insert(
            0,
            {
                "kind": "status",
                "expected": "succeeded_or_failed"
                if allow_terminal_failure
                else "succeeded",
                "actual": final_status,
                "ok": False,
            },
        )

    for raw in [part.strip() for part in spec_text.split(";") if part.strip()]:
        if raw.startswith("step_contract_json:"):
            detail = step_contract_assertion(raw[len("step_contract_json:"):], actual_call_steps(result))
            details.append(detail)
            ok = detail["ok"]
        elif raw.startswith("workspace_file_cycle:"):
            successful = [step for step in actual_call_steps(result) if successful_call_step(step)]
            detail = workspace_file_cycle_assertion(raw[len("workspace_file_cycle:"):], result, successful)
            details.append(detail)
            ok = detail["ok"]
        elif raw.startswith("skill_outcome_json:"):
            detail = skill_outcome_json_assertion(raw[len("skill_outcome_json:"):], text, result)
            details.append(detail)
            ok = detail["ok"]
        elif raw.startswith("contains:"):
            needle = raw[len("contains:") :]
            ok = needle in text
            details.append({"kind": "contains", "value": needle, "ok": ok})
        elif raw.startswith("observed_eq:"):
            field, sep, expected = raw[len("observed_eq:"):].partition("=")
            values = observed_machine_field_values(result, field)
            ok = bool(field and sep) and expected in values
            details.append({"kind": "observed_eq", "field": field,
                            "expected": expected, "observed_values": values, "ok": ok})
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
        elif raw.startswith("result_text_json_records_eq:"):
            expected = raw[len("result_text_json_records_eq:"):]
            try:
                decoded = json.loads(str(result.get("text") or ""))
                expected_records = json.loads(expected)
            except json.JSONDecodeError:
                decoded = expected_records = _MISSING
            candidates = [decoded] if isinstance(decoded, list) else [
                decoded[key] for key in ("result", "output")
                if isinstance(decoded, dict) and key in decoded
            ]
            expected_text = value_to_compare_text(expected_records)
            actual_values = [value_to_compare_text(value) for value in candidates]
            ok = isinstance(expected_records, list) and bool(candidates) and all(
                isinstance(value, list) and value_to_compare_text(value) == expected_text
                for value in candidates
            )
            details.append({"kind": "result_text_json_records_eq", "expected": expected_text,
                            "actual_values": actual_values, "ok": ok})
        elif raw.startswith("result_text_json_eq:"):
            expr = raw[len("result_text_json_eq:") :]
            pointer, sep, expected = expr.partition("=")
            try:
                decoded_result_text = json.loads(str(result.get("text") or ""))
            except json.JSONDecodeError:
                decoded_result_text = _MISSING
            actual = json_pointer_get(decoded_result_text, pointer)
            actual_text = value_to_compare_text(actual)
            ok = bool(sep) and json_value_matches_expected(actual, expected)
            details.append(
                {
                    "kind": "result_text_json_eq",
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
    harness_evidence_path: str = "",
) -> dict[str, Any]:
    path = Path(final_json_path) if final_json_path else None
    if path is not None and path.is_file():
        obj = json.loads(path.read_text(encoding="utf-8"))
    else:
        obj = {}
    obj, trace_evidence = restore_execution_streams(obj, path, task_id) if path else (obj, None)
    data = obj.get("data") or {}
    result = data.get("result_json") or {}
    text = str(result.get("text") or "")
    messages = result.get("messages")
    visible_parts = [text, str(data.get("error_text") or "")]
    if isinstance(messages, list):
        visible_parts.extend(
            str(item.get("text") or "")
            for item in messages
            if isinstance(item, dict)
        )
    observable_text = "\n".join(part for part in visible_parts if part)
    evidence_path = Path(harness_evidence_path) if harness_evidence_path else None
    harness_evidence: dict[str, Any] = {}
    if evidence_path is not None and evidence_path.is_file():
        loaded_evidence = json.loads(evidence_path.read_text(encoding="utf-8"))
        if isinstance(loaded_evidence, dict):
            harness_evidence = loaded_evidence
    final_status = effective_status.strip() or str(data.get("status") or "")
    assertion, assertion_details = evaluate_expectations(
        expectation_spec,
        tags,
        obj,
        final_status,
        observable_text,
        result,
        harness_evidence,
    )
    if trace_evidence is not None:
        assertion_details.append(trace_evidence)
        if not trace_evidence["ok"]:
            assertion = "fail"

    return {
        "source_line": source_line,
        "case_name": case_name,
        "tags": tags,
        "mode": mode or "ask",
        "prompt": prompt,
        "task_id": task_id,
        "status": final_status,
        "text": text or None,
        "messages": messages,
        "error_text": data.get("error_text"),
        "harness_evidence": harness_evidence or None,
        "started_at": started_at,
        "ended_at": ended_at,
        "wall_seconds": max(0, ended_at - started_at) if ended_at and started_at else None,
        "efficiency": task_efficiency(result),
        "expect_substr": expectation_spec or None,
        "assertion": assertion,
        "assertion_details": assertion_details,
    }


def main(argv: list[str]) -> int:
    if len(argv) not in {12, 13}:
        print(
            "usage: manual_case_assertions.py SOURCE_LINE CASE_NAME TAGS PROMPT "
            "TASK_ID FINAL_JSON STATUS STARTED_AT ENDED_AT EXPECT MODE "
            "[HARNESS_EVIDENCE_JSON]",
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
        harness_evidence_path=argv[12] if len(argv) == 13 else "",
    )
    print(json.dumps(row, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
