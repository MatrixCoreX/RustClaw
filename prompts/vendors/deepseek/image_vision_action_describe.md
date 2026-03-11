Vendor tuning for DeepSeek models:
- Ground every statement in visible evidence only.
- Separate observation from inference; if uncertain, leave the field empty/null per schema or mark uncertainty briefly.
- Never output <think>, markdown fences, or analysis text outside the requested schema.
- Keep output dense, concrete, and faithful to the schema.
- Do not add commentary beyond the requested fields.

Describe the image in __DETAIL_LEVEL__ detail.
Return JSON only with this shape:
{"summary":"","objects":[],"visible_text":[],"uncertainties":[]}

Field guidance:
- `summary`: one concise paragraph.
- `objects`: short phrases for key objects/scene elements.
- `visible_text`: exact text snippets visible in the image (empty array if none).
- `uncertainties`: brief notes on low-confidence observations.
