## workspace_patch - runtime patch adapter

This hidden adapter backs planner-owned `fs_basic` actions: `workspace.preview_replace_text`, `workspace.replace_text`, `workspace.apply_patch`, `workspace.diff`, and `workspace.revert_checkpoint`.

## Machine contract
- `preview_replace_text` requires one exact UTF-8 match and returns bounded diff/hash evidence without writing.
- `replace_text` atomically applies that unique replacement, rejects stale/ambiguous state, preserves coherent CRLF files, and returns a checkpoint.
- `apply_patch` validates a bounded unified diff with Git, verifies exact context and optional
  `precondition_hashes`, snapshots every target, and returns patch/checkpoint evidence.
- `diff` returns a checkpoint patch or a bounded current Git diff as structured JSON.
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
