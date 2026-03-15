#!/usr/bin/env python3
import argparse
import json
from collections import Counter
from pathlib import Path
from typing import Dict, List


def parse_markdown_table(md_text: str) -> List[Dict[str, str]]:
    lines = md_text.splitlines()
    header_idx = None
    for i, line in enumerate(lines):
        if line.strip().lower().startswith("| id |") and "expected_mode" in line and "actual_mode" in line:
            header_idx = i
            break
    if header_idx is None or header_idx + 2 >= len(lines):
        return []

    headers = [h.strip() for h in lines[header_idx].strip().strip("|").split("|")]
    rows: List[Dict[str, str]] = []
    for line in lines[header_idx + 2 :]:
        s = line.strip()
        if not s.startswith("|"):
            break
        if set(s.replace("|", "").replace("-", "").strip()) == set():
            continue
        cols = [c.strip() for c in s.strip().strip("|").split("|")]
        if len(cols) != len(headers):
            continue
        row = dict(zip(headers, cols))
        if row.get("id", "").startswith("R-"):
            rows.append(row)
    return rows


def ratio(numerator: int, denominator: int) -> str:
    if denominator <= 0:
        return "0/0 (0.0%)"
    pct = (numerator / denominator) * 100
    return f"{numerator}/{denominator} ({pct:.1f}%)"


def main() -> None:
    parser = argparse.ArgumentParser(description="Compute routing replay metrics from failed-routing-cases.md")
    parser.add_argument(
        "--path",
        default="failed-routing-cases.md",
        help="Path to failed-routing-cases markdown file (default: failed-routing-cases.md)",
    )
    parser.add_argument(
        "--status",
        default="open",
        choices=["open", "fixed", "wontfix", "all"],
        help="Filter by case status (default: open)",
    )
    parser.add_argument("--json", action="store_true", help="Output JSON")
    args = parser.parse_args()

    md_path = Path(args.path)
    if not md_path.exists():
        raise SystemExit(f"file not found: {md_path}")

    rows = parse_markdown_table(md_path.read_text(encoding="utf-8"))
    if args.status != "all":
        rows = [r for r in rows if r.get("status", "").strip().lower() == args.status]

    samples_total = len(rows)
    mode_correct = sum(1 for r in rows if r.get("actual_mode") == r.get("expected_mode"))
    profile_correct = sum(1 for r in rows if r.get("selected_profile") == r.get("expected_profile"))

    high_rows = [r for r in rows if r.get("impact", "").strip().lower() == "high"]
    high_mode_correct = sum(1 for r in high_rows if r.get("actual_mode") == r.get("expected_mode"))
    high_profile_correct = sum(1 for r in high_rows if r.get("selected_profile") == r.get("expected_profile"))

    chat_act_rows = [r for r in rows if r.get("expected_mode") == "chat_act"]
    chat_act_correct = sum(1 for r in chat_act_rows if r.get("actual_mode") == "chat_act")

    ask_clarify_rows = [r for r in rows if r.get("expected_mode") == "ask_clarify"]
    ask_clarify_misexecute_count = sum(
        1 for r in ask_clarify_rows if r.get("actual_mode") in {"act", "chat_act"}
    )

    root_cause_counter = Counter(r.get("root_cause", "").strip() for r in rows if r.get("root_cause"))
    root_cause_top3 = root_cause_counter.most_common(3)

    result = {
        "samples_total": samples_total,
        "mode_accuracy": ratio(mode_correct, samples_total),
        "profile_accuracy": ratio(profile_correct, samples_total),
        "high_impact_mode_accuracy": ratio(high_mode_correct, len(high_rows)),
        "high_impact_profile_accuracy": ratio(high_profile_correct, len(high_rows)),
        "chat_act_accuracy": ratio(chat_act_correct, len(chat_act_rows)),
        "ask_clarify_misexecute_count": ask_clarify_misexecute_count,
        "root_cause_top3": root_cause_top3,
    }

    if args.json:
        print(json.dumps(result, ensure_ascii=False, indent=2))
        return

    print(f"samples_total: {result['samples_total']}")
    print(f"mode_accuracy: {result['mode_accuracy']}")
    print(f"profile_accuracy: {result['profile_accuracy']}")
    print(f"high_impact_mode_accuracy: {result['high_impact_mode_accuracy']}")
    print(f"high_impact_profile_accuracy: {result['high_impact_profile_accuracy']}")
    print(f"chat_act_accuracy: {result['chat_act_accuracy']}")
    print(f"ask_clarify_misexecute_count: {result['ask_clarify_misexecute_count']}")
    print(f"root_cause_top3: {result['root_cause_top3']}")


if __name__ == "__main__":
    main()
