#!/usr/bin/env python3
"""Compare two provider run directories by contract-level behavior.

This is an offline A/B checker for client-like NL run logs. It compares
structured trace fields, not final prose, so MiniMax/OpenAI/etc. can differ in
wording while still being held to the same task contract.
"""

from __future__ import annotations

import argparse
import json
import shutil
import tempfile
from pathlib import Path
from typing import Any

from evaluate_client_like_run import observe_file, sort_key


SCHEMA_RECOVERY_TOKENS = (
    "schema_recovery",
    "schema_recovered",
    "raw_parse_ok",
    "recovered_tool_call_contract",
)


def normalized_list(values: list[str]) -> tuple[str, ...]:
    return tuple(sorted(str(value) for value in values if str(value).strip()))


def load_observations(run_dir: Path) -> dict[int, Any]:
    if not run_dir.is_dir():
        raise SystemExit(f"Not a directory: {run_dir}")
    files = sorted(run_dir.glob("turn_*_case_*.json"), key=sort_key)
    if not files:
        raise SystemExit(f"No turn_*_case_*.json files found under {run_dir}")
    observations: dict[int, Any] = {}
    for path in files:
        obs = observe_file(path)
        if obs.case in observations:
            raise SystemExit(f"Duplicate case {obs.case} under {run_dir}")
        observations[obs.case] = obs
    return observations


def provider_inconclusive(obs: Any) -> bool:
    values = {
        obs.status,
        obs.stop_signal,
        obs.stop_failure_attribution,
        *obs.error_kinds,
        *obs.failure_attributions,
        *obs.verifier_issue_attributions,
    }
    lowered = {str(value).strip().lower() for value in values if str(value).strip()}
    if "not_run_after_provider_unavailable" in lowered:
        return True
    return bool(
        {"provider_error", "provider_unavailable", "llm_provider_unavailable"} & lowered
    )


def structural_signature(obs: Any) -> dict[str, Any]:
    return {
        "status": obs.status,
        "contract_match": obs.contract_match,
        "contract_semantic_kind": obs.contract_semantic_kind,
        "contract_final_answer_shape": obs.contract_final_answer_shape,
        "required_evidence": normalized_list(obs.required_evidence),
        "missing_evidence": normalized_list(obs.missing_evidence),
        "finalizer_final_answer_shape": obs.finalizer_final_answer_shape,
        "finalizer_final_answer_shape_class": obs.finalizer_final_answer_shape_class,
        "finalizer_coarse_response_shape": obs.finalizer_coarse_response_shape,
        "finalizer_allows_model_language": obs.finalizer_allows_model_language,
        "plan_action_refs": normalized_list(obs.plan_action_refs),
        "requested_action_refs": normalized_list(obs.requested_action_refs),
        "executed": normalized_list(obs.executed),
        "contract_policy_decisions": normalized_list(obs.contract_policy_decisions),
        "error_kinds": normalized_list(obs.error_kinds),
        "failure_attributions": normalized_list(obs.failure_attributions),
        "verifier_issue_attributions": normalized_list(obs.verifier_issue_attributions),
        "stop_signal": obs.stop_signal,
        "stop_failure_attribution": obs.stop_failure_attribution,
    }


def iter_json_values(value: Any) -> Any:
    yield value
    if isinstance(value, dict):
        for key, child in value.items():
            yield key
            yield from iter_json_values(child)
    elif isinstance(value, list):
        for child in value:
            yield from iter_json_values(child)


def schema_recovery_signal_count(run_dir: Path) -> int:
    count = 0
    for path in sorted(run_dir.glob("turn_*_case_*.json"), key=sort_key):
        if schema_recovery_signals_for_file(path):
            count += 1
    return count


def schema_recovery_signals_for_file(path: Path) -> list[str]:
    if not path.is_file():
        return []
    obj = json.loads(path.read_text(encoding="utf-8"))
    text = "\n".join(str(value) for value in iter_json_values(obj))
    return [token for token in SCHEMA_RECOVERY_TOKENS if token in text]


