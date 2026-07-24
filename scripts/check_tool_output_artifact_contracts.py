#!/usr/bin/env python3
"""Guard streaming tool-output artifacts and planner-visible range handles."""

from __future__ import annotations

import argparse
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]

REQUIRED_TOKENS: dict[str, tuple[str, ...]] = {
    "crates/clawd/src/skills/builtin_run_cmd.rs": (
        "mpsc::channel(64)",
        "CommandOutputArtifactWriter::new",
        "output_hard_limit",
        "machine_projection",
    ),
    "crates/clawd/src/skills/builtin_run_cmd_artifact.rs": (
        '"kind": "tool_output_artifact"',
        '"artifact_refs"',
        '"range_handles"',
        '"read_capability": "artifact.read_range"',
        '"size_bytes"',
        '"sha256"',
    ),
    "crates/clawd/src/capability_result.rs": (
        "artifact_refs_from_sources(output, extra)",
        "serde_json::from_str::<Value>(output.trim())",
    ),
    "crates/clawd/src/skills/builtin_run_cmd_artifact_tests.rs": (
        "large_output_is_streamed_to_artifacts_with_range_handles",
        "small_output_stays_inline_without_artifact_files",
    ),
    "crates/clawd/src/capability_result_tests.rs": (
        "machine_output_artifact_refs_are_promoted_to_the_result_envelope",
    ),
}

FORBIDDEN_TOKENS: dict[str, tuple[str, ...]] = {
    "crates/clawd/src/skills/builtin_run_cmd.rs": (
        "mpsc::unbounded_channel()",
        "output_limit_reached",
        "output limit reached; killing shell",
    ),
}


def scan_texts(texts: dict[str, str | None]) -> list[str]:
    findings: list[str] = []
    for rel_path, tokens in REQUIRED_TOKENS.items():
        text = texts.get(rel_path)
        if text is None:
            findings.append(f"missing_or_unreadable:{rel_path}")
            continue
        for token in tokens:
            if token not in text:
                findings.append(f"missing_token:{rel_path}:{token}")
    for rel_path, tokens in FORBIDDEN_TOKENS.items():
        text = texts.get(rel_path)
        if text is None:
            findings.append(f"missing_or_unreadable:{rel_path}")
            continue
        for token in tokens:
            if token in text:
                findings.append(f"forbidden_token:{rel_path}:{token}")
    return findings


def read_repo_texts() -> dict[str, str | None]:
    paths = set(REQUIRED_TOKENS) | set(FORBIDDEN_TOKENS)
    out: dict[str, str | None] = {}
    for rel_path in paths:
        try:
            out[rel_path] = (ROOT / rel_path).read_text(encoding="utf-8")
        except (FileNotFoundError, UnicodeDecodeError):
            out[rel_path] = None
    return out


def minimal_good_texts() -> dict[str, str | None]:
    texts = {
        rel_path: "\n".join(tokens)
        for rel_path, tokens in REQUIRED_TOKENS.items()
    }
    for rel_path in FORBIDDEN_TOKENS:
        texts.setdefault(rel_path, "")
    return texts


def run_self_test() -> None:
    good = minimal_good_texts()
    assert not scan_texts(good)

    missing = dict(good)
    missing["crates/clawd/src/skills/builtin_run_cmd_artifact.rs"] = (
        '"kind": "tool_output_artifact"'
    )
    assert any("range_handles" in item for item in scan_texts(missing))

    regressed = dict(good)
    regressed["crates/clawd/src/skills/builtin_run_cmd.rs"] += (
        "\nmpsc::unbounded_channel()"
    )
    assert any("forbidden_token" in item for item in scan_texts(regressed))
    print("TOOL_OUTPUT_ARTIFACT_CONTRACT_SELF_TEST ok")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        run_self_test()
        return 0
    findings = scan_texts(read_repo_texts())
    if findings:
        print(f"TOOL_OUTPUT_ARTIFACT_CONTRACT_CHECK findings={len(findings)}")
        for finding in findings:
            print(finding)
        return 1
    print("TOOL_OUTPUT_ARTIFACT_CONTRACT_CHECK findings=0")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
