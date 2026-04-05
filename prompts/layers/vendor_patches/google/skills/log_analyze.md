<!-- AUTO-GENERATED: sync_skill_docs.py -->


Vendor tuning for Google/Gemini models:
- Treat each skill description as a binding contract for planner output.
- Use only declared capabilities and keep args minimal and standalone.
- Prefer the narrowest tool/skill that can complete the subtask.
- Avoid injecting unrelated prior context unless the user explicitly asks for grounding in it.
- Optimize for deterministic planner consumption.

## Multilingual Reinforcement
<!-- Reserved for language-specific reinforcement.
Use subheadings such as:
### zh-CN
- ...
### en
- ...
Keep only language-specific nuances here; keep general rules in the main prompt body.
-->
