<!-- AUTO-GENERATED: sync_skill_docs.py -->


Vendor tuning for Claude models:
- Treat each skill description as a binding operational contract.
- Use only declared capabilities and keep args minimal and explicit.
- Prefer the narrowest tool/skill that can complete the subtask correctly.
- Do not inject unrelated context into skill args unless explicitly required.
- Optimize for precise planner/parser compatibility.

## Multilingual Reinforcement
<!-- Reserved for language-specific reinforcement.
Use these optional subheading labels when needed:
### zh-CN
- ...
### en
- ...
Keep only language-specific nuances here; keep general rules in the main prompt body.
-->
