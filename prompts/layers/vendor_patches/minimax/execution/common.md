Vendor patch for MiniMax execution models:
- Plan-repair triggers must change the plan. Returning the exact same `steps` array that triggered the repair is invalid; if you do not know how to satisfy the trigger, fall back to a single concise `respond` step explaining the limitation rather than re-emitting the rejected plan.
- For the repair trigger `plan_missing_terminal_user_answer`, the malformed plan only contains observation/inspection steps (for example `list_dir`, `read_file`, `run_cmd`, `fs_search`) without producing any final user-facing answer. The repair MUST preserve the original observation step(s) and APPEND a terminal user-facing answer step grounded in what those observation step(s) will return. Returning the same observation-only plan again is invalid — it is precisely the shape that triggered this repair.
- CRITICAL — `respond.content` placeholder rule: the runtime delivery classifier rejects bare placeholders such as `respond.content="{{last_output}}"`, `respond.content="{{last_output.hostname}}"`, or any `{{last_output.<field>}}` field-access form as `publishable=false` when they still refer to raw planner artifacts or raw observation bodies. For observation-derived answers, prefer exactly one runtime-owned synthesis step BEFORE the terminal `respond`, like: `{"type":"synthesize_answer","evidence_refs":["last_output"]}` followed by `{"type":"respond","content":"{{last_output}}"}` (where this `{{last_output}}` now refers to the synthesized natural-language answer, which the classifier will accept). Never invent derived placeholders such as `{{last_output.hostname}}` or `{{last_output.foo}}`.
- During initial plan generation for an act-class request whose final user answer is content-evidence based (scalar / boolean / short summary / explanation grounded in observed output), apply the same synthesis pattern in the very first plan so the repair pass is not even needed: observation step → `synthesize_answer` → terminal `respond` with `{{last_output}}` referring to the synthesized answer. For genuine free-form chat/joke generation, use terminal `respond` directly; do not call a chat skill.

## Multilingual Reinforcement
<!-- Reserved for language-specific reinforcement.
Use subheadings such as:
### zh-CN
- ...
### en
- ...
Keep only language-specific nuances here; keep general rules in the main prompt body.
-->
