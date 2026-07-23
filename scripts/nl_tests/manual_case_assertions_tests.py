#!/usr/bin/env python3
"""Tests for structured assertions used by the manual NL runner."""
from __future__ import annotations

import tempfile
from pathlib import Path

from manual_case_assertions import build_summary_row


def result_with_steps(
    steps: list[dict],
    completed_side_effect_count: int = 0,
    text: str = "machine result",
) -> dict:
    return {
        "ok": True,
        "data": {
            "status": "succeeded",
            "result_json": {
                "text": text,
                "task_journal": {
                    "summary": {
                        "round_count": 1,
                        "step_count": len(steps),
                        "coding_workflow": {
                            "completed_side_effect_count": completed_side_effect_count,
                        },
                        "task_metrics": {
                            "llm_calls_per_task": 2,
                            "llm_elapsed_ms_per_task": 120,
                            "by_prompt": {
                                "plan": {
                                    "prompt_bytes_after_max": 2048,
                                    "provider_attempt_count": 2,
                                    "prompt_truncation_count": 0,
                                }
                            },
                        },
                    },
                    "trace": {"step_results": steps},
                },
            },
        },
    }


def terminal_step() -> dict:
    return {
        "requested_action_type": "respond",
        "action_kind": "respond",
        "status": "ok",
    }


def capability_step(
    *,
    dry_run: bool = True,
    capability: str = "fixture.preview",
    observed_fields: dict[str, object] | None = None,
) -> dict:
    items = []
    if dry_run:
        items.append({"field": "dry_run", "excerpt": "true"})
    for field, value in (observed_fields or {}).items():
        items.append({"field": f"extra.{field}", "excerpt": value})
    return {
        "requested_action_type": "call_capability",
        "action_kind": "call_capability",
        "requested_capability": capability,
        "resolved_capability": capability,
        "status": "ok",
        "observed_evidence": {"items": items},
    }


def preview_capability_step_without_dry_run_evidence() -> dict:
    step = capability_step(dry_run=False, capability="fixture.preview_render")
    step["requested_action_ref"] = "fixture.preview_render"
    return step


def write_result(root: Path, name: str, value: dict) -> Path:
    import json

    path = root / name
    path.write_text(json.dumps(value), encoding="utf-8")
    return path


def row_for(path: Path, tags: str, expect: str = "contains:machine") -> dict:
    return build_summary_row(
        source_line=1,
        case_name="fixture",
        tags=tags,
        prompt="fixture",
        task_id="task-fixture",
        final_json_path=str(path),
        effective_status="succeeded",
        started_at=10,
        ended_at=12,
        expectation_spec=expect,
        mode="ask",
    )


