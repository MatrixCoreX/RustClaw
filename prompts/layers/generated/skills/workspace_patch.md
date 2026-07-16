## workspace_patch - runtime patch adapter

This hidden runtime adapter applies planner-owned `fs_basic` patch actions. Planner calls should use
`workspace.apply_patch`, `workspace.diff`, or `workspace.revert_checkpoint` through `fs_basic`
instead of selecting this adapter directly.

## Machine contract
- `apply_patch` validates a bounded unified diff with Git, verifies exact context and optional
  `precondition_hashes`, snapshots every target, and returns patch/checkpoint evidence.
- `diff` returns a checkpoint patch or a bounded current Git diff as structured JSON.
- `rewind` restores a checkpoint only when every target still has its recorded post-patch hash.
- Paths are workspace-relative. Parent traversal, runtime state paths, unsupported file types, and
  symbolic-link traversal are rejected.
- Errors expose stable `error_code` and `message_key` fields. Do not infer control state from prose.

## Multilingual Reinforcement
<!-- Reserved for language-specific reinforcement.
Use these optional subheading labels when needed:
### zh-CN
- ...
### en
- ...
Keep only language-specific nuances here; keep general rules in the main prompt body.
-->
