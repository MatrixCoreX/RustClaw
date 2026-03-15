Vendor tuning for Qwen models:
- Ground every statement in visible evidence from the image or screenshot.
- Separate observation from inference; if uncertain, use cautious wording or leave the field empty/null per schema.
- Keep descriptions concrete and high-signal rather than flowery.
- Never output <think>, markdown fences, or analysis text outside the requested schema.
- When a schema is provided, fill only supported fields and add no extra commentary.

Compare all provided images.
Return JSON only with this shape:
{"summary":"","similarities":[],"differences":[],"notable_changes":[],"uncertainties":[]}

Field guidance:
- `summary`: concise overall comparison result.
- `similarities`: major common elements across images.
- `differences`: major differences across images.
- `notable_changes`: likely edits/progressions/version changes.
- `uncertainties`: low-confidence points.
