## workspace_patch - runtime patch adapter

This hidden adapter backs planner-owned `fs_basic` actions: `workspace.preview_replace_text`, `workspace.replace_text`, `workspace.preview_edit_text`, `workspace.edit_text`, `workspace.apply_patch`, `workspace.diff`, and `workspace.revert_checkpoint`.

## Machine contract
- `preview_replace_text` defaults to one exact UTF-8 match and returns bounded diff/hash evidence without writing. `replace_all=true` explicitly selects every match; `expected_occurrences` may pin the exact count up to 10,000.
- `replace_text` accepts either top-level `old_text`/`new_text` or a non-empty `edits[]` array. Batch edits run sequentially in memory and commit once, so any failed edit leaves the file unchanged. Successful writes preserve coherent CRLF files and return one checkpoint.
- `apply_patch` validates a bounded unified diff with Git when the workspace is a Git worktree. A non-Git workspace uses the bounded pure-Rust unified-diff engine. Both verify exact context and optional `precondition_hashes`, snapshot every target, and return patch/checkpoint evidence.
- `diff` returns checkpoint evidence for every replacement/edit/patch path, or a bounded current Git diff when no checkpoint is supplied.
- `rewind` restores a checkpoint only when every target still has its recorded post-patch hash.
- Paths are workspace-relative. Parent traversal, runtime state paths, unsupported file types, and
  symbolic-link traversal are rejected.
- Errors expose stable `error_code` and `message_key` fields. Do not infer control state from prose.
- Replacement missing/ambiguous/stale error codes are exact-call observations; inspect current file state before replanning and never reinterpret them as natural language.

## Multilingual Reinforcement
<!-- Reserved for language-specific reinforcement.
Use these optional subheading labels when needed:
### zh-CN
- ...
### en
- ...
Keep only language-specific nuances here; keep general rules in the main prompt body.
-->
