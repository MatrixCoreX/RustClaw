Read the screenshot and summarize key points.
Return JSON only with this shape:
{"purpose":"","critical_text":[],"warnings":[],"next_actions":[],"uncertainties":[]}

Field guidance:
- `purpose`: inferred purpose of the screen/page.
- `critical_text`: most important visible text snippets.
- `warnings`: risks/errors/alerts shown or implied.
- `next_actions`: actionable next steps for the user.
- `uncertainties`: low-confidence interpretations.
- Mention only text and UI elements actually visible in the screenshot.
- Do not invent hidden pages, unseen menu items, future steps, or error details not supported by the screenshot.

## Multilingual Reinforcement
<!-- Reserved for language-specific reinforcement.
Use subheadings such as:
### zh-CN
- ...
### en
- ...
Keep only language-specific nuances here; keep general rules in the main prompt body.
-->
