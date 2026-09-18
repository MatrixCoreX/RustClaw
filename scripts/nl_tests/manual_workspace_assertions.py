"""Strict, ordered workspace-file lifecycle evidence for NL acceptance only."""
import json
import re
from pathlib import PurePosixPath


def digest(value):
    if not isinstance(value, str):
        return None
    value = value.removeprefix("sha256:")
    return value if re.fullmatch(r"[0-9a-f]{64}", value) else None


def workspace_file_cycle_assertion(spec_text, result, successful_steps):
    detail = {"kind": "workspace_file_cycle", "schema_version": 1, "ok": False,
              "evidence_steps": [], "reason": "invalid_spec"}
    try:
        spec = json.loads(spec_text)
    except (TypeError, ValueError):
        return detail
    if not isinstance(spec, dict):
        return detail
    path = spec.get("path")
    if (spec.get("schema_version") != 1 or not isinstance(path, str) or not path
            or PurePosixPath(path).is_absolute() or ".." in PurePosixPath(path).parts
            or digest(spec.get("sha256")) is None
            or any(type(spec.get(key)) is not int or spec[key] < 0
                   for key in ("size_bytes", "line_count"))):
        return detail
    detail["expected"] = spec
    expected = (digest(spec["sha256"]), spec["size_bytes"])
    journal = result.get("task_journal") or {}
    trace = journal.get("trace") or {}
    caps = trace.get("capability_results") or []
    steps = {s["step_id"]: (i, s) for i, s in enumerate(successful_steps) if s.get("step_id")}
    current = None
    created = verified = removed = False
    previous_index = -1
    bound_read_paths = {path}

    def fail(reason):
        detail["reason"] = reason
        return detail

    def snapshot_state(snapshot):
        if not isinstance(snapshot, dict) or snapshot.get("path") != path:
            raise ValueError("snapshot_path_mismatch")
        if snapshot.get("kind") == "missing":
            if snapshot.get("sha256") is not None or snapshot.get("size_bytes") is not None:
                raise ValueError("invalid_missing_snapshot")
            return None
        value = snapshot.get("size_bytes")
        sha = digest(snapshot.get("sha256"))
        if snapshot.get("kind") != "file" or sha is None or type(value) is not int or value < 0:
            raise ValueError("invalid_file_snapshot")
        return sha, value

    # Only runtime-bound capability outputs count; never inspect assistant prose
    # or assume that a successful early write is still the final file state.
    for cap in caps:
        if not isinstance(cap, dict):
            return fail("invalid_capability_record")
        capability = cap.get("capability")
        output = (cap.get("data") or {}).get("output")
        mutates = cap.get("effect") == "mutate"
        reads = capability == "filesystem.read_text_range"
        if not mutates and not reads:
            continue
        sid = (cap.get("provenance") or {}).get("step_id")
        bound = steps.get(sid)
        if (cap.get("status") != "ok" or cap.get("truncated") is True
                or (cap.get("provenance") or {}).get("source") != "runtime_step"
                or bound is None or bound[0] <= previous_index
                or capability not in (bound[1].get("resolved_capability"), bound[1].get("requested_capability"))):
            return fail("unverified_or_unordered_execution")
        previous_index = bound[0]
        if not isinstance(output, dict):
            return fail("structured_output_missing")
        if mutates:
            if (output.get("state") == "no_op" and output.get("status") == "ok"
                    and output.get("changed_files") == [] and output.get("before")
                    and output.get("before") == output.get("after")):
                continue
            if (output.get("state") != "applied" or output.get("status") != "ok"
                    or output.get("target_path") != path or output.get("changed_files") != [path]
                    or not isinstance(output.get("before"), list) or len(output["before"]) != 1
                    or not isinstance(output.get("after"), list) or len(output["after"]) != 1):
                return fail("mutation_scope_or_state_invalid")
            try:
                before = snapshot_state(output["before"][0])
                after = snapshot_state(output["after"][0])
            except ValueError as error:
                return fail(str(error))
            if before != current:
                return fail("mutation_chain_mismatch")
            if after is None:
                if not created or not verified or before != expected:
                    return fail("remove_without_verified_content")
                removed = True
            else:
                created = True
                removed = False
                for ref in cap.get("evidence") or []:
                    locator = ref.get("locator")
                    if (ref.get("source") == capability and ref.get("id") == sid
                            and (ref.get("metadata") or {}).get("step_id") == sid
                            and isinstance(locator, str) and PurePosixPath(locator).is_absolute()):
                        bound_read_paths.add(locator)
            current = after
            verified = False
            detail["evidence_steps"].append({"step_id": sid, "capability": capability,
                                              "before": before, "after": after})
        elif output.get("path") in bound_read_paths:
            actual = (digest(output.get("sha256")), output.get("size_bytes"))
            if actual != current:
                return fail("readback_snapshot_mismatch")
            safety = output.get("line_safety") or {}
            start_line = 0 if spec["line_count"] == 0 else 1
            verified = (actual == expected and output.get("total_lines") == spec["line_count"]
                        and output.get("returned_line_count") == spec["line_count"]
                        and output.get("start_line") == start_line and output.get("end_line") == spec["line_count"]
                        and output.get("truncated") is False
                        and not safety.get("excerpt_truncated") and not safety.get("truncated_lines"))
            detail["evidence_steps"].append({"step_id": sid, "capability": capability,
                                              "snapshot": actual, "verified": verified})
    detail["ok"] = created and removed and current is None
    detail["reason"] = "verified_readback_then_removed" if detail["ok"] else "lifecycle_incomplete"
    return detail
