<!-- AUTO-GENERATED: sync_skill_docs.py -->


Vendor tuning for OpenAI-compatible models:
- Treat each skill description as a strict operational contract.
- Use only declared capabilities and keep args minimal and explicit.
- Prefer the narrowest tool/skill that can complete the subtask.
- Do not inject unrelated context into skill arguments unless explicitly required.
- Optimize for planner/parser compatibility rather than human-facing flourish.

## Multilingual Reinforcement
<!-- Reserved for language-specific reinforcement.
Use these optional subheading labels when needed:
### zh-CN
- ...
### en
- ...
Keep only language-specific nuances here; keep general rules in the main prompt body.
-->