def main() -> int:
    with tempfile.TemporaryDirectory(prefix="manual-case-assertions-") as tmp:
        root = Path(tmp)

        direct = write_result(root, "direct.json", result_with_steps([terminal_step()]))
        direct_row = row_for(
            direct,
            "covers:fixture,dry_run;requires_tool_call=true;dry_run,no_external_side_effect",
        )
        assert direct_row["assertion"] == "fail"
        assert direct_row["efficiency"]["llm_call_count"] == 2

        capability = write_result(
            root,
            "capability.json",
            result_with_steps([capability_step(), terminal_step()]),
        )
        capability_row = row_for(
            capability,
            "covers:fixture,dry_run,capability:fixture.preview;"
            "requires_tool_call=true;dry_run,no_external_side_effect",
        )
        assert capability_row["assertion"] == "pass"

        wrong_capability_row = row_for(
            capability,
            "capability:fixture.other;requires_tool_call=true;dry_run",
        )
        assert wrong_capability_row["assertion"] == "fail"

        missing_dry_run = write_result(
            root,
            "missing-dry-run.json",
            result_with_steps(
                [
                    capability_step(dry_run=False, capability="fixture.generate"),
                    terminal_step(),
                ]
            ),
        )
        missing_dry_run_row = row_for(
            missing_dry_run,
            "requires_tool_call=true;dry_run,no_external_side_effect",
        )
        assert missing_dry_run_row["assertion"] == "fail"

        preview_dry_run = write_result(
            root,
            "preview-dry-run.json",
            result_with_steps([preview_capability_step_without_dry_run_evidence()]),
        )
        preview_dry_run_row = row_for(
            preview_dry_run,
            "requires_tool_call=true;dry_run,no_external_side_effect",
        )
        assert preview_dry_run_row["assertion"] == "pass"

        no_tool = row_for(
            direct,
            "requires_tool_call=false;local_readonly",
        )
        assert no_tool["assertion"] == "pass"

        unexpected_tool = row_for(
            capability,
            "requires_tool_call=false;local_readonly",
        )
        assert unexpected_tool["assertion"] == "fail"

        side_effect = write_result(
            root,
            "side-effect.json",
            result_with_steps([capability_step()], completed_side_effect_count=1),
        )
        side_effect_row = row_for(
            side_effect,
            "requires_tool_call=true;dry_run,no_external_side_effect",
        )
        assert side_effect_row["assertion"] == "fail"

        misleading_field_value = write_result(
            root,
            "misleading-field-value.json",
            result_with_steps(
                [capability_step()],
                text='rewind_references=["dry_run:checkpoint:pre_patch"]',
            ),
        )
        misleading_field_row = row_for(
            misleading_field_value,
            "final_field:checkpoint;final_field:rewind_references",
            expect="",
        )
        assert misleading_field_row["assertion"] == "fail"

        complete_fields = write_result(
            root,
            "complete-fields.json",
            result_with_steps(
                [capability_step()],
                text='{"checkpoint":{},"rewind_references":[]}',
            ),
        )
        complete_fields_row = row_for(
            complete_fields,
            "final_field:checkpoint;final_field:rewind_references",
            expect="",
        )
        assert complete_fields_row["assertion"] == "pass"

        mismatched_observed_field = write_result(
            root,
            "mismatched-observed-field.json",
            result_with_steps(
                [capability_step(observed_fields={"line_count": 10})],
                text="line_count: 1",
            ),
        )
        mismatched_observed_field_row = row_for(
            mismatched_observed_field,
            "final_observed_field:line_count",
            expect="",
        )
        assert mismatched_observed_field_row["assertion"] == "fail"

        matched_observed_field = write_result(
            root,
            "matched-observed-field.json",
            result_with_steps(
                [capability_step(observed_fields={"line_count": 10})],
                text="line_count: 10",
            ),
        )
        matched_observed_field_row = row_for(
            matched_observed_field,
            "final_observed_field:line_count",
            expect="",
        )
        assert matched_observed_field_row["assertion"] == "pass"

        one_line_machine_fields = write_result(
            root,
            "one-line-machine-fields.json",
            result_with_steps(
                [
                    capability_step(
                        observed_fields={
                            "provider": "fixture",
                            "model": "fixture-v1",
                        }
                    )
                ],
                text="provider=fixture model=fixture-v1",
            ),
        )
        one_line_machine_fields_row = row_for(
            one_line_machine_fields,
            "final_observed_field:provider;final_observed_field:model",
            expect="",
        )
        assert one_line_machine_fields_row["assertion"] == "pass"

        composite_fields = write_result(
            root,
            "composite-fields.json",
            result_with_steps(
                [
                    {
                        **capability_step(dry_run=False),
                        "observed_evidence": {
                            "items": [
                                {
                                    "field": "extra.async_contract",
                                    "kind": "object",
                                    "keys": ["status", "poll_adapter"],
                                    "key_count": 2,
                                },
                                {
                                    "field": "extra.planned_outputs",
                                    "kind": "array",
                                    "count": 1,
                                },
                            ]
                        },
                    }
                ],
                text=(
                    'planned_outputs=[{"path":"out.mp4","type":"video_file"}] '
                    'async_contract={"poll_adapter":{"kind":"media_job_poll"},"status":"accepted"}'
                ),
            ),
        )
        composite_fields_row = row_for(
            composite_fields,
            "final_observed_field:planned_outputs;final_observed_field:async_contract",
            expect="",
        )
        assert composite_fields_row["assertion"] == "pass"

        redacted_object_subset = write_result(
            root,
            "redacted-object-subset.json",
            result_with_steps(
                [
                    {
                        **capability_step(dry_run=False),
                        "observed_evidence": {
                            "items": [
                                {
                                    "field": "extra.async_contract",
                                    "kind": "object",
                                    "keys": [
                                        "cancel_token",
                                        "poll_adapter",
                                        "provider",
                                        "status",
                                    ],
                                    "key_count": 4,
                                },
                                {
                                    "field": "extra.async_contract.cancel_token",
                                    "kind": "string",
                                    "redacted": True,
                                },
                                {
                                    "field": "extra.async_contract.poll_adapter",
                                    "kind": "object",
                                    "keys": ["kind"],
                                    "key_count": 1,
                                },
                                {
                                    "field": "extra.async_contract.poll_adapter.kind",
                                    "kind": "string",
                                    "excerpt": "media_job_poll",
                                },
                                {
                                    "field": "extra.async_contract.status",
                                    "kind": "string",
                                    "excerpt": "accepted",
                                },
                            ]
                        },
                    }
                ],
                text=(
                    'async_contract={"poll_adapter":{"kind":"media_job_poll"},'
                    '"status":"accepted"}'
                ),
            ),
        )
        redacted_object_subset_row = row_for(
            redacted_object_subset,
            "final_observed_field:async_contract",
            expect="",
        )
        assert redacted_object_subset_row["assertion"] == "pass"

        invented_object_field = write_result(
            root,
            "invented-object-field.json",
            result_with_steps(
                [
                    {
                        **capability_step(dry_run=False),
                        "observed_evidence": {
                            "items": [
                                {
                                    "field": "extra.async_contract",
                                    "kind": "object",
                                    "keys": ["status"],
                                    "key_count": 1,
                                }
                            ]
                        },
                    }
                ],
                text='async_contract={"invented":true}',
            ),
        )
        invented_object_field_row = row_for(
            invented_object_field,
            "final_observed_field:async_contract",
            expect="",
        )
        assert invented_object_field_row["assertion"] == "fail"

        mismatched_object_value = write_result(
            root,
            "mismatched-object-value.json",
            result_with_steps(
                [
                    {
                        **capability_step(dry_run=False),
                        "observed_evidence": {
                            "items": [
                                {
                                    "field": "extra.async_contract",
                                    "kind": "object",
                                    "keys": ["status"],
                                    "key_count": 1,
                                },
                                {
                                    "field": "extra.async_contract.status",
                                    "kind": "string",
                                    "excerpt": "accepted",
                                },
                            ]
                        },
                    }
                ],
                text='async_contract={"status":"completed"}',
            ),
        )
        mismatched_object_value_row = row_for(
            mismatched_object_value,
            "final_observed_field:async_contract",
            expect="",
        )
        assert mismatched_object_value_row["assertion"] == "fail"

        yaml_composite_fields = write_result(
            root,
            "yaml-composite-fields.json",
            result_with_steps(
                [
                    {
                        **capability_step(
                            dry_run=False,
                            observed_fields={"provider": "fixture"},
                        ),
                        "observed_evidence": {
                            "items": [
                                {
                                    "field": "extra.provider",
                                    "kind": "string",
                                    "excerpt": "fixture",
                                },
                                {
                                    "field": "extra.planned_outputs",
                                    "kind": "array",
                                    "count": 1,
                                },
                                {
                                    "field": "extra.async_contract",
                                    "kind": "object",
                                    "keys": ["status", "poll_adapter"],
                                    "key_count": 2,
                                },
                            ]
                        },
                    }
                ],
                text=(
                    "```yaml\n"
                    "provider: fixture\n"
                    "planned_outputs:\n"
                    "  - type: video_file\n"
                    "    path: out.mp4\n"
                    "async_contract:\n"
                    "  status: accepted\n"
                    "  poll_adapter:\n"
                    "    kind: media_job_poll\n"
                    "```\n\nRendered explanation."
                ),
            ),
        )
        yaml_composite_fields_row = row_for(
            yaml_composite_fields,
            "final_observed_field:provider;final_observed_field:planned_outputs;"
            "final_observed_field:async_contract",
            expect="",
        )
        assert yaml_composite_fields_row["assertion"] == "pass"

        normalized_path = write_result(
            root,
            "normalized-path.json",
            result_with_steps(
                [
                    capability_step(
                        observed_fields={
                            "path": "README.md",
                            "resolved_path": "/workspace/README.md",
                        }
                    )
                ],
                text="path: /workspace/README.md",
            ),
        )
        normalized_path_row = row_for(
            normalized_path,
            "final_observed_field:path",
            expect="",
        )
        assert normalized_path_row["assertion"] == "pass"

        markdown_machine_fields = write_result(
            root,
            "markdown-machine-fields.json",
            result_with_steps(
                [
                    capability_step(
                        observed_fields={
                            "path": "README.md",
                            "resolved_path": "/workspace/README.md",
                            "exists": False,
                        }
                    )
                ],
                text=(
                    "- **path**: `/workspace/README.md`\n"
                    "- `exists`: `false`\n"
                    "- error_code: path_not_found"
                ),
            ),
        )
        markdown_machine_fields_row = row_for(
            markdown_machine_fields,
            "final_observed_field:path;final_observed_field:exists;final_field:error_code",
            expect="path_not_found",
        )
        assert markdown_machine_fields_row["assertion"] == "pass"

        observed_container = result_with_steps(
            [capability_step(observed_fields={"namespaces": []})],
            text="No namespaces are available.",
        )
        observed_container["data"]["result_json"]["task_journal"]["trace"][
            "step_results"
        ][0]["observed_evidence"]["items"][-1] = {
            "field": "extra.namespaces",
            "kind": "array",
            "count": 0,
        }
        observed_container_path = write_result(
            root,
            "observed-container.json",
            observed_container,
        )
        assert (
            row_for(
                observed_container_path,
                "observed_field:namespaces",
                expect="",
            )["assertion"]
            == "pass"
        )
        assert (
            row_for(
                observed_container_path,
                "observed_field:documents",
                expect="",
            )["assertion"]
            == "fail"
        )

    print("MANUAL_CASE_ASSERTIONS_TESTS ok")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
