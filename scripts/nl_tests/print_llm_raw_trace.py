#!/usr/bin/env python3
"""Print raw LLM return fields for NL test cases.

The NL shell runners tail `logs/model_io.log` while a task is polling. This
helper keeps a byte-offset state file and prints numbered `LLM#N` records for
the selected task. Long fields are rendered as head/tail excerpts with the full
log path, so Codex chat shows the raw field names and key values without
dumping multi-megabyte prompts.
"""
from __future__ import annotations

import argparse
import json
import sys
import tempfile
from pathlib import Path
from typing import Any


DEFAULT_MAX_CHARS = 2400
LONG_FIELD_NAMES = {
    "prompt",
    "response",
    "clean_response",
    "raw_response",
    "request_payload",
    "response_text",
}


def read_state(path: Path | None) -> dict[str, Any]:
    if path is None or not path.exists():
        return {"offset": 0, "next_index": 1, "active_task_id": None}
    raw = path.read_text(encoding="utf-8").strip()
    if not raw:
        return {"offset": 0, "next_index": 1, "active_task_id": None}
    try:
        value = json.loads(raw)
    except json.JSONDecodeError:
        try:
            return {"offset": int(raw), "next_index": 1, "active_task_id": None}
        except ValueError:
            return {"offset": 0, "next_index": 1, "active_task_id": None}
    if isinstance(value, dict):
        return {
            "offset": max(0, int(value.get("offset") or 0)),
            "next_index": max(1, int(value.get("next_index") or 1)),
            "active_task_id": value.get("active_task_id"),
        }
    if isinstance(value, int):
        return {"offset": max(0, value), "next_index": 1, "active_task_id": None}
    return {"offset": 0, "next_index": 1, "active_task_id": None}


def write_state(
    path: Path | None,
    offset: int,
    next_index: int,
    active_task_id: str | None = None,
) -> None:
    if path is None:
        return
    path.write_text(
        json.dumps(
            {
                "active_task_id": active_task_id,
                "offset": offset,
                "next_index": next_index,
            },
            sort_keys=True,
        ),
        encoding="utf-8",
    )


def next_index_for_task(state: dict[str, Any], task_id: str | None) -> int:
    if task_id and state.get("active_task_id") != task_id:
        return 1
    return max(1, int(state.get("next_index") or 1))


def text_value(value: Any) -> str:
    if value is None:
        return "<null>"
    if isinstance(value, str):
        return value
    return json.dumps(value, ensure_ascii=False, sort_keys=True)


def compact_line(value: str) -> str:
    return value.replace("\r", "\\r").replace("\n", "\\n")