def compare_observations(
    left: dict[int, Any],
    right: dict[int, Any],
    left_label: str,
    right_label: str,
) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    failures: list[dict[str, Any]] = []
    inconclusive: list[dict[str, Any]] = []
    all_cases = sorted(set(left) | set(right))
    for case_id in all_cases:
        left_obs = left.get(case_id)
        right_obs = right.get(case_id)
        if left_obs is None or right_obs is None:
            failures.append(
                {
                    "case": case_id,
                    "failure": "missing_case",
                    left_label: bool(left_obs),
                    right_label: bool(right_obs),
                }
            )
            continue
        if provider_inconclusive(left_obs) or provider_inconclusive(right_obs):
            inconclusive.append(
                {
                    "case": case_id,
                    left_label: {
                        "file": left_obs.file,
                        "status": left_obs.status,
                        "error_kinds": left_obs.error_kinds,
                        "failure_attributions": left_obs.failure_attributions,
                        "stop_failure_attribution": left_obs.stop_failure_attribution,
                    },
                    right_label: {
                        "file": right_obs.file,
                        "status": right_obs.status,
                        "error_kinds": right_obs.error_kinds,
                        "failure_attributions": right_obs.failure_attributions,
                        "stop_failure_attribution": right_obs.stop_failure_attribution,
                    },
                }
            )
            continue
        left_sig = structural_signature(left_obs)
        right_sig = structural_signature(right_obs)
        drift = {
            key: {left_label: left_sig[key], right_label: value}
            for key, value in right_sig.items()
            if value != left_sig[key]
        }
        if drift:
            failures.append(
                {
                    "case": case_id,
                    "failure": "contract_provider_drift",
                    "drift": drift,
                    left_label: {"file": left_obs.file, "prompt": left_obs.prompt},
                    right_label: {"file": right_obs.file, "prompt": right_obs.prompt},
                }
            )
    return failures, inconclusive


def min_repro_side(obs: Any | None, run_dir: Path) -> dict[str, Any] | None:
    if obs is None:
        return None
    return {
        "file": obs.file,
        "request": obs.prompt,
        "status": obs.status,
        "provider_raw_parse_status": {
            "schema_recovery_signals": schema_recovery_signals_for_file(run_dir / obs.file),
        },
        "contract": {
            "match": obs.contract_match,
            "semantic_kind": obs.contract_semantic_kind,
            "final_answer_shape": obs.contract_final_answer_shape,
            "required_evidence": obs.required_evidence,
            "missing_evidence": obs.missing_evidence,
        },
        "actions": {
            "planned": obs.plan_action_refs,
            "requested": obs.requested_action_refs,
            "executed": obs.executed,
        },
        "policy_decisions": obs.contract_policy_decisions,
        "evidence": {
            "observed_fields": obs.observed_evidence_fields,
            "observed_canonical": obs.observed_evidence_canonical,
            "missing": obs.missing_evidence,
        },
        "finalizer": {
            "final_answer_shape": obs.finalizer_final_answer_shape,
            "final_answer_shape_class": obs.finalizer_final_answer_shape_class,
            "allows_model_language": obs.finalizer_allows_model_language,
        },
        "failure": {
            "error_kinds": obs.error_kinds,
            "failure_attributions": obs.failure_attributions,
            "verifier_issue_attributions": obs.verifier_issue_attributions,
            "stop_signal": obs.stop_signal,
            "stop_failure_attribution": obs.stop_failure_attribution,
        },
        "final_answer_preview": str(obs.final_text or "")[:240],
    }


def write_min_repro(
    path: Path,
    failures: list[dict[str, Any]],
    left: dict[int, Any],
    right: dict[int, Any],
    *,
    left_dir: Path,
    right_dir: Path,
    left_label: str,
    right_label: str,
) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", encoding="utf-8") as handle:
        for failure in failures:
            case_id = int(failure["case"])
            row = {
                "case": case_id,
                "failure": failure.get("failure"),
                "drift": failure.get("drift", {}),
                left_label: min_repro_side(left.get(case_id), left_dir),
                right_label: min_repro_side(right.get(case_id), right_dir),
            }
            handle.write(json.dumps(row, ensure_ascii=False, sort_keys=True) + "\n")


def write_minimal_run_case(
    run_dir: Path,
    case_id: int,
    *,
    contract_match: str = "scalar_count",
    final_shape: str = "scalar",
    executed_skill: str = "fs_basic",
    status: str = "succeeded",
    failure_attribution: str = "",
) -> None:
    result_status = status
    step: dict[str, Any] = {
        "executed_skill": executed_skill,
        "requested_action_ref": f"{executed_skill}.count_entries"
        if executed_skill == "fs_basic"
        else executed_skill,
        "status": "ok" if status == "succeeded" else "error",
    }
    if failure_attribution:
        step["failure_attribution"] = failure_attribution
        step["error_kind"] = "provider_unavailable"
    payload = {
        "ok": result_status == "succeeded",
        "data": {
            "status": result_status,
            "result_json": {
                "text": "3",
                "task_journal": {
                    "summary": {
                        "input_text": "count fixture",
                        "route_result": {
                            "legacy_first_layer_decision": "planner_execute",
                            "route_gate_kind": "execute",
                        },
                        "finalizer_summary": {
                            "final_answer_shape": final_shape,
                            "final_answer_shape_class": "scalar_value",
                            "coarse_response_shape": "scalar",
                            "allows_model_language": False,
                            "grounded_ok": True,
                        },
                    },
                    "trace": {
                        "contract_matrix": {
                            "contract_match": contract_match,
                            "semantic_kind": contract_match,
                            "final_answer_shape": final_shape,
                        },
                        "evidence_coverage": {
                            "required_evidence": ["count"],
                            "observed_fields": ["count"],
                            "observed_canonical": ["count"],
                            "missing_evidence": [],
                        },
                        "step_results": [step],
                    },
                },
            },
        },
    }
    run_dir.mkdir(parents=True, exist_ok=True)
    (run_dir / f"turn_1_case_{case_id}.json").write_text(
        json.dumps(payload, ensure_ascii=False, sort_keys=True),
        encoding="utf-8",
    )


