#!/usr/bin/env python3
"""Build a client-like continuous NL aggregate case file.

The historical NL case directory contains several row shapes:
- prompt
- name|prompt
- name|prompt|clarification_answer
- suite|name|tags|prompt[|expect=...]
- case_name|turn1|turn2|turn3[|turn4]
- suite|case_name|turn1|turn2|turn3

This script normalizes those shapes into the format consumed by
run_client_like_continuous_suite.sh:
  suite|name|tags|prompt|expect=optional substring
"""

from __future__ import annotations

import argparse
import re
from dataclasses import dataclass, replace
from pathlib import Path


CANONICAL_SUITES = {
    "act",
    "ask",
    "chat",
    "chat_act",
    "compound",
    "capability_boundary_regression",
    "continuous",
    "crypto",
    "dynamic_guard",
    "failure",
    "file",
    "fs_config_basic",
    "manual",
    "mixed",
    "regression",
    "schedule",
}

AUTH_CONTEXT_VALUES = {"user", "admin"}
METADATA_TAG_RE = re.compile(r"^[A-Za-z0-9_.:-]+$")

SIDE_EFFECT_NAME_TOKENS = (
    "absolute_saved_path",
    "builtin_write_file_deliver",
    "file_delivery",
    "generate_hello_sh",
    "mutate_write_file",
    "saved_path",
    "text_match_file_write_and_deliver",
    "workspace_note_path_only",
    "write_and_deliver",
    "write_file_run",
)

DRY_RUN_NAME_TOKENS = (
    "draft_preview",
)

ALLOW_CLARIFY_NAME_TOKENS = (
    "clarify",
    "followup_change_remaining_step",
    "followup_continue_synonym",
    "followup_discuss_then_continue",
    "followup_failure_reason_synonym",
    "followup_go_on",
    "followup_not_that_old",
    "followup_pick_up_from_there",
    "followup_redo_from_start",
    "followup_restart_not_resume",
    "just_a_path",
    "one_word",
)

RISKY_FILE_NAMES = {
    "nl_cases_sensitive_flows.txt",
    "nl_cases_ops_http_repair.txt",
}

GENERATED_FILE_NAMES = {
    "nl_cases_client_like_all_aggregate.txt",
}

RISKY_PROMPT_PATTERNS = [
    r"\{\{[^}]+\}\}",  # unresolved harness placeholders
    r"\bset\s+server\.listen\b",
    r"\btools\.allow_sudo\b",
    r"\bapply the config change directly\b",
    r"直接帮我改掉这个配置",
    r"后台启动一个本地静态 HTTP 服务",
    r"\bstart a static http server\b",
    r"\brepair .*index\.html\b",
    r"\bremove_file\b",
    r"\bdelete\b",
    r"删除\s+",
]

COMMENT_PREFIXES = ("#",)


def should_preserve_expect(tags: str, expect: str, preserve_expects: bool) -> bool:
    if not expect.strip():
        return False
    if preserve_expects:
        return True
    tagset = {part.strip().lower() for part in tags.split(",") if part.strip()}
    return "expect_exact_scalar" in tagset


@dataclass(frozen=True)
class CaseRow:
    suite: str
    name: str
    tags: str
    prompt: str
    expect: str = ""
    source: str = ""

    def line(self) -> str:
        base = "|".join(
            [
                sanitize_field(self.suite),
                sanitize_field(self.name),
                sanitize_tags(self.tags),
                self.prompt.replace("\t", " ").strip(),
            ]
        )
        if self.expect:
            return f"{base}|expect={self.expect.strip()}"
        return base


def sanitize_field(value: str) -> str:
    value = value.strip().replace("\t", " ")
    value = re.sub(r"\s+", "_", value)
    value = re.sub(r"[^A-Za-z0-9_.:-]+", "_", value)
    return value.strip("_") or "case"


def sanitize_tags(value: str) -> str:
    value = value.strip().replace("\t", " ")
    value = re.sub(r"\s+", "_", value)
    value = value.replace("|", "_")
    return value or "client_like,aggregate"


def split_tag_values(value: str) -> list[str]:
    return [tag.strip() for tag in value.split(",") if tag.strip()]


def append_tag(value: str, tag: str) -> str:
    tags = split_tag_values(value)
    lower_tags = {item.lower() for item in tags}
    if tag.lower() not in lower_tags:
        tags.append(tag)
    return ",".join(tags)


