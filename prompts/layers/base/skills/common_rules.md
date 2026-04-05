Shared skill prompt contract:
- `prompts/layers/generated/skills/<name>.md` is the canonical prompt body generated from `INTERFACE.md`.
- Treat the generated default skill prompt as the main source of truth. Vendor-specific behavior should be expressed only as a thin patch when strictly necessary.
- Follow the declared interface strictly. If the request exceeds interface scope, prefer one concise clarification over guessing.

## Multilingual Reinforcement
<!-- Reserved for language-specific reinforcement.
Use subheadings such as:
### zh-CN
- ...
### en
- ...
Keep only language-specific nuances here; keep general rules in the main prompt body.
-->
