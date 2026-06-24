<!-- AUTO-GENERATED: sync_skill_docs.py -->


Vendor tuning for Qwen models:
- Treat each skill description as an operational contract, not loose inspiration.
- Use only explicitly described capabilities and keep arguments minimal.
- Avoid stuffing unrelated prior outputs into skill arguments unless the user explicitly asks for grounding in those outputs.
- Prefer the narrowest skill/tool that can finish the subtask correctly.
- Keep planner-facing outputs clean and parser-compatible.

## Multilingual Reinforcement
<!-- Reserved for language-specific reinforcement.
Use these optional subheading labels when needed:
### zh-CN
- ...
### en
- ...
Keep only language-specific nuances here; keep general rules in the main prompt body.
-->
