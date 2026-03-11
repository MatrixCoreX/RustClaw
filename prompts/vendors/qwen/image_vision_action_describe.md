Vendor tuning for Qwen models:
- Ground every statement in visible evidence from the image or screenshot.
- Separate observation from inference; if uncertain, use cautious wording or leave the field empty/null per schema.
- Keep descriptions concrete and high-signal rather than flowery.
- Never output <think>, markdown fences, or analysis text outside the requested schema.
- When a schema is provided, fill only supported fields and add no extra commentary.

Describe the image in __DETAIL_LEVEL__ detail.
Return JSON only with this shape:
{"summary":"","objects":[],"visible_text":[],"uncertainties":[]}

Field guidance:
- `summary`: one concise paragraph.
- `objects`: short phrases for key objects/scene elements.
- `visible_text`: exact text snippets visible in the image (empty array if none).
- `uncertainties`: brief notes on low-confidence observations.
