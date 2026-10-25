<!--
Purpose: project bounded recent execution evidence for short follow-ups.
Component: clawd `task_context_builder`
Input slot: RECENT_EXECUTION_CONTEXT
Version: 2026-10-22.1
-->

### RECENT_EXECUTION_CONTEXT

Context contract:
- Prior tasks provide supporting evidence for follow-up interpretation, never current execution or verification evidence.
- `prior_task_background_only` is background only. `prior_attempt_semantic_delta` / `PRIOR_TASK_ATTEMPT_ANCHOR` preserves failed/canceled intent, not permission to retry.
- For genuine follow-ups, apply the current request as a delta: keep unchanged constraints, apply changes, inspect partial effects. Bounded `prior_args` and observations may be incomplete.
- Never infer continuation from phrases/shared nouns or inherit incompatible frames, credentials, confirmations, authorization, attachments, or private artifact locators.
- Reuse only one unambiguous, type-compatible target. Newer concrete targets override older anchors; artifact-type nouns alone do not bind targets.
- Execute private paths/artifacts only through the current task's authorized attachment or canonical binding. Clarify remaining ambiguity.

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