def print_field(name: str, value: Any, max_chars: int, indent: str) -> None:
    text = text_value(value)
    if len(text) <= max_chars and name not in LONG_FIELD_NAMES:
        print(f"{indent}{name}={compact_line(text)}")
        return
    if len(text) <= max_chars:
        print(f"{indent}{name}={compact_line(text)}")
        return
    half = max(80, max_chars // 2)
    print(f"{indent}{name}.chars={len(text)}")
    print(f"{indent}{name}.head={compact_line(text[:half])}")
    print(f"{indent}{name}.tail={compact_line(text[-half:])}")


def parse_json_text(value: Any) -> Any | None:
    if not isinstance(value, str):
        return None
    text = value.strip()
    if not text:
        return None
    try:
        return json.loads(text)
    except json.JSONDecodeError:
        return None


def parsed_json_fields(value: Any) -> str:
    parsed = parse_json_text(value)
    if isinstance(parsed, dict):
        return json.dumps(sorted(parsed.keys()), ensure_ascii=False)
    if isinstance(parsed, list):
        return f"list[{len(parsed)}]"
    if parsed is None:
        return "<not_json>"
    return type(parsed).__name__


def raw_response_metadata(raw_response: Any) -> tuple[Any, Any]:
    parsed = parse_json_text(raw_response)
    if not isinstance(parsed, dict):
        return None, None
    finish_reason = None
    choices = parsed.get("choices")
    if isinstance(choices, list) and choices:
        first = choices[0]
        if isinstance(first, dict):
            finish_reason = first.get("finish_reason")
    return finish_reason, parsed.get("usage")


def task_matches(row: dict[str, Any], task_id: str | None) -> bool:
    if not task_id:
        return True
    return str(row.get("task_id") or "") == task_id


def read_new_rows(log_path: Path, offset: int) -> tuple[list[tuple[int, str]], int]:
    if not log_path.exists():
        return [], offset
    with log_path.open("rb") as fh:
        fh.seek(offset)
        chunk = fh.read()
        new_offset = fh.tell()
    if not chunk:
        return [], new_offset
    rows: list[tuple[int, str]] = []
    cursor = offset
    for raw_line in chunk.splitlines():
        try:
            line = raw_line.decode("utf-8", errors="replace")
        except Exception:
            line = ""
        rows.append((cursor, line))
        cursor += len(raw_line) + 1
    return rows, new_offset


def print_row(
    row: dict[str, Any],
    index: int,
    log_path: Path,
    row_offset: int,
    max_chars: int,
    indent: str,
) -> None:
    prompt_source = row.get("prompt_source")
    prompt_label = row.get("prompt_label") or prompt_source
    logical_prompt_path = row.get("logical_prompt_path") or prompt_source
    response_text = row.get("clean_response") or row.get("response")
    finish_reason, raw_usage = raw_response_metadata(row.get("raw_response"))

    raw_fields = sorted(row.keys())
    print(f"{indent}[LLM#{index}]")
    print(f"{indent}  log_path={log_path}")
    print(f"{indent}  row_offset={row_offset}")
    print(f"{indent}  raw_fields={json.dumps(raw_fields, ensure_ascii=False)}")
    core_fields = {
        "task_id": row.get("task_id"),
        "call_id": row.get("call_id"),
        "prompt_label": prompt_label,
        "stage": row.get("stage") or prompt_source or row.get("mode"),
        "logical_prompt_path": logical_prompt_path,
        "prompt_source": prompt_source,
        "provider": row.get("provider"),
        "vendor": row.get("vendor"),
        "provider_type": row.get("provider_type"),
        "model": row.get("model"),
        "model_kind": row.get("model_kind"),
        "status": row.get("status"),
        "finish_reason": row.get("finish_reason") or finish_reason,
        "usage": row.get("usage"),
        "raw_usage": raw_usage,
        "error": row.get("error"),
        "parsed_json_fields": parsed_json_fields(response_text),
    }
    for name, value in core_fields.items():
        print_field(f"  {name}", value, max_chars, indent)

    print_field("  response_text", response_text, max_chars, indent)
    print_field("  raw_response", row.get("raw_response"), max_chars, indent)
    print_field("  request_payload", row.get("request_payload"), max_chars, indent)


def init_state(log_path: Path, state_file: Path | None) -> int:
    offset = log_path.stat().st_size if log_path.exists() else 0
    write_state(state_file, offset, 1)
    return 0


def run_self_test() -> int:
    with tempfile.TemporaryDirectory() as raw_tmp:
        tmp = Path(raw_tmp)
        log_path = tmp / "model_io.log"
        state_path = tmp / "state.json"
        row = {
            "task_id": "task-1",
            "call_id": "task-1:planner",
            "prompt_source": "planner",
            "provider": "vendor-test",
            "vendor": "test",
            "model": "test-model",
            "status": "ok",
            "clean_response": '{"action":"respond"}',
            "raw_response": json.dumps(
                {
                    "choices": [{"finish_reason": "stop"}],
                    "usage": {"prompt_tokens": 7, "completion_tokens": 3},
                }
            ),
            "usage": {"prompt_tokens": 7, "completion_tokens": 3, "total_tokens": 10},
        }
        log_path.write_text(json.dumps(row) + "\n", encoding="utf-8")
        output_lines: list[str] = []

        class Capture:
            def write(self, value: str) -> int:
                output_lines.append(value)
                return len(value)

            def flush(self) -> None:
                return None

        original_stdout = sys.stdout
        try:
            sys.stdout = Capture()  # type: ignore[assignment]
            rows, new_offset = read_new_rows(log_path, 0)
            next_index = 1
            for row_offset, raw_line in rows:
                parsed = json.loads(raw_line)
                print_row(parsed, next_index, log_path, row_offset, 600, "  ")
                next_index += 1
            write_state(state_path, new_offset, next_index, "task-1")
        finally:
            sys.stdout = original_stdout

        rendered = "".join(output_lines)
        assert "[LLM#1]" in rendered, rendered
        assert "raw_fields=" in rendered, rendered
        assert "finish_reason=stop" in rendered, rendered
        assert 'parsed_json_fields=["action"]' in rendered, rendered
        assert read_state(state_path)["next_index"] == 2
        assert read_state(state_path)["active_task_id"] == "task-1"
        assert next_index_for_task(read_state(state_path), "task-1") == 2
        assert next_index_for_task(read_state(state_path), "task-2") == 1
    print("SELF_TEST_OK")
    return 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--log", type=Path)
    parser.add_argument("--task-id")
    parser.add_argument("--state-file", type=Path)
    parser.add_argument("--init-state", action="store_true")
    parser.add_argument("--max-field-chars", type=int, default=DEFAULT_MAX_CHARS)
    parser.add_argument("--indent", default="  ")
    args = parser.parse_args(argv)

    if args.self_test:
        return run_self_test()
    if args.log is None:
        parser.error("--log is required unless --self-test is used")

    if args.max_field_chars < 240:
        parser.error("--max-field-chars must be >= 240")

    if args.init_state:
        return init_state(args.log, args.state_file)

    state = read_state(args.state_file)
    rows, new_offset = read_new_rows(args.log, state["offset"])
    next_index = next_index_for_task(state, args.task_id)
    printed = 0
    for row_offset, raw_line in rows:
        line = raw_line.strip()
        if not line:
            continue
        try:
            row = json.loads(line)
        except json.JSONDecodeError:
            continue
        if not isinstance(row, dict) or not task_matches(row, args.task_id):
            continue
        print_row(row, next_index, args.log, row_offset, args.max_field_chars, args.indent)
        next_index += 1
        printed += 1
    active_task_id = args.task_id or state.get("active_task_id")
    write_state(args.state_file, new_offset, next_index, active_task_id)
    if printed:
        task_part = f" task_id={args.task_id}" if args.task_id else ""
        print(
            f"{args.indent}[LLM_TRACE] rows_printed={printed}{task_part} "
            f"next_llm_index={next_index} log_path={args.log}"
        )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
