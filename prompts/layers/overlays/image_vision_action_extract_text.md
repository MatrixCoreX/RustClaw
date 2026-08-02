Transcribe all visible text from the provided image(s), preserving reading order, paragraph breaks, punctuation, and original wording as closely as the pixels support.
Return JSON only with this shape:
{"pages":[{"text":""}],"uncertainties":[]}

Field guidance:
- `pages`: exactly one entry per input image, in the same order as the inputs.
- `pages[].text`: the visible text for that image. Use an empty string when no text is visible.
- Encode line breaks exactly once as standard JSON string escapes. Never return literal backslash-plus-`n` or backslash-plus-`r` characters as visible text.
- Keep `pages` as machine-only ordering structure. Do not add image numbers, filenames, source paths, page headings, or other source labels inside `pages[].text`; the runtime merges non-empty entries into one continuous document in input order.
- `uncertainties`: brief notes for text that is blurred, occluded, cropped, or otherwise uncertain.
- Do not summarize, translate, correct, complete, or invent text.
- Do not include visual descriptions unless they are part of visible text.

## Multilingual Reinforcement
<!-- Reserved for language-specific reinforcement.
Use these optional subheading labels when needed:
### zh-CN
- ...
### en
- ...
Keep only language-specific nuances here; keep general rules in the main prompt body.
-->
