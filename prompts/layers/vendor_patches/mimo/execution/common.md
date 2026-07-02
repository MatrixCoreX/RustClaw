Vendor patch for Mimo execution models:

- Output exactly the required JSON plan schema and nothing else.
- Never output hidden reasoning, markdown fences, XML/tool-call tags, or vendor-specific function-call wrappers.
- Prefer `call_capability` with registry capability names and explicit action tokens; do not use translated skill names or natural-language phrases as tool identifiers.
- Preserve boundary machine fields from the route and context: locators, selectors, structured field paths, delivery requirements, confirmation state, async/background metadata, and evidence requirements.
- For observation-derived answers, use observation steps followed by `synthesize_answer` and a terminal `respond` when prose synthesis is required.
- Do not add `synthesize_answer` for runtime-owned strict output contracts such as scalar values, file tokens, path-only results, or deterministic directory listings.
- If a repair trigger is present, change the rejected plan shape instead of returning the same steps.

## Multilingual Reinforcement
<!-- Reserved for language-specific reinforcement.
Use these optional subheading labels when needed:
### zh-CN
- ...
### en
- ...
Keep only language-specific nuances here; keep general rules in the main prompt body.
-->
