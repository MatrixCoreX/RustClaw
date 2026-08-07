<!--
Purpose: project bounded recent execution evidence for short follow-ups.
Component: clawd `task_context_builder`
Input slot: RECENT_EXECUTION_CONTEXT
Version: 2026-07-17.2
-->

### RECENT_EXECUTION_CONTEXT

Context contract:
- Use this block only as supporting evidence for genuinely short follow-up requests.
- Every row in this block belongs to a prior task. Prior-task success, output, or discussion never satisfies the current task's execution or verification requirements.
- Reuse a previous target only when the current request or recent context already binds exactly one concrete target of the correct type.
- Prior-task private paths and artifacts are not executable inputs. Use only an attachment or canonical artifact binding supplied by the current task.
- A concrete target in a newer recent turn takes precedence over an older execution anchor. Never replace that newer target with the anchor's target.
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
