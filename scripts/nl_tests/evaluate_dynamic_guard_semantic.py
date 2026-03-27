#!/usr/bin/env python3
import argparse
import json
import re
from pathlib import Path
from typing import Dict, List, Optional, Tuple

PATH_RE = re.compile(r"(?P<path>(?:/|\./|\.\./)[^\s\"'`，,;；:：)）(]+)")
NUM_RE = re.compile(r"^-?\d+(?:\.\d+)?$")

NOT_FOUND_MARKERS = [
    "not found",
    "file not found",
    "未找到",
    "不存在",
    "无法读取",
    "不可获取",
    "cannot read",
    "unable to read",
]

GENERIC_REASK_MARKERS = [
    "what would you like me to do with",
    "do you want me to",
    "please provide",
    "请告诉我",
    "请提供",
    "你想让我",
]

DELIVERY_MARKERS = [
    "发给我",
    "发我",
    "发一下",
    "发送给我",
    "send me",
    "send it",
    "deliver",
    "as a file",
]

ONE_SENTENCE_MARKERS = [
    "一句话",
    "one sentence",
]

SCALAR_MARKERS = [
    "只输出数字",
    "只输出值",
    "only output number",
    "only output the value",
    "only output value",
]


def read_jsonl(path: Path) -> List[Dict]:
    rows: List[Dict] = []
    if not path.is_file():
        return rows
    for raw in path.read_text(encoding="utf-8").splitlines():
        raw = raw.strip()
        if not raw:
            continue
        try:
            rows.append(json.loads(raw))
        except Exception:
            continue
    return rows


def normalize_text(s: Optional[str]) -> str:
    return (s or "").strip()


def lower_text(s: Optional[str]) -> str:
    return normalize_text(s).lower()


def extract_explicit_path(text: str) -> Optional[str]:
    if not text:
        return None
    m = PATH_RE.search(text)
    if not m:
        return None
    path = m.group("path").strip()
    return path or None


def resolve_path(path_text: Optional[str], workspace_root: Path) -> Optional[Path]:
    if not path_text:
        return None
    p = Path(path_text)
    if p.is_absolute():
        return p
    return (workspace_root / p).resolve()


def file_exists(path_text: Optional[str], workspace_root: Path) -> bool:
    p = resolve_path(path_text, workspace_root)
    if p is None:
        return False
    return p.is_file()


def has_marker(text: str, markers: List[str]) -> bool:
    t = lower_text(text)
    return any(m in t for m in markers)


def text_looks_not_found(text: str) -> bool:
    return has_marker(text, NOT_FOUND_MARKERS)


def text_looks_generic_reask(text: str) -> bool:
    return has_marker(text, GENERIC_REASK_MARKERS)


def prompt_is_delivery(prompt: str) -> bool:
    return has_marker(prompt, DELIVERY_MARKERS)


def prompt_requires_one_sentence(prompt: str) -> bool:
    return has_marker(prompt, ONE_SENTENCE_MARKERS)


def prompt_requires_scalar(prompt: str) -> bool:
    return has_marker(prompt, SCALAR_MARKERS)


def is_question_like(text: str) -> bool:
    t = normalize_text(text)
    if not t:
        return False
    if "?" in t or "？" in t:
        return True
    return text_looks_generic_reask(t)


def count_sentence_like_units(text: str) -> int:
    t = normalize_text(text)
    if not t:
        return 0
    # split by sentence punctuation and line breaks
    chunks = re.split(r"[\n\r]+|[。！？!?]+", t)
    chunks = [c.strip() for c in chunks if c.strip()]
    return len(chunks)


def collect_file_tokens(text: str, messages: List[str]) -> List[str]:
    out: List[str] = []
    for line in normalize_text(text).splitlines():
        line = line.strip()
        if line.startswith("FILE:"):
            out.append(line)
    for msg in messages:
        for line in normalize_text(msg).splitlines():
            line = line.strip()
            if line.startswith("FILE:"):
                out.append(line)
    return out


def evaluate_manual_row(row: Dict, workspace_root: Path) -> Tuple[bool, List[str]]:
    reasons: List[str] = []
    status = str(row.get("status") or "")
    prompt = normalize_text(row.get("prompt"))
    text = normalize_text(row.get("text"))
    messages = row.get("messages") or []
    if not isinstance(messages, list):
        messages = []

    if status != "succeeded":
        reasons.append(f"status={status}")

    explicit_path = extract_explicit_path(prompt)
    explicit_path_exists = file_exists(explicit_path, workspace_root)

    if explicit_path and explicit_path_exists and text_looks_not_found(text):
        reasons.append("explicit path exists but output says not found/unreadable")

    if prompt_is_delivery(prompt) and explicit_path and explicit_path_exists:
        file_tokens = collect_file_tokens(text, messages)
        if not file_tokens:
            reasons.append("delivery request with existing explicit path but no FILE token")

    if prompt_requires_one_sentence(prompt):
        if count_sentence_like_units(text) != 1:
            reasons.append("one-sentence requirement violated")

    if prompt_requires_scalar(prompt):
        if not NUM_RE.match(text):
            reasons.append("scalar-only requirement violated")

    return (len(reasons) == 0), reasons


