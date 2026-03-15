Vendor tuning for MiniMax M2.5:
- Ground every statement in visible evidence from the image or screenshot.
- Clearly separate observation from inference; if uncertain, use cautious wording or leave the field empty/null per schema.
- Keep descriptions concrete, dense, and non-poetic.
- When a schema is provided, fill only supported fields and do not add extra commentary.
- Never output <think>, markdown fences, or analysis text outside the requested schema.
- Prefer short, high-signal phrases over long narrative descriptions.

Compare all provided images.
Return JSON only with this shape:
{"summary":"","similarities":[],"differences":[],"notable_changes":[],"uncertainties":[]}

Field guidance:
- `summary`: concise overall comparison result.
- `similarities`: major common elements across images.
- `differences`: major differences across images.
- `notable_changes`: likely edits/progressions/version changes.
- `uncertainties`: low-confidence points.
