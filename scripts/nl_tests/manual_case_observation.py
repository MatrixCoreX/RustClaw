"""Bind read-only acceptance polling to an immutable prior case identity."""
import argparse
import hashlib
import json
from pathlib import Path
import sys
import uuid


def object_value(value):
    if not isinstance(value, dict):
        raise ValueError("object_required")
    return value


def canonical_task_id(value):
    if not isinstance(value, str) or str(uuid.UUID(value)) != value:
        raise ValueError("canonical_task_id_required")
    return value


def read_source(path):
    if path.is_symlink() or not path.is_file():
        raise ValueError("regular_source_required")
    raw = path.read_bytes()
    return json.loads(raw), hashlib.sha256(raw).hexdigest()


def source_binding(case_dir, source_line, name, tags, prompt, expectation):
    if case_dir.is_symlink() or not case_dir.is_dir():
        raise ValueError("case_directory_required")
    case_dir = case_dir.resolve()
    source_run = case_dir.parent
    submit, submit_hash = read_source(case_dir / "submit.json")
    task_id = canonical_task_id(object_value(object_value(submit).get("data")).get("task_id"))
    final, final_hash = read_source(case_dir / "final.json")
    if object_value(object_value(final).get("data")).get("task_id") != task_id:
        raise ValueError("final_identity_mismatch")
    summary = source_run / "summary.jsonl"
    if summary.is_symlink() or not summary.is_file():
        raise ValueError("regular_summary_required")
    summary_hash = hashlib.sha256()
    rows = []
    with summary.open("rb") as stream:
        for line in stream:
            summary_hash.update(line)
            row = object_value(json.loads(line))
            if row.get("task_id") == task_id:
                rows.append(row)
    if len(rows) != 1:
        raise ValueError("unique_original_row_required")
    row = rows[0]
    recorded_expectation = row.get("expect_substr")
    if recorded_expectation is None:
        recorded_expectation = ""
    if (type(row.get("source_line")) is not int or row["source_line"] != source_line
            or row.get("case_name") != name or row.get("tags") != tags
            or row.get("prompt") != prompt or row.get("mode") != "ask"
            or recorded_expectation != expectation):
        raise ValueError("case_contract_mismatch")
    return {"schema_version": 1, "mode": "existing_task_readonly", "task_id": task_id,
            "source_case": str(case_dir), "source_run": str(source_run),
            "source_sha256": {"submit.json": submit_hash, "final.json": final_hash,
                              "summary.jsonl": summary_hash.hexdigest()}}


def bind_case(case_dir, output, source_line, name, tags, prompt, expectation):
    evidence = source_binding(case_dir, source_line, name, tags, prompt, expectation)
    if output.resolve().is_relative_to(Path(evidence["source_run"])):
        raise ValueError("source_run_is_readonly")
    with output.open("x", encoding="utf-8") as stream:
        json.dump(evidence, stream, ensure_ascii=False, sort_keys=True)
        stream.write("\n")
    return evidence["task_id"]


def check_reply(task_id, path):
    task_id = canonical_task_id(task_id)
    reply = object_value(json.loads(path.read_text()))
    if reply.get("ok") is not True or object_value(reply.get("data")).get("task_id") != task_id:
        raise ValueError("reply_identity_mismatch")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    bind = commands.add_parser("bind")
    bind.add_argument("case_dir", type=Path)
    bind.add_argument("output", type=Path)
    bind.add_argument("source_line", type=int)
    for name in ("name", "tags", "prompt", "expectation"):
        bind.add_argument(name)
    check = commands.add_parser("check-reply")
    check.add_argument("task_id")
    check.add_argument("path", type=Path)
    args = parser.parse_args()
    try:
        if args.command == "bind":
            print(bind_case(args.case_dir, args.output, args.source_line, args.name,
                            args.tags, args.prompt, args.expectation))
        else:
            check_reply(args.task_id, args.path)
    except (OSError, ValueError, TypeError) as error:
        code = ("observation_source_binding_invalid" if args.command == "bind"
                else "observation_reply_identity_invalid")
        print(f"{code}:{type(error).__name__}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
