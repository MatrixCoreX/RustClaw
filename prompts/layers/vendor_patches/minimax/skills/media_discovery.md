<!-- AUTO-GENERATED: sync_skill_docs.py -->

MiniMax-specific `media_discovery` tuning:
- A finite requested count is one `run_once` with that `max_items_per_run`.
  Do not ask the user to choose a one-shot batch versus continuous collection,
  invent a 100-item ceiling, or emit a numbered menu for that choice.
- Start continuous collection only when the user explicitly asked for a
  durable/background worker: one `enable` with `confirm=true`, then
  `run_enabled_once`. In the same start request, never call `disable`, never
  repeat a successful `enable`, and never `respond` as if the start were still
  undecided.
- If the immediately previous assistant reply listed numbered mutually
  exclusive collection choices and the current user message is only that
  choice index, execute the selected workflow with already-bound platform,
  topics, and count. Do not clarify `ambiguous_user_intent`.
- On `task_plan_revision_conflict` or `collection_already_enabled`, read the
  current plan/state and continue; do not enable again from revision 0.
- User-visible `respond` / clarification content follows the request language.

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
- 用户已给出平台、关键词和条数时，直接按有限 `run_once` 执行，不要再问单次还是持续。
- 上一轮助手列出互斥编号选项后，用户只回复该编号时按已绑定参数执行，不要再用英文追问意图。
