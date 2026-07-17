<!--
Purpose: project trusted runtime path facts into agent context.
Component: clawd `task_context_builder`
Placeholders: __RUNTIME_CONTEXT__
Version: 2026-07-17.1
-->

__RUNTIME_CONTEXT__

Context contract:
- Treat these values as current-turn runtime facts.
- For local filesystem operations, `workspace_root` is the default workspace boundary.
- `current_process_cwd` is the working directory of the `clawd` process and does not expand the workspace boundary.

## Multilingual Reinforcement
<!-- Reserved for language-specific reinforcement.
Use these optional subheading labels when needed:
### zh-CN
- ...
### en
- ...
Keep only language-specific nuances here; keep general rules in the main prompt body.
-->
