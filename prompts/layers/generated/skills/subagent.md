<!-- AUTO-GENERATED: sync_skill_docs.py -->
<!-- Source: crates/clawd/src/skills/interfaces/subagent/INTERFACE.md -->
# Skill: subagent

- `agent.subagent`: one bounded read-only child loop.
- `agent.subagent_batch`: two or more independent read-only child loops.
- `agent.subagent_persistent`: durable/resumable child work or a child DAG.
- Every child requires a trusted `role`, scoped `objective`, non-empty
  `context_refs`, and non-empty `allowed_capabilities`.
- Findings are child-loop output. Never supply findings, permissions,
  isolation, model policy, tool policy, or runtime policy from the parent.
- Accept completion only from the closed structured child-result contract.
  Persistent writers use isolated worktrees and require parent patch review.

## Multilingual Reinforcement
<!-- Reserved for language-specific reinforcement.
Use these optional subheading labels when needed:
### zh-CN
- ...
### en
- ...
Keep only language-specific nuances here; keep general rules in the main prompt body.
-->
