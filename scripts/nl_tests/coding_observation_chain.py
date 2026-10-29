"""Validate explicit same-task observation records before selecting logical cases."""
import json
from pathlib import Path

from manual_case_lifecycle import poll_status
from manual_case_observation import check_reply, object_value, read_source, source_binding


def canonical(value):
    return json.dumps(value, sort_keys=True, ensure_ascii=False, allow_nan=False)


def contract(row):
    return {key: row.get(key) for key in
            ("source_line", "case_name", "task_id", "tags", "prompt", "mode", "expect_substr")}


def timestamp(row, field):
    value = row.get(field)
    if type(value) is not int or value <= 0:
        raise ValueError("observation_time_missing_or_invalid")
    return value


def final_path(run, name):
    matches = list(run.glob(f"case_*_{name}/final.json"))
    if len(matches) != 1 or matches[0].is_symlink():
        raise ValueError("observation_final_missing_or_ambiguous")
    return matches[0]


def apply_observations(state, rows, observation_runs):
    selected = list(rows)
    proofs = []
    used_runs, used_cases = set(), set()
    for path in observation_runs:
        run = Path(path).resolve()
        if Path(path).is_symlink() or run.is_relative_to((state / "results").resolve()) or run in used_runs:
            raise ValueError("observation_requires_distinct_supplemental_run")
        used_runs.add(run)
        summary = run / "summary.jsonl"
        if summary.is_symlink() or not summary.is_file():
            raise ValueError("observation_summary_missing")
        supplemental = [object_value(json.loads(line)) for line in summary.read_text().splitlines()]
        if len(supplemental) != 1:
            raise ValueError("observation_requires_one_result")
        observed = supplemental[0]
        if observed.get("status") != "succeeded" or observed.get("assertion") != "pass":
            raise ValueError("observation_not_accepted")
        candidates = [index for index, row in enumerate(rows)
                      if row.get("task_id") == observed.get("task_id")
                      and row.get("case_name") == observed.get("case_name")]
        if len(candidates) != 1 or candidates[0] in used_cases:
            raise ValueError("observation_original_identity_ambiguous")
        index = candidates[0]
        used_cases.add(index)
        original = rows[index]
        if canonical(contract(original)) != canonical(contract(observed)):
            raise ValueError("observation_case_contract_changed")
        final = final_path(run, observed["case_name"])
        proof = object_value(read_source(final.parent / "observation.json")[0])
        source = proof.get("source_case")
        if not isinstance(source, str):
            raise ValueError("observation_source_missing")
        source = Path(source).resolve()
        if source.parent != original["run_dir"].resolve():
            raise ValueError("observation_source_outside_original_run")
        expected = source_binding(source, original["source_line"], original["case_name"],
                                  original["tags"], original["prompt"], original["expect_substr"] or "")
        if canonical(proof) != canonical(expected):
            raise ValueError("observation_source_digest_or_identity_changed")
        source_data = object_value(read_source(source / "final.json")[0].get("data"))
        if (original.get("status") not in {"provider_unavailable", "timeout"}
                or source_data.get("status") not in {"queued", "running"}
                or (original["status"] == "provider_unavailable" and poll_status(source_data) != "provider_wait")):
            raise ValueError("original_case_not_resumable")
        check_reply(original["task_id"], final)
        if object_value(read_source(final)[0].get("data")).get("status") != "succeeded":
            raise ValueError("observation_terminal_status_mismatch")
        start, end = timestamp(observed, "started_at"), timestamp(observed, "ended_at")
        if (start < timestamp(original, "ended_at") or end < start
                or (index + 1 < len(rows) and end > timestamp(rows[index + 1], "started_at"))):
            raise ValueError("observation_order_invalid")
        selected[index] = {**observed, "run_dir": run}
        proofs.append({"case_name": observed["case_name"], "task_id": observed["task_id"],
                       "original_status": original["status"], "source_run": str(original["run_dir"]),
                       "observation_run": str(run), "source_sha256": expected["source_sha256"]})
    return selected, proofs
