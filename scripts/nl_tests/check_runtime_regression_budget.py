#!/usr/bin/env python3
"""Evaluate rollout metrics against a versioned runtime regression budget."""

from __future__ import annotations

import argparse
import json
import math
from pathlib import Path
import tomllib
from typing import Any


ROOT = Path(__file__).resolve().parents[2]
DEFAULT_CONTRACT = ROOT / "scripts/inventories/runtime_regression_budgets.toml"

REQUIRED_FIELDS = {
    "min_pass_rate",
    "max_avg_prompt_tokens",
    "max_avg_uncached_input_tokens",
    "min_cache_read_ratio",
    "require_cache_metrics",
    "max_avg_llm_calls",
    "max_avg_tool_calls",
    "max_avg_wall_time_ms",
    "require_complete_wall_time",
    "max_prompt_truncations",
    "max_provider_final_errors",
}


def nested(payload: dict[str, Any], *path: str, default: Any = None) -> Any:
    value: Any = payload
    for key in path:
        if not isinstance(value, dict):
            return default
        value = value.get(key)
    return default if value is None else value


def number(value: Any) -> float:
    try:
        return float(value)
    except (TypeError, ValueError):
        return 0.0


def load_contract(path: Path, profile: str) -> dict[str, Any]:
    with path.open("rb") as handle:
        payload = tomllib.load(handle)
    if payload.get("schema_version") != 1:
        raise ValueError("runtime_budget.schema_version_must_be_1")
    profiles = payload.get("profiles")
    if not isinstance(profiles, dict) or not isinstance(profiles.get(profile), dict):
        raise ValueError(f"runtime_budget.profile_missing profile={profile}")
    selected = profiles[profile]
    missing = sorted(REQUIRED_FIELDS - set(selected))
    if missing:
        raise ValueError(f"runtime_budget.fields_missing fields={','.join(missing)}")
    if "min_wall_time_samples" in selected and (type(selected["min_wall_time_samples"]) is not int or selected["min_wall_time_samples"] < 1):
        raise ValueError("runtime_budget.min_wall_time_samples_invalid")
    if "max_p95_wall_time_ms" in selected and (type(selected["max_p95_wall_time_ms"]) not in (int,float) or not math.isfinite(selected["max_p95_wall_time_ms"]) or selected["max_p95_wall_time_ms"] < 0):
        raise ValueError("runtime_budget.max_p95_wall_time_ms_invalid")
    return selected


def threshold(
    checks: dict[str, dict[str, Any]],
    name: str,
    observed: Any,
    limit: Any,
    passed: bool,
) -> None:
    checks[name] = {
        "observed": observed,
        "limit": limit,
        "passed": passed,
    }


def evaluate(metrics: dict[str, Any], budget: dict[str, Any]) -> dict[str, Any]:
    checks: dict[str, dict[str, Any]] = {}

    pass_rate = number(metrics.get("pass_rate"))
    threshold(
        checks,
        "min_pass_rate",
        pass_rate,
        budget["min_pass_rate"],
        pass_rate >= number(budget["min_pass_rate"]),
    )

    avg_prompt_tokens = number(nested(metrics, "llm", "avg_input_tokens_per_turn"))
    threshold(
        checks,
        "max_avg_prompt_tokens",
        avg_prompt_tokens,
        budget["max_avg_prompt_tokens"],
        avg_prompt_tokens <= number(budget["max_avg_prompt_tokens"]),
    )

    avg_uncached_tokens = number(
        nested(metrics, "llm", "avg_uncached_input_tokens_per_turn")
    )
    threshold(
        checks,
        "max_avg_uncached_input_tokens",
        avg_uncached_tokens,
        budget["max_avg_uncached_input_tokens"],
        avg_uncached_tokens <= number(budget["max_avg_uncached_input_tokens"]),
    )

    cache_status = str(nested(metrics, "llm", "cache_metric_status", default="not_reported"))
    require_cache = bool(budget["require_cache_metrics"])
    threshold(
        checks,
        "cache_metrics_available",
        cache_status,
        "recorded" if require_cache else "optional",
        not require_cache or cache_status == "recorded",
    )
    cache_read_ratio = number(nested(metrics, "llm", "cache_read_ratio"))
    threshold(
        checks,
        "min_cache_read_ratio",
        cache_read_ratio,
        budget["min_cache_read_ratio"],
        (cache_status != "recorded" and not require_cache)
        or cache_read_ratio >= number(budget["min_cache_read_ratio"]),
    )

    avg_llm_calls = number(nested(metrics, "llm", "avg_calls_per_turn"))
    threshold(
        checks,
        "max_avg_llm_calls",
        avg_llm_calls,
        budget["max_avg_llm_calls"],
        avg_llm_calls <= number(budget["max_avg_llm_calls"]),
    )
    avg_tool_calls = number(nested(metrics, "execution", "avg_tool_calls_per_turn"))
    threshold(
        checks,
        "max_avg_tool_calls",
        avg_tool_calls,
        budget["max_avg_tool_calls"],
        avg_tool_calls <= number(budget["max_avg_tool_calls"]),
    )

    wall_status = str(
        nested(metrics, "wall_time", "recording_status", default="not_recorded")
    )
    require_wall = bool(budget["require_complete_wall_time"])
    threshold(
        checks,
        "complete_wall_time",
        wall_status,
        "complete" if require_wall else "optional",
        not require_wall or wall_status == "complete",
    )
    avg_wall_time = number(nested(metrics, "wall_time", "avg_ms"))
    threshold(
        checks,
        "max_avg_wall_time_ms",
        avg_wall_time,
        budget["max_avg_wall_time_ms"],
        (wall_status == "not_recorded" and not require_wall)
        or avg_wall_time <= number(budget["max_avg_wall_time_ms"]),
    )

    prompt_truncations = number(nested(metrics, "llm", "prompt_truncation_count"))
    if "min_wall_time_samples" in budget:
        count = nested(metrics,"wall_time","recorded_turns")
        valid = type(count) is int and count >= 0
        threshold(checks,"min_wall_time_samples",count if valid else None,budget["min_wall_time_samples"],
                  valid and count >= budget["min_wall_time_samples"] and wall_status == "complete")
    if "max_p95_wall_time_ms" in budget:
        p95 = nested(metrics,"wall_time","p95_ms")
        method = nested(metrics,"wall_time","percentile_method")
        samples = nested(metrics,"wall_time","samples_ms")
        count = nested(metrics,"wall_time","recorded_turns")
        samples_valid = isinstance(samples,list) and bool(samples) and len(samples)==count and all(type(v) in (int,float) and math.isfinite(v) and v >= 0 for v in samples)
        expected = sorted(samples)[math.ceil(len(samples)*.95)-1] if samples_valid else None
        valid = type(p95) in (int,float) and math.isfinite(p95) and p95 >= 0
        threshold(checks,"max_p95_wall_time_ms",p95 if valid else None,budget["max_p95_wall_time_ms"],
                  valid and samples_valid and p95 == expected and method == "nearest_rank" and wall_status == "complete" and p95 <= budget["max_p95_wall_time_ms"])
    threshold(
        checks,
        "max_prompt_truncations",
        prompt_truncations,
        budget["max_prompt_truncations"],
        prompt_truncations <= number(budget["max_prompt_truncations"]),
    )
    provider_errors = number(nested(metrics, "llm", "provider_final_error_count"))
    threshold(
        checks,
        "max_provider_final_errors",
        provider_errors,
        budget["max_provider_final_errors"],
        provider_errors <= number(budget["max_provider_final_errors"]),
    )

    failures = [name for name, result in checks.items() if not result["passed"]]
    return {
        "schema_version": 1,
        "checks": checks,
        "failures": failures,
        "passed": not failures,
    }