def derive_metadata_tags(row: CaseRow) -> CaseRow:
    """Derive safety tags from harness metadata only.

    The client-like aggregate intentionally avoids inspecting prompt text here:
    prompts are user-language samples, while case names and tags are stable
    machine metadata maintained by the NL test harness.
    """

    tags = split_tag_values(row.tags)
    lower_tags = {tag.lower() for tag in tags}
    lower_name = row.name.lower()
    lower_suite = row.suite.lower()

    has_side_effect = "side_effect" in lower_tags
    has_side_effect = has_side_effect or "write_and_deliver" in lower_tags
    has_side_effect = has_side_effect or lower_suite == "schedule"
    has_side_effect = has_side_effect or (
        "mutate" in lower_tags and "dry_run" not in lower_tags
    )
    has_side_effect = has_side_effect or any(
        token in lower_name for token in SIDE_EFFECT_NAME_TOKENS
    )

    derived_tags = row.tags
    if has_side_effect:
        derived_tags = append_tag(derived_tags, "side_effect")
    if any(token in lower_name for token in DRY_RUN_NAME_TOKENS):
        derived_tags = append_tag(derived_tags, "dry_run")
    if (
        lower_suite == "ask"
        or "clarify" in lower_tags
        or any(token in lower_name for token in ALLOW_CLARIFY_NAME_TOKENS)
    ):
        derived_tags = append_tag(derived_tags, "allow_clarify")
    if derived_tags == row.tags:
        return row
    return replace(row, tags=derived_tags)


def should_skip_prompt(prompt: str, include_risky: bool) -> bool:
    if include_risky:
        return False
    lower = prompt.lower()
    return any(re.search(pattern, lower, flags=re.IGNORECASE) for pattern in RISKY_PROMPT_PATTERNS)


def is_comment_or_empty(line: str) -> bool:
    stripped = line.strip()
    return not stripped or stripped.startswith(COMMENT_PREFIXES)


def split_columns(line: str) -> list[str]:
    return [part.strip() for part in line.split("|")]


def split_canonical(line: str) -> list[str]:
    return [part.strip() for part in line.split("|", 3)]


def split_five_column(line: str) -> list[str]:
    return [part.strip() for part in line.split("|", 4)]


def split_expect_suffix(prompt: str) -> tuple[str, str]:
    marker = "|expect="
    if marker not in prompt:
        return prompt.strip(), ""
    prompt_part, expect = prompt.rsplit(marker, 1)
    return prompt_part.strip(), expect.strip()


def looks_like_metadata_tags(value: str) -> bool:
    """Return true for harness metadata tags, not user turns.

    Historical case files use both `suite|name|tags|prompt` and
    `case_name|turn1|turn2|turn3`. Unknown suite names are common, so the
    aggregate builder cannot rely only on a fixed suite allowlist. The tags
    column, however, is machine metadata: empty or ASCII identifiers separated
    by commas, such as `act,fs,list`, `skill:read_file`, or `write_and_deliver`.
    """

    stripped = value.strip()
    if not stripped:
        return True
    return all(METADATA_TAG_RE.fullmatch(part.strip()) for part in stripped.split(","))


def row_from_prompt(
    source: Path,
    index: int,
    prompt: str,
    *,
    suite: str = "aggregate",
    name_prefix: str = "",
    tags: str = "client_like,aggregate",
) -> CaseRow:
    stem = sanitize_field(source.stem)
    name = sanitize_field(name_prefix or f"{stem}_{index:04d}")
    return CaseRow(suite=suite, name=name, tags=tags, prompt=prompt, source=str(source))


def expand_turns(source: Path, case_name: str, turns: list[str], tags: str) -> list[CaseRow]:
    rows: list[CaseRow] = []
    clean_turns = [turn.strip() for turn in turns if turn.strip()]
    for idx, turn in enumerate(clean_turns, 1):
        rows.append(
            CaseRow(
                suite="continuous",
                name=sanitize_field(f"{case_name}_turn{idx}"),
                tags=tags,
                prompt=turn,
                source=str(source),
            )
        )
    return rows


