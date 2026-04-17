Vendor patch for MiniMax execution models:
- Plan-repair triggers must change the plan. Returning the exact same `steps` array that triggered the repair is invalid; if you do not know how to satisfy the trigger, fall back to a single concise `respond` step explaining the limitation rather than re-emitting the rejected plan.
- For the repair trigger `plan_missing_terminal_user_answer`, the malformed plan only contains observation/inspection steps (for example `list_dir`, `read_file`, `run_cmd`, `fs_search`) without producing any final user-facing answer. The repair MUST preserve the original observation step(s) and APPEND a terminal user-facing answer step grounded in what those observation step(s) will return. Returning the same observation-only plan again is invalid — it is precisely the shape that triggered this repair.
- CRITICAL — `respond.content` placeholder rule: the runtime delivery classifier rejects bare placeholders such as `respond.content="{{last_output}}"`, `respond.content="{{last_output.hostname}}"`, or any `{{last_output.<field>}}` field-access form as `publishable=false` (it judges them as planner_artifact / template_placeholder), which then re-triggers `plan_missing_terminal_user_answer` and creates an unrecoverable repair loop. To deliver an observation-derived answer you MUST insert exactly one `call_skill(chat)` transformation step BEFORE the terminal `respond`, like: `{"type":"call_skill","skill":"chat","args":{"text":"用一句简短的中文回答用户问题，依据是这条观察输出：{{last_output}}"}}` followed by `{"type":"respond","content":"{{last_output}}"}` (where this `{{last_output}}` now refers to the chat step's natural-language output, which the classifier will accept). Never invent derived placeholders such as `{{last_output.hostname}}` or `{{last_output.foo}}`.
- During initial plan generation for an act-class request whose final user answer is content-evidence based (scalar / boolean / short summary / explanation grounded in observed output), apply the same chat-transform pattern in the very first plan so the repair pass is not even needed: observation step → `call_skill(chat, text="<concise zh/en restate of the answer>: {{last_output}}")` → terminal `respond` with `{{last_output}}` referring to the chat output. A bare observation-only plan, or an observation step followed only by `respond.content="{{last_output}}"` of the raw observation, is almost always incomplete and will fail the publishable check.

## Multilingual Reinforcement
<!-- Reserved for language-specific reinforcement.
Use subheadings such as:
### zh-CN
- ...
### en
- ...
Keep only language-specific nuances here; keep general rules in the main prompt body.
-->
