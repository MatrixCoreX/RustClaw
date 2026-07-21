#!/usr/bin/env python3
"""Guard the persisted child-task DAG, trusted roles, and ownership boundary."""

from __future__ import annotations

import argparse
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]

REQUIRED_TOKENS: dict[str, tuple[str, ...]] = {
    "migrations/001_init.sql": (
        "CREATE TABLE IF NOT EXISTS child_task_graphs",
        "CREATE TABLE IF NOT EXISTS child_task_graph_nodes",
        "CREATE TABLE IF NOT EXISTS child_task_graph_edges",
        "owned_paths_json",
        "steering_version",
    ),
    "crates/clawd/src/repo/child_task_graph.rs": (
        "prepare_child_task_graph",
        "ensure_acyclic",
        "path_ownership_serialization",
        "record_child_graph_terminal",
        "record_child_graph_steering",
        "replace_child_graph_node_for_retry",
        "reconcile_child_task_graphs_after_restart",
        "terminate_parent_graph_children",
    ),
    "crates/clawd/src/repo/tasks.rs": (
        "LEFT JOIN child_task_graph_nodes graph_node",
        "graph_node.readiness IN ('ready', 'running')",
        "mark_child_graph_node_running",
        "terminate_parent_graph_children",
    ),
    "crates/clawd/src/repo/child_patch.rs": (
        "load_and_validate_graph_ownership",
        "child_patch_graph_node_missing",
        "child_patch_path_ownership_mismatch",
    ),
    "crates/clawd/src/agent_runtime_contract.rs": (
        "SubagentRoleDefinition",
        "load_subagent_role_definitions",
        "allowed_permission_profiles",
        "trusted_config_can_define_role_without_rust_branch",
    ),
    "crates/clawd/src/worker/runtime_support/stale_recovery.rs": (
        "stale_child_requeue_result_json",
        "child_claim_requeued",
        "lease_owner = NULL",
        "claimed_at = 0",
    ),
    "crates/clawd/src/repo/child_task_graph_tests.rs": (
        "graph_rejects_cycles_and_missing_dependencies",
        "disjoint_writers_are_ready_and_overlapping_writers_are_serialized",
        "restart_reconciliation_uses_task_rows_to_release_successor",
    ),
    "crates/clawd/src/repo/tasks_tests/child_graph.rs": (
        "dependency-blocked writer must not be claimed",
        "parent_failure_cancels_unfinished_graph_and_publishes_snapshot",
    ),
    "crates/clawd/src/skills/builtin_child_task_patch_tests.rs": (
        "persisted_path_ownership_blocks_out_of_scope_child_patch",
    ),
    "configs/agent_guard.toml": (
        "[agent.subagents.role_definitions.writer]",
        'default_permission_profile = "local_worktree"',
        'allowed_permission_profiles = ["local_worktree"]',
    ),
}

FORBIDDEN_TOKENS: dict[str, tuple[str, ...]] = {
    "crates/clawd/src/agent_runtime_contract.rs": (
        "enum SubagentRole",
        "parse_token(value",
    ),
    "crates/clawd/src/repo/child_task_graph.rs": (
        "user_text",
        "request_text",
        "Regex::",
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
    output: dict[str, str | None] = {}
    for rel_path in paths:
        try:
            output[rel_path] = (ROOT / rel_path).read_text(encoding="utf-8")
        except (FileNotFoundError, UnicodeDecodeError):
            output[rel_path] = None
    return output


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
    missing["crates/clawd/src/repo/tasks.rs"] = "mark_child_graph_node_running"
    assert any("graph_node.readiness" in item for item in scan_texts(missing))

    regressed = dict(good)
    regressed["crates/clawd/src/agent_runtime_contract.rs"] += "\nenum SubagentRole"
    assert any("forbidden_token" in item for item in scan_texts(regressed))
    print("CHILD_TASK_GRAPH_CONTRACT_SELF_TEST ok")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        run_self_test()
        return 0
    findings = scan_texts(read_repo_texts())
    if findings:
        print(f"CHILD_TASK_GRAPH_CONTRACT_CHECK findings={len(findings)}")
        for finding in findings:
            print(finding)
        return 1
    print("CHILD_TASK_GRAPH_CONTRACT_CHECK findings=0")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
