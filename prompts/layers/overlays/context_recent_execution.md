<!--
Purpose: project bounded recent execution evidence for short follow-ups.
Component: clawd `task_context_builder`
Input slot: RECENT_EXECUTION_CONTEXT
Version: 2026-07-17.2
-->

### RECENT_EXECUTION_CONTEXT

Context contract:
- Use this block only as supporting evidence for genuinely short follow-up requests.
- Reuse a previous target only when the current request or recent context already binds exactly one concrete target of the correct type.
- Do not let this block override a required clarification.
- Do not treat an artifact-type noun alone as a concrete target.

__RECENT_EXECUTION_CONTEXT__

## Multilingual Reinforcement
<!-- Reserved for language-specific reinforcement.
Use these optional subheading labels when needed:
### zh-CN
- ...
### en
- ...
Keep only language-specific nuances here; keep general rules in the main prompt body.
-->
