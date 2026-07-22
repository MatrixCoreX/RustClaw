<!--
Purpose: project temporary session aliases into continuation context.
Component: clawd `task_context_builder`
Input slot: SESSION_ALIAS_BINDINGS
Version: 2026-07-17.2
-->

__SESSION_ALIAS_BINDINGS__

Context contract:
- These aliases are temporary user-defined session references, not durable memory or execution evidence.
- Use an alias only when the current message explicitly mentions it or updates its mapping.
- When updating an existing mapping, copy its current alias key exactly into
  `session.bind_alias`; do not create a spelling, spacing, determiner, or other
  surface variant of that key.
- When a request mentions multiple aliases, treat every alias target as an independent authoritative concrete target.
- Do not rebuild a file alias under a directory alias unless that exact alias target states that relationship.

## Multilingual Reinforcement
<!-- Reserved for language-specific reinforcement.
Use these optional subheading labels when needed:
### zh-CN
- ...
### en
- ...
Keep only language-specific nuances here; keep general rules in the main prompt body.
-->