def parse_line(source: Path, line: str, index: int, preserve_expects: bool) -> list[CaseRow]:
    cols = split_columns(line)
    canonical = split_canonical(line)
    source_stem = sanitize_field(source.stem)

    if len(cols) == 1:
        return [
            row_from_prompt(
                source,
                index,
                cols[0],
                suite="single",
                name_prefix=f"{source_stem}_{index:04d}",
                tags="single,client_like,aggregate",
            )
        ]

    if len(cols) == 2:
        name, prompt = cols
        return [
            CaseRow(
                suite="single",
                name=sanitize_field(f"{source_stem}_{name}"),
                tags="two_column,client_like,aggregate",
                prompt=prompt,
                source=str(source),
            )
        ]

    if len(cols) == 3:
        name, prompt, answer = cols
        return expand_turns(
            source,
            f"{source_stem}_{name}",
            [prompt, answer],
            "clarify_chain,client_like,aggregate",
        )

    first = cols[0].strip()
    first_norm = first.strip().lower()

    if first_norm == "context_chain":
        _, case_name, *turns = cols
        return expand_turns(
            source,
            f"{source_stem}_{case_name}",
            turns,
            "context_chain,client_like,aggregate",
        )

    if first_norm == "clarify":
        _, case_name, *turns = cols
        return expand_turns(
            source,
            f"{source_stem}_{case_name}",
            turns,
            "clarify_chain,client_like,aggregate",
        )

    if first.startswith(("task_updates", "context_", "followup_", "clarify_")):
        case_name, *turns = cols
        return expand_turns(
            source,
            f"{source_stem}_{case_name}",
            turns,
            "turn_chain,client_like,aggregate",
        )

    five = split_five_column(line)
    if len(five) == 5 and five[1].strip().lower() in AUTH_CONTEXT_VALUES:
        # Structured assertion shape:
        #   name|auth|assertion|expected|prompt
        # `auth`, `assertion`, and `expected` are harness metadata, not user
        # turns. Keep the real prompt as a single client-like case.
        name, auth, assertion, expected, prompt = five
        expect = expected if preserve_expects else ""
        return [
            CaseRow(
                suite="single",
                name=sanitize_field(f"{source_stem}_{name}"),
                tags=",".join(
                    part
                    for part in [
                        f"auth:{sanitize_field(auth)}",
                        f"assertion:{sanitize_field(assertion)}",
                        "structured_assertion",
                        "client_like",
                        "aggregate",
                    ]
                    if part
                ),
                prompt=prompt,
                expect=expect,
                source=str(source),
            )
        ]

    if len(canonical) == 4 and (
        first_norm in CANONICAL_SUITES or looks_like_metadata_tags(canonical[2])
    ):
        suite, name, tags, prompt = canonical
        prompt, stripped_expect = split_expect_suffix(prompt)
        expect = (
            stripped_expect
            if should_preserve_expect(tags, stripped_expect, preserve_expects)
            else ""
        )
        return [
            CaseRow(
                suite=suite.strip(),
                name=sanitize_field(f"{source_stem}_{name}"),
                tags=",".join(
                    part
                    for part in [
                        tags.strip(),
                        "client_like",
                        "aggregate",
                    ]
                    if part
                ),
                prompt=prompt,
                expect=expect,
                source=str(source),
            )
        ]

    if len(cols) >= 4:
        # Legacy multi-turn shape:
        #   case_name|turn1|turn2|turn3[|turn4]
        # Expand every turn so client-like aggregate tests preserve the
        # semantic setup instead of running only the final refinement.
        name, *turns = cols
        return expand_turns(
            source,
            f"{source_stem}_{name}",
            turns,
            "legacy_shape,turn_chain,client_like,aggregate",
        )

    return []


def build_rows(
    cases_dir: Path,
    include_risky: bool,
    include_temp: bool,
    preserve_expects: bool,
) -> tuple[list[CaseRow], dict[str, int]]:
    rows: list[CaseRow] = []
    stats = {
        "files_seen": 0,
        "files_skipped_temp": 0,
        "files_skipped_risky": 0,
        "rows_seen": 0,
        "rows_expect_dropped": 0,
        "rows_skipped_risky": 0,
        "rows_emitted": 0,
        "rows_deduped": 0,
    }
    seen: set[tuple[str, str]] = set()
    files = sorted(cases_dir.rglob("*.txt"))
    for path in files:
        if path.name in GENERATED_FILE_NAMES:
            continue
        if not include_temp and path.name.startswith("_tmp_"):
            stats["files_skipped_temp"] += 1
            continue
        if not include_risky and path.name in RISKY_FILE_NAMES:
            stats["files_skipped_risky"] += 1
            continue
        stats["files_seen"] += 1
        line_index = 0
        for raw in path.read_text(encoding="utf-8", errors="ignore").splitlines():
            if is_comment_or_empty(raw):
                continue
            line_index += 1
            stats["rows_seen"] += 1
            if not preserve_expects and "|expect=" in raw:
                stats["rows_expect_dropped"] += 1
            for row in parse_line(path, raw.strip(), line_index, preserve_expects):
                if should_skip_prompt(row.prompt, include_risky):
                    stats["rows_skipped_risky"] += 1
                    continue
                row = derive_metadata_tags(row)
                if row.suite != "continuous":
                    key = (canonical_prompt_key(row.prompt), row.expect.strip())
                    if key in seen:
                        stats["rows_deduped"] += 1
                        continue
                    seen.add(key)
                rows.append(row)
    stats["rows_emitted"] = len(rows)
    return rows, stats


def canonical_prompt_key(prompt: str) -> str:
    return re.sub(r"\s+", " ", prompt).strip()


def repeated_case_name(name: str, repeat_index: int) -> str:
    suffix = f"_repeat{repeat_index:02d}"
    match = re.search(r"(_turn[0-9]+)$", name)
    if match:
        return sanitize_field(f"{name[: match.start()]}{suffix}{match.group(1)}")
    return sanitize_field(f"{name}{suffix}")


