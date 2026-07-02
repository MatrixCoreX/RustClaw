Vendor tuning for Mimo models:

- Treat each skill description as an operational contract, not loose inspiration.
- Prefer registry capability names and explicit action tokens over translated skill names, colloquial user phrases, or inferred legacy semantic kinds.
- Use only the capabilities explicitly described by the skill and keep arguments minimal and standalone.
- Avoid stuffing unrelated prior outputs into skill arguments unless the user explicitly asks for grounding in those outputs.
- Keep downstream outputs compatible with the existing planner and parser expectations.
- Optimize for strict planner consumption instead of human-facing flourish.

## Multilingual Reinforcement
<!-- Reserved for language-specific reinforcement.
Use these optional subheading labels when needed:
### zh-CN
- ...
### en
- ...
Keep only language-specific nuances here; keep general rules in the main prompt body.
-->
