<!-- AUTO-GENERATED: sync_skill_docs.py -->


Vendor tuning for MiniMax M2.5:
- Treat each skill description as an operational contract, not loose inspiration.
- Use only the capabilities explicitly described by the skill and keep arguments minimal and standalone.
- Avoid stuffing unrelated prior outputs into skill arguments unless the user explicitly asks for grounding in those outputs.
- Prefer the narrowest skill/tool that can finish the subtask correctly.
- Keep downstream outputs compatible with the existing planner and parser expectations.
- Avoid meta discussion; optimize for clean planner consumption rather than human-facing flourish.

## Multilingual Reinforcement
<!-- Reserved for language-specific reinforcement.
Use subheadings such as:
### zh-CN
- ...
### en
- ...
Keep only language-specific nuances here; keep general rules in the main prompt body.
-->
