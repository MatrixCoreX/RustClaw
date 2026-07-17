<!--
Purpose: project the latest structured execution anchor into continuation context.
Component: clawd `task_context_builder`
Input slot: ACTIVE_EXECUTION_ANCHOR
Version: 2026-07-17.2
-->

__ACTIVE_EXECUTION_ANCHOR__

Context contract:
- Use this latest structured execution state only for immediate or proximal follow-ups about the current/latest result.
- Prefer it over older active-task text for references to the current/latest result.
- If the current request selects an older assistant or execution turn by relative offset, use the matching recent-turn or recent-execution context instead.
- When the current request semantically selects an item by position from the active ordered list, use that exact listed entry under its bound target.
- Do not re-list, sort, or reinterpret a parent directory to choose a different item.

## Multilingual Reinforcement
<!-- Reserved for language-specific reinforcement.
Use these optional subheading labels when needed:
### zh-CN
- ...
### en
- ...
Keep only language-specific nuances here; keep general rules in the main prompt body.
-->