def self_test() -> None:
    budget = load_contract(DEFAULT_CONTRACT, "continuous_coding")
    metrics = {
        "pass_rate": 1.0,
        "llm": {
            "avg_input_tokens_per_turn": 40000,
            "avg_uncached_input_tokens_per_turn": 30000,
            "cache_metric_status": "recorded",
            "cache_read_ratio": 0.25,
            "avg_calls_per_turn": 3,
            "prompt_truncation_count": 0,
            "provider_final_error_count": 0,
        },
        "execution": {"avg_tool_calls_per_turn": 8},
        "wall_time": {"recording_status": "complete", "avg_ms": 120000},
    }
    assert evaluate(metrics, budget)["passed"]
    regressed = json.loads(json.dumps(metrics))
    regressed["llm"]["avg_uncached_input_tokens_per_turn"] = 90000
    result = evaluate(regressed, budget)
    assert not result["passed"]
    assert "max_avg_uncached_input_tokens" in result["failures"]
    repeated = load_contract(DEFAULT_CONTRACT,"repeated_readonly")
    sample = json.loads(json.dumps(metrics))
    sample["wall_time"] = {"recording_status":"complete","recorded_turns":10,"samples_ms":[12000]*9+[18000],"percentile_method":"nearest_rank","avg_ms":12600,"p95_ms":18000}
    assert evaluate(sample,repeated)["passed"]
    sample["wall_time"]["recorded_turns"] = 1
    assert "min_wall_time_samples" in evaluate(sample,repeated)["failures"]
    sample["wall_time"]["recorded_turns"] = 10
    for invalid in (None,True,float('nan'),float('inf'),60001):
        sample["wall_time"]["p95_ms"] = invalid
        assert "max_p95_wall_time_ms" in evaluate(sample,repeated)["failures"]
    print("RUNTIME_REGRESSION_BUDGET_SELF_TEST ok")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("metrics", nargs="?", help="Rollout metrics JSON")
    parser.add_argument("--contract", default=str(DEFAULT_CONTRACT))
    parser.add_argument("--profile", default="focused")
    parser.add_argument("--output")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()

    if args.self_test:
        self_test()
        return 0
    if not args.metrics:
        parser.error("metrics path is required")
    metrics = json.loads(Path(args.metrics).read_text(encoding="utf-8"))
    budget = load_contract(Path(args.contract), args.profile)
    result = evaluate(metrics, budget)
    result["profile"] = args.profile
    result["contract"] = str(Path(args.contract))
    encoded = json.dumps(result, ensure_ascii=False, indent=2, sort_keys=True) + "\n"
    if args.output:
        Path(args.output).write_text(encoded, encoding="utf-8")
    print(
        "RUNTIME_REGRESSION_BUDGET_CHECK "
        f"profile={args.profile} passed={str(result['passed']).lower()} "
        f"failures={','.join(result['failures']) or 'none'}"
    )
    return 0 if result["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
