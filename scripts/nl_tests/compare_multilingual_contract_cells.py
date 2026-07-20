#!/usr/bin/env python3
"""Compare multilingual live NL results for the same contract cell.

The generator emits rows with `base_case_id` and `nl_variant` when
`--multilingual-variants` is used. This checker pairs those rows with
client-like live run logs and verifies that the structural contract behavior is
stable across languages. It deliberately does not compare final prose.
"""

from __future__ import annotations

import argparse
import json
from collections import defaultdict
from pathlib import Path
from types import SimpleNamespace
from typing import Any

from evaluate_client_like_run import observe_file, sort_key


DEFAULT_VARIANTS = ("zh_cn", "en_us", "ja_jp", "ko_kr", "fr_fr", "mixed")


def load_case_rows(path: Path) -> dict[int, dict[str, Any]]:
    rows: dict[int, dict[str, Any]] = {}
    for idx, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        line = line.strip()
        if not line:
            continue
        row = json.loads(line)
        if not isinstance(row, dict):
            raise ValueError(f"{path}:{idx}: expected JSON object")
        rows[idx] = row
    return rows


def normalized_list(values: list[str]) -> tuple[str, ...]:
    return tuple(sorted(str(value) for value in values if str(value).strip()))


def signature(obs: Any) -> dict[str, Any]:
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
        "executed": normalized_list(obs.executed),
        "requested_action_refs": normalized_list(obs.requested_action_refs),
        "plan_action_refs": normalized_list(obs.plan_action_refs),
        "stop_signal": obs.stop_signal,
        "stop_failure_attribution": obs.stop_failure_attribution,
    }


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


def strict_shape_wrapping_failure(obs: Any) -> str:
    if obs.finalizer_allows_model_language is not False:
        return ""
    contract_shape = str(obs.contract_final_answer_shape or "").strip().lower()
    finalizer_class = str(obs.finalizer_final_answer_shape_class or "").strip().lower()
    final_shape = str(obs.final_shape or "").strip().lower()
    required = {str(value).strip().lower() for value in obs.required_evidence}

    if contract_shape == "scalar" and "count" in required and final_shape != "integer":
        return "strict count scalar must stay a bare integer"
    if finalizer_class == "single_path" and final_shape not in {"path", "file_token"}:
        return "strict single_path must stay a path or delivery token"
    if finalizer_class == "strict_list" and final_shape != "list":
        return "strict list must stay line-list shaped"
    return ""


def compare_group(
    group_id: str, items: list[tuple[dict[str, Any], Any]]
) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    failures: list[dict[str, Any]] = []
    inconclusive: list[dict[str, Any]] = []
    variants = [str(row.get("nl_variant") or "") for row, _ in items]
    missing_variants = sorted(set(DEFAULT_VARIANTS) - set(variants))
    if missing_variants:
        failures.append(
            {
                "base_case_id": group_id,
                "failure": "missing_variants",
                "missing": missing_variants,
                "observed": sorted(variants),
            }
        )

    inconclusive_items = [
        {
            "variant": row.get("nl_variant"),
            "file": obs.file,
            "status": obs.status,
            "error_kinds": obs.error_kinds,
            "failure_attributions": obs.failure_attributions,
            "stop_failure_attribution": obs.stop_failure_attribution,
        }
        for row, obs in items
        if provider_inconclusive(obs)
    ]
    if inconclusive_items:
        inconclusive.append(
            {
                "base_case_id": group_id,
                "reason": "provider_inconclusive",
                "items": inconclusive_items,
            }
        )
        return failures, inconclusive

    for row, obs in items:
        wrapping_failure = strict_shape_wrapping_failure(obs)
        if wrapping_failure:
            failures.append(
                {
                    "base_case_id": group_id,
                    "variant": row.get("nl_variant"),
                    "failure": "strict_shape_wrapped",
                    "reason": wrapping_failure,
                    "contract_final_answer_shape": obs.contract_final_answer_shape,
                    "finalizer_final_answer_shape_class": obs.finalizer_final_answer_shape_class,
                    "final_shape": obs.final_shape,
                    "file": obs.file,
                }
            )

    baseline_row, baseline_obs = items[0]
    baseline_signature = signature(baseline_obs)
    for row, obs in items[1:]:
        observed = signature(obs)
        drift = {
            key: {"baseline": baseline_signature[key], "observed": value}
            for key, value in observed.items()
            if value != baseline_signature[key]
        }
        if drift:
            failures.append(
                {
                    "base_case_id": group_id,
                    "baseline_variant": baseline_row.get("nl_variant"),
                    "variant": row.get("nl_variant"),
                    "failure": "structural_drift",
                    "drift": drift,
                    "baseline_file": baseline_obs.file,
                    "file": obs.file,
                }
            )
    return failures, inconclusive


