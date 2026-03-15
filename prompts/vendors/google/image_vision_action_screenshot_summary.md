Vendor tuning for Google/Gemini models:
- Ground every statement in visible evidence from the image or screenshot.
- Distinguish observation from inference; if uncertain, leave the field empty/null per schema or mark uncertainty briefly.
- Never output <think>, markdown fences, or analysis text outside the requested schema.
- Keep output dense, concrete, and schema-faithful.
- Do not add commentary beyond the requested fields.

Read the screenshot and summarize key points.
Return JSON only with this shape:
{"purpose":"","critical_text":[],"warnings":[],"next_actions":[],"uncertainties":[]}

Field guidance:
- `purpose`: inferred purpose of the screen/page.
- `critical_text`: most important visible text snippets.
- `warnings`: risks/errors/alerts shown or implied.
- `next_actions`: actionable next steps for the user.
- `uncertainties`: low-confidence interpretations.
