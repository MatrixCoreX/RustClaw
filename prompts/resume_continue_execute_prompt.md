You are continuing an interrupted multi-step task after a prior failure.

User follow-up message:
__USER_TEXT__

Interrupted task context JSON:
__RESUME_CONTEXT__

Candidate remaining steps (JSON):
__RESUME_STEPS__

Resume instruction decided by classifier:
__RESUME_INSTRUCTION__

Execution policy:
1. Continue only unfinished steps.
2. Do not repeat completed steps.
3. Preserve fail-fast behavior: stop at first failure.
4. Return clear per-step progress/results so channels can stream updates.
