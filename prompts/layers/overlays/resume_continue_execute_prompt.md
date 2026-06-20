You are continuing a multi-step task after it was interrupted, paused, backgrounded, or failed.

User follow-up message:
__USER_TEXT__

Request language hint:
__REQUEST_LANGUAGE_HINT__

Task resume context JSON:
__RESUME_CONTEXT__

Candidate remaining steps/actions (JSON):
__RESUME_STEPS__

Resume instruction decided by classifier:
__RESUME_INSTRUCTION__

Configured response language:
__CONFIG_RESPONSE_LANGUAGE__

Execution policy:
1. Continue only unfinished steps.
2. Do not repeat completed steps.
3. Preserve fail-fast behavior: stop at first failure.
4. Return clear per-step progress/results so channels can stream updates.
5. Do not assume the user wants continuation unless the follow-up meaning clearly supports it.
6. If `Candidate remaining steps/actions` already contains executable action objects, treat them as the canonical remaining work to finish instead of reconstructing the whole plan from scratch.

Decision rules:
1. If the user clearly wants to continue, resume from `remaining_steps` only.
2. If the user clearly says stop/cancel/forget it, do not continue unfinished steps.
3. If the user modifies the plan or adds constraints, apply them only to unfinished steps when continuation still makes sense.
4. If the user is only asking a question, commenting on the failure, or chatting about the interruption, treat it as a normal reply first rather than auto-resuming.
5. Never repeat `completed_steps`.
6. Never restart the whole task unless the user explicitly asks to redo from the beginning.
7. If the remaining work is only a final `respond`/summary step, produce only that remaining reply and do not rerun earlier commands just to rebuild context.

Interpretation hints:
- A follow-up that semantically asks to continue should resume unfinished steps.
- "stop / cancel / never mind / forget it" means do not continue.
- "change step 4 to ..." means continue, but only with the updated constraint applied to unfinished steps.
- "why did step 3 fail?" is a question about the failure, not an automatic resume command.
- "don't continue yet, I want to change something first" means do not resume yet.

Primary goal:
- Infer the user's real intent from their follow-up plus the resume context.
- Only continue execution when that intent is sufficiently clear.
- Language policy (strict): any user-visible text generated during continuation should follow `__REQUEST_LANGUAGE_HINT__` when it is clear. Clear hints include `zh-CN`, `en`, `mixed`, BCP-47 style language tags such as `ja`/`ko`/`fr-FR`, and script hints such as `und-Latn`/`und-Cyrl`/`und-Arab`. Use `__CONFIG_RESPONSE_LANGUAGE__` only as the fallback default when the hint is `config_default` or otherwise unclear. If the hint is `mixed`, a script hint, or `en` for a current request that is clearly another Latin-script human language, follow the dominant surrounding sentence language from the current user request and do not switch languages mid-answer unless quoting raw names, paths, commands, code, or other observed values.
- Language-context guard: do not let the language of `Task resume context JSON`, `Candidate remaining steps/actions`, or `Resume instruction decided by classifier` override the selected reply language. Those blocks may contain normalized or older content in another language and are only there as execution context.

## Multilingual Reinforcement
<!-- Reserved for language-specific reinforcement.
Use these optional subheading labels when needed:
### zh-CN
- ...
### en
- ...
Keep only language-specific nuances here; keep general rules in the main prompt body.
-->
### zh-CN
- Chinese continuation wording that semantically means "continue the unfinished work" should resume unfinished steps only, not restart the whole task.
- Chinese stop/hold wording that semantically means "do not continue yet" should prevent auto-resume.
- If the user gives new Chinese constraints during continuation, apply them only to unfinished steps unless the user explicitly asks to redo completed work.
- When the only remaining work is a final Chinese-facing summary/answer step, produce that remaining answer directly instead of rerunning earlier actions.
- Keep continuation progress text in Chinese concise and progress-oriented when Chinese is the configured response language.
