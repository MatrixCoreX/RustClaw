#!/usr/bin/env python3
"""Render official cargo-audit JSON without an Issue API or advisory filtering."""
import argparse
import html
import json
from pathlib import Path


def cell(value):
    return html.escape(" ".join(str(value).splitlines())).replace("|", "\\|").replace("`", "\\`")


def render_report(report):
    vulnerabilities, warnings = report["vulnerabilities"], report["warnings"]
    count = vulnerabilities["count"]
    if (type(count) is not int or count < 0 or type(vulnerabilities["found"]) is not bool
            or vulnerabilities["found"] != (count > 0)
            or not isinstance(vulnerabilities["list"], list)
            or len(vulnerabilities["list"]) != count or not isinstance(warnings, dict)):
        raise ValueError("invalid audit counters")
    rows = [("vulnerability", entry) for entry in vulnerabilities["list"]]
    for kind, entries in sorted(warnings.items()):
        if not isinstance(entries, list):
            raise ValueError("invalid audit warnings")
        rows.extend((kind, entry) for entry in entries)
    warning_count = len(rows) - count
    lines = ["## Rust dependency audit", "", f"Vulnerabilities: {count}",
             f"Warnings: {warning_count}", "",
             "Warnings remain visible and require review; they are not automatically ignored.", "",
             "| Class | Package | Version | Advisory |", "| --- | --- | --- | --- |"]
    for kind, entry in rows:
        package = entry["package"]
        advisory = entry.get("advisory") or {}
        lines.append("| " + " | ".join(cell(value) for value in
            (kind, package["name"], package["version"], advisory.get("id", "-"))) + " |")
    return "\n".join(lines) + "\n", count, warning_count


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("report", type=Path)
    parser.add_argument("--audit-exit-code", required=True, type=int, choices=range(256))
    parser.add_argument("--summary", required=True, type=Path)
    args = parser.parse_args(argv)
    try:
        summary, vulnerabilities, warnings = render_report(json.loads(args.report.read_text()))
    except (OSError, ValueError, KeyError, TypeError, AttributeError):
        print("::error title=Rust dependency audit::Missing or invalid cargo-audit JSON; inspect the retained audit artifact.")
        return 2
    summary += f"\nOfficial cargo-audit exit code: {args.audit_exit_code}\n"
    with args.summary.open("a", encoding="utf-8") as stream:
        stream.write(summary)
    print(summary)
    if warnings:
        print(f"::warning title=Rust dependency audit::{warnings} dependency warnings; review the summary and retained artifact.")
    # A transport/database/tool failure must never become green just because
    # its JSON contains no known vulnerability entries.
    return args.audit_exit_code or int(vulnerabilities > 0)


if __name__ == "__main__":
    raise SystemExit(main())