def evaluate_clarify_row(row: Dict, workspace_root: Path) -> Tuple[bool, List[str]]:
    reasons: List[str] = []
    case_name = str(row.get("case_name") or "")
    turn1 = row.get("turn1") or {}
    turn2 = row.get("turn2") or {}

    t1_status = str(turn1.get("status") or "")
    t2_status = str(turn2.get("status") or "")
    t1_prompt = normalize_text(turn1.get("prompt"))
    t2_prompt = normalize_text(turn2.get("prompt"))
    t1_text = normalize_text(turn1.get("text"))
    t2_text = normalize_text(turn2.get("text"))

    if t1_status != "succeeded":
        reasons.append(f"turn1 status={t1_status}")
    if t2_status != "succeeded":
        reasons.append(f"turn2 status={t2_status}")

    if not is_question_like(t1_text):
        reasons.append("turn1 should clarify first but does not look like a clarification")

    if text_looks_generic_reask(t2_text):
        reasons.append("turn2 should execute after locator but re-asks generic question")

    t2_path = extract_explicit_path(t2_prompt)
    t2_path_exists = file_exists(t2_path, workspace_root)

    if t2_path and t2_path_exists and text_looks_not_found(t2_text):
        reasons.append("turn2 locator path exists but output says not found/unreadable")

    if prompt_is_delivery(t1_prompt) and t2_path and t2_path_exists:
        if not collect_file_tokens(t2_text, []):
            reasons.append("turn2 delivery result missing FILE token")

    if "log_tail_focus" in case_name and "model_io.log" in t1_text:
        reasons.append("turn1 drifted to unrelated model_io.log")

    return (len(reasons) == 0), reasons


def evaluate_context_row(row: Dict, workspace_root: Path) -> Tuple[bool, List[str]]:
    reasons: List[str] = []
    case_name = str(row.get("case_name") or "")
    turns = [row.get("turn1") or {}, row.get("turn2") or {}, row.get("turn3") or {}]

    for idx, turn in enumerate(turns, start=1):
        status = str(turn.get("status") or "")
        prompt = normalize_text(turn.get("prompt"))
        text = normalize_text(turn.get("text"))
        if status != "succeeded":
            reasons.append(f"turn{idx} status={status}")
        explicit_path = extract_explicit_path(prompt)
        if explicit_path and file_exists(explicit_path, workspace_root) and text_looks_not_found(text):
            reasons.append(f"turn{idx} explicit path exists but output says not found/unreadable")
        if prompt_requires_one_sentence(prompt) and count_sentence_like_units(text) != 1:
            reasons.append(f"turn{idx} one-sentence requirement violated")
        if prompt_requires_scalar(prompt) and not NUM_RE.match(text):
            reasons.append(f"turn{idx} scalar-only requirement violated")

    turn3_text = normalize_text((row.get("turn3") or {}).get("text"))
    if "context_two_targets_disambiguate" in case_name and text_looks_generic_reask(turn3_text):
        reasons.append("turn3 should execute bound log tail but returned generic re-ask")
    if "context_two_targets_disambiguate" in case_name and "no attempt to read" in lower_text(turn3_text):
        reasons.append("turn3 indicates no execution attempt for already bound target")
    if "context_bound_alias_rebind" in case_name and "把「乙」" in turn3_text:
        reasons.append("alias rebind response contains unrelated phantom alias")

    return (len(reasons) == 0), reasons


def main() -> int:
    parser = argparse.ArgumentParser(description="Semantic evaluator for dynamic guard suites")
    parser.add_argument("--suite", required=True, choices=["manual", "clarify", "context_chain"])
    parser.add_argument("--case-file", required=False)
    parser.add_argument("--summary-jsonl", required=True)
    parser.add_argument("--report-jsonl", required=True)
    parser.add_argument("--workspace-root", required=True)
    parser.add_argument("--fail-on-fail", action="store_true")
    args = parser.parse_args()

    summary_path = Path(args.summary_jsonl)
    report_path = Path(args.report_jsonl)
    workspace_root = Path(args.workspace_root)

    rows = read_jsonl(summary_path)
    report_rows: List[Dict] = []

    for row in rows:
        if args.suite == "manual":
            passed, reasons = evaluate_manual_row(row, workspace_root)
        elif args.suite == "clarify":
            passed, reasons = evaluate_clarify_row(row, workspace_root)
        else:
            passed, reasons = evaluate_context_row(row, workspace_root)
        report = {
            "suite": args.suite,
            "case_name": row.get("case_name"),
            "pass": passed,
            "reasons": reasons,
        }
        report_rows.append(report)

    report_path.parent.mkdir(parents=True, exist_ok=True)
    with report_path.open("w", encoding="utf-8") as f:
        for row in report_rows:
            f.write(json.dumps(row, ensure_ascii=False) + "\n")

    total = len(report_rows)
    passed = sum(1 for r in report_rows if r.get("pass"))
    failed = total - passed
    print(f"SEMANTIC_SUMMARY suite={args.suite} total={total} pass={passed} fail={failed}")
    if failed:
        print(f"SEMANTIC_REPORT {report_path}")
        for row in report_rows:
            if row.get("pass"):
                continue
            reasons = "; ".join(row.get("reasons") or [])
            print(f"  - {row.get('case_name')}: {reasons}")

    if args.fail_on_fail and failed > 0:
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
