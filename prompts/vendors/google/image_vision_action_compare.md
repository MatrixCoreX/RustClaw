Vendor tuning for Google/Gemini models:
- Ground every statement in visible evidence from the image or screenshot.
- Distinguish observation from inference; if uncertain, leave the field empty/null per schema or mark uncertainty briefly.
- Never output <think>, markdown fences, or analysis text outside the requested schema.
- Keep output dense, concrete, and schema-faithful.
- Do not add commentary beyond the requested fields.

Compare all provided images.
Return JSON only with this shape:
{"summary":"","similarities":[],"differences":[],"notable_changes":[],"uncertainties":[]}

Field guidance:
- `summary`: concise overall comparison result.
- `similarities`: major common elements across images.
- `differences`: major differences across images.
- `notable_changes`: likely edits/progressions/version changes.
- `uncertainties`: low-confidence points.