def extend_rows_to_target(rows: list[CaseRow], target_rows: int) -> tuple[list[CaseRow], int]:
    if target_rows <= 0 or len(rows) >= target_rows:
        return rows, 0
    if not rows:
        raise ValueError("cannot extend an empty aggregate to a target row count")

    extended = list(rows)
    extra_index = 0
    while len(extended) < target_rows:
        row = rows[extra_index % len(rows)]
        repeat_index = (extra_index // len(rows)) + 2
        extended.append(
            CaseRow(
                suite=row.suite,
                name=repeated_case_name(row.name, repeat_index),
                tags=row.tags,
                prompt=row.prompt,
                expect=row.expect,
                source=row.source,
            )
        )
        extra_index += 1
    return extended, len(extended) - len(rows)


def render_aggregate(out_path: Path, rows: list[CaseRow], stats: dict[str, int], include_risky: bool) -> str:
    lines = [
        "# Generated client-like continuous NL aggregate.",
        "# Do not edit by hand; regenerate with scripts/nl_tests/build_client_like_case_aggregate.py.",
        "# Deduplication: global prompt text for non-continuous cases; continuous chains keep duplicate prompts.",
        "# Run:",
        f"#   bash scripts/nl_tests/run_client_like_continuous_suite.sh --case-file {out_path.as_posix()} --prompt-reply-only --quality-guard",
        f"# include_risky={include_risky}",
        "# stats="
        + " ".join(f"{key}={value}" for key, value in sorted(stats.items())),
        "# target_rows=0 means no row-count padding.",
        "# Format: suite|name|tags|prompt|expect=optional substring",
        "",
    ]
    current_source = ""
    for row in rows:
        if row.source != current_source:
            current_source = row.source
            lines.append(f"# source: {current_source}")
        lines.append(row.line())
    return "\n".join(lines) + "\n"


def write_aggregate(out_path: Path, rows: list[CaseRow], stats: dict[str, int], include_risky: bool) -> None:
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(render_aggregate(out_path, rows, stats, include_risky), encoding="utf-8")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--cases-dir",
        default="scripts/nl_tests/cases",
        help="Case directory to scan.",
    )
    parser.add_argument(
        "--out",
        default="scripts/nl_tests/cases/nl_cases_client_like_all_aggregate.txt",
        help="Aggregate case file to write.",
    )
    parser.add_argument(
        "--include-risky",
        action="store_true",
        help="Include mutating/config/placeholder cases. Default keeps the aggregate safe.",
    )
    parser.add_argument(
        "--include-temp",
        action="store_true",
        help="Include _tmp_ case files.",
    )
    parser.add_argument(
        "--preserve-expects",
        action="store_true",
        help=(
            "Preserve legacy expect= substring checks. Default drops them because "
            "the client-like aggregate relies on quality-guard and many old "
            "single-shot expects conflict with exact-output contracts."
        ),
    )
    parser.add_argument(
        "--target-rows",
        type=int,
        default=2100,
        help=(
            "Pad the aggregate to this many executable rows by replaying existing "
            "case prompts with unique case names. The default leaves enough "
            "headroom for a 2000-case safe run after excluded tags. Use 0 to "
            "disable padding."
        ),
    )
    parser.add_argument(
        "--check",
        action="store_true",
        help="Exit non-zero if the aggregate file is missing or not up to date.",
    )
    args = parser.parse_args()

    cases_dir = Path(args.cases_dir)
    out_path = Path(args.out)
    rows, stats = build_rows(
        cases_dir,
        include_risky=args.include_risky,
        include_temp=args.include_temp,
        preserve_expects=args.preserve_expects,
    )
    rows, padded_rows = extend_rows_to_target(rows, args.target_rows)
    stats["rows_padded"] = padded_rows
    stats["rows_output"] = len(rows)
    if args.check:
        expected = render_aggregate(out_path, rows, stats, include_risky=args.include_risky)
        actual = out_path.read_text(encoding="utf-8") if out_path.exists() else ""
        if actual != expected:
            print(
                "CLIENT_LIKE_AGGREGATE_OUTDATED "
                f"out={out_path} "
                + " ".join(f"{key}={value}" for key, value in sorted(stats.items()))
            )
            return 1
        print(
            "CLIENT_LIKE_AGGREGATE_OK "
            f"out={out_path} "
            + " ".join(f"{key}={value}" for key, value in sorted(stats.items()))
        )
        return 0

    write_aggregate(out_path, rows, stats, include_risky=args.include_risky)
    print(
        "CLIENT_LIKE_AGGREGATE_BUILT "
        f"out={out_path} "
        + " ".join(f"{key}={value}" for key, value in sorted(stats.items()))
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
