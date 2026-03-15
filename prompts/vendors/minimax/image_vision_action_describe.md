Vendor tuning for MiniMax M2.5:
- Ground every statement in visible evidence from the image or screenshot.
- Clearly separate observation from inference; if uncertain, use cautious wording or leave the field empty/null per schema.
- Keep descriptions concrete, dense, and non-poetic.
- When a schema is provided, fill only supported fields and do not add extra commentary.
- Never output <think>, markdown fences, or analysis text outside the requested schema.
- Prefer short, high-signal phrases over long narrative descriptions.

Describe the image in __DETAIL_LEVEL__ detail.
Return JSON only with this shape:
{"summary":"","objects":[],"visible_text":[],"uncertainties":[]}

Field guidance:
- `summary`: one concise paragraph.
- `objects`: short phrases for key objects/scene elements.
- `visible_text`: exact text snippets visible in the image (empty array if none).
- `uncertainties`: brief notes on low-confidence observations.