def run_self_test() -> int:
    root = Path(tempfile.mkdtemp(prefix="rustclaw-provider-compare-"))
    try:
        left = root / "left"
        right = root / "right"
        write_minimal_run_case(left, 1)
        write_minimal_run_case(right, 1)
        failures, inconclusive = compare_observations(
            load_observations(left), load_observations(right), "left", "right"
        )
        if failures or inconclusive:
            raise AssertionError(f"identical fixture failed: {failures=} {inconclusive=}")

        drift = root / "drift"
        write_minimal_run_case(drift, 1, final_shape="name_list")
        failures, inconclusive = compare_observations(
            load_observations(left), load_observations(drift), "left", "drift"
        )
        if not failures or failures[0].get("failure") != "contract_provider_drift":
            raise AssertionError(f"drift fixture did not fail as expected: {failures=}")
        if inconclusive:
            raise AssertionError(f"drift fixture should not be inconclusive: {inconclusive=}")
        min_repro = root / "min-repro.jsonl"
        write_min_repro(
            min_repro,
            failures,
            load_observations(left),
            load_observations(drift),
            left_dir=left,
            right_dir=drift,
            left_label="left",
            right_label="drift",
        )
        rows = [
            json.loads(line)
            for line in min_repro.read_text(encoding="utf-8").splitlines()
            if line.strip()
        ]
        if not rows or rows[0]["left"]["contract"]["match"] != "scalar_count":
            raise AssertionError(f"min repro export missing contract context: {rows=}")

        unavailable = root / "unavailable"
        write_minimal_run_case(
            unavailable,
            1,
            status="failed",
            failure_attribution="provider_error",
        )
        failures, inconclusive = compare_observations(
            load_observations(left), load_observations(unavailable), "left", "unavailable"
        )
        if failures or not inconclusive:
            raise AssertionError(
                f"provider unavailable fixture should be inconclusive: {failures=} {inconclusive=}"
            )
    finally:
        shutil.rmtree(root, ignore_errors=True)
    print("CONTRACT_PROVIDER_COMPARE_SELF_TEST_OK")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--left", type=Path, help="left provider run directory")
    parser.add_argument("--right", type=Path, help="right provider run directory")
    parser.add_argument("--left-label", default="left")
    parser.add_argument("--right-label", default="right")
    parser.add_argument("--min-repro-out", type=Path, help="write failing case min repro JSONL")
    parser.add_argument("--self-test", action="store_true", help="run built-in comparator smoke tests")
    args = parser.parse_args()

    if args.self_test:
        return run_self_test()

    if args.left is None or args.right is None:
        parser.error("--left and --right are required unless --self-test is used")

    left = load_observations(args.left)
    right = load_observations(args.right)
    failures, inconclusive = compare_observations(
        left, right, args.left_label, args.right_label
    )
    schema_left = schema_recovery_signal_count(args.left)
    schema_right = schema_recovery_signal_count(args.right)

    if failures:
        if args.min_repro_out is not None:
            write_min_repro(
                args.min_repro_out,
                failures,
                left,
                right,
                left_dir=args.left,
                right_dir=args.right,
                left_label=args.left_label,
                right_label=args.right_label,
            )
        for failure in failures:
            print(json.dumps(failure, ensure_ascii=False, sort_keys=True))
        print(
            "CONTRACT_PROVIDER_COMPARE_FAIL "
            f"compared={len(set(left) & set(right))} "
            f"failed={len(failures)} "
            f"inconclusive={len(inconclusive)} "
            f"schema_recovery_{args.left_label}={schema_left} "
            f"schema_recovery_{args.right_label}={schema_right}"
            + (f" min_repro={args.min_repro_out}" if args.min_repro_out else "")
        )
        return 1

    print(
        "CONTRACT_PROVIDER_COMPARE_OK "
        f"compared={len(set(left) & set(right))} "
        f"inconclusive={len(inconclusive)} "
        f"schema_recovery_{args.left_label}={schema_left} "
        f"schema_recovery_{args.right_label}={schema_right}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