def fake_obs(**overrides: Any) -> Any:
    base = {
        "status": "succeeded",
        "contract_match": "generic_exact_count",
        "contract_semantic_kind": "none",
        "contract_final_answer_shape": "scalar",
        "required_evidence": ["count"],
        "missing_evidence": [],
        "finalizer_final_answer_shape": "scalar",
        "finalizer_final_answer_shape_class": "scalar_value",
        "finalizer_coarse_response_shape": "scalar",
        "finalizer_allows_model_language": False,
        "executed": ["fs_basic.count_entries"],
        "requested_action_refs": ["fs_basic.count_entries"],
        "plan_action_refs": ["fs_basic.count_entries"],
        "stop_signal": "",
        "stop_failure_attribution": "",
        "error_kinds": [],
        "failure_attributions": [],
        "verifier_issue_attributions": [],
        "final_shape": "integer",
        "file": "turn_1_case_1.json",
    }
    base.update(overrides)
    return SimpleNamespace(**base)


def run_self_test() -> int:
    rows = [{"base_case_id": "case-1", "nl_variant": variant} for variant in DEFAULT_VARIANTS]
    ok_items = [(row, fake_obs(file=f"turn_1_case_{idx}.json")) for idx, row in enumerate(rows, 1)]
    failures, inconclusive = compare_group("case-1", ok_items)
    if failures or inconclusive:
        raise AssertionError(f"identical multilingual fixture failed: {failures=} {inconclusive=}")

    drift_items = list(ok_items)
    drift_items[2] = (
        rows[2],
        fake_obs(plan_action_refs=["archive_basic.list"], file="turn_1_case_3.json"),
    )
    failures, inconclusive = compare_group("case-1", drift_items)
    if not failures or failures[0].get("failure") != "structural_drift":
        raise AssertionError(f"drift fixture did not fail as expected: {failures=}")
    if inconclusive:
        raise AssertionError(f"drift fixture should not be inconclusive: {inconclusive=}")

    wrapped_items = list(ok_items)
    wrapped_items[3] = (
        rows[3],
        fake_obs(final_shape="non_empty", file="turn_1_case_4.json"),
    )
    failures, _ = compare_group("case-1", wrapped_items)
    if not any(failure.get("failure") == "strict_shape_wrapped" for failure in failures):
        raise AssertionError(f"strict wrapped fixture did not fail: {failures=}")

    unavailable_items = list(ok_items)
    unavailable_items[4] = (
        rows[4],
        fake_obs(
            status="failed",
            error_kinds=["provider_unavailable"],
            failure_attributions=["provider_error"],
            file="turn_1_case_5.json",
        ),
    )
    failures, inconclusive = compare_group("case-1", unavailable_items)
    if failures or not inconclusive:
        raise AssertionError(
            f"provider unavailable fixture should be inconclusive: {failures=} {inconclusive=}"
        )
    print("MULTILINGUAL_CONTRACT_CELL_COMPARE_SELF_TEST_OK")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("run_dir", type=Path, nargs="?", help="client-like continuous run log directory")
    parser.add_argument(
        "--case-jsonl",
        type=Path,
        help="JSONL rows used for the live run, from generate_contract_matrix_cases.py --nl --multilingual-variants",
    )
    parser.add_argument("--self-test", action="store_true", help="run built-in comparator smoke tests")
    args = parser.parse_args()

    if args.self_test:
        return run_self_test()

    if args.run_dir is None or args.case_jsonl is None:
        parser.error("run_dir and --case-jsonl are required unless --self-test is used")

    if not args.run_dir.is_dir():
        raise SystemExit(f"Not a directory: {args.run_dir}")
    cases = load_case_rows(args.case_jsonl)
    files = sorted(args.run_dir.glob("turn_*_case_*.json"), key=sort_key)
    if not files:
        raise SystemExit(f"No turn_*_case_*.json files found under {args.run_dir}")

    grouped: dict[str, list[tuple[dict[str, Any], Any]]] = defaultdict(list)
    for path in files:
        obs = observe_file(path)
        row = cases.get(obs.case)
        if row is None:
            raise SystemExit(f"No case metadata for case {obs.case} ({path.name})")
        group_id = str(row.get("base_case_id") or row.get("case_id") or obs.case)
        grouped[group_id].append((row, obs))

    failures: list[dict[str, Any]] = []
    inconclusive: list[dict[str, Any]] = []
    for group_id, items in sorted(grouped.items()):
        items.sort(key=lambda item: DEFAULT_VARIANTS.index(str(item[0].get("nl_variant") or "")))
        group_failures, group_inconclusive = compare_group(group_id, items)
        failures.extend(group_failures)
        inconclusive.extend(group_inconclusive)

    if failures:
        for failure in failures:
            print(json.dumps(failure, ensure_ascii=False, sort_keys=True))
        print(
            f"MULTILINGUAL_CONTRACT_CELL_COMPARE_FAIL groups={len(grouped)} failures={len(failures)} inconclusive={len(inconclusive)}"
        )
        return 1
    print(
        f"MULTILINGUAL_CONTRACT_CELL_COMPARE_OK groups={len(grouped)} observations={len(files)} inconclusive={len(inconclusive)}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
