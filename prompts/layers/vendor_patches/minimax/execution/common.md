Vendor patch for MiniMax execution models:
- Plan-repair triggers must change the plan. Returning the exact same `steps` array that triggered the repair is invalid; if you do not know how to satisfy the trigger, fall back to a single concise `respond` step explaining the limitation rather than re-emitting the rejected plan.
- For the repair trigger `plan_missing_terminal_user_answer`, the malformed plan only contains observation/inspection steps (for example `list_dir`, `read_file`, `run_cmd`, `fs_search`) without producing any final user-facing answer. The repair MUST preserve the original observation step(s) and APPEND exactly one terminal `respond` step that delivers the requested answer (boolean / scalar / short summary / explanation / file-token) grounded in what those observation step(s) will return. If the final answer needs to be derived from observed data, you may use `{{last_output}}` or insert one `call_skill(chat)` transformation step before the terminal `respond`. Returning the same observation-only plan again is invalid — it is precisely the shape that triggered this repair.
- During initial plan generation for an act-class request whose final user answer is content-evidence based (boolean / scalar / short summary / explanation grounded in observed output), prefer to include the terminal `respond` step in the very first plan so the repair pass is not even needed. A bare observation-only plan is almost always incomplete unless the runtime explicitly accepts observation-only finalize.

## Multilingual Reinforcement
<!-- Reserved for language-specific reinforcement.
Use subheadings such as:
### zh-CN
- ...
### en
- ...
Keep only language-specific nuances here; keep general rules in the main prompt body.
-->
