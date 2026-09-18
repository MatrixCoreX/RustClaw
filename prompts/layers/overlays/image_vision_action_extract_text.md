Transcribe all visible text from the provided image(s), preserving reading order, paragraph breaks, punctuation, and original wording as closely as the pixels support.
Return JSON only with this shape:
{"pages":[{"text":""}],"uncertainties":[]}

Field guidance:
- `pages`: exactly one entry per input image, in the same order as the inputs. Never merge, omit, or duplicate an input image.
- `pages[].text`: the visible text for that image. Use an empty string when no text is visible.
- Encode line breaks exactly once as standard JSON string escapes. Never return literal backslash-plus-`n` or backslash-plus-`r` characters as visible text.
- Keep `pages` as machine-only ordering structure. Do not add image numbers, filenames, source paths, page headings, or other source labels inside `pages[].text`; the runtime merges non-empty entries into one continuous document in input order.
- Preserve every line-start marker that is visibly present in the image, including sequence numbers, numbered-list punctuation, bullets, middle dots, and other list symbols. Preserve its glyph and ordering as closely as the pixels support.
- Never add a line-start number, bullet, middle dot, Markdown marker, or other list prefix when that marker is not visibly present in the image. A visually unmarked line must remain unmarked.
- `uncertainties`: brief notes for text that is blurred, occluded, cropped, or otherwise uncertain.
- Do not summarize, translate, correct, complete, or invent text.
- Do not include visual descriptions unless they are part of visible text.

Coverage:
- Scan the full frame: titles, body copy, captions, UI chrome, stickers, watermarks, badges, price tags, and overlay text on photos.
- Do not skip overlay, sticker, caption, or watermark text merely because it looks decorative or repeated.
- Read small, faint, outlined, stylized, or low-contrast text as far as the pixels support. Prefer `uncertainties` over dropping a visible line.
- Keep mixed-language text, Traditional vs Simplified glyphs, kana, hangul, emoji, hashtags, @mentions, URLs, and fullwidth punctuation as shown. Do not convert scripts.

Reading order:
- Default: top-to-bottom, then left-to-right.
- For vertical CJK columns, read each column top-to-bottom and columns right-to-left.
- For multi-column layouts or stacked cards, finish one visual region before the next.

Accuracy:
- Similar-looking characters (0/O, 1/l/I, and close CJK glyphs) must follow the pixels, not a guessed word.

## Multilingual Reinforcement
<!-- Reserved for language-specific reinforcement.
Use these optional subheading labels when needed:
### zh-CN
- ...
### en
- ...
Keep only language-specific nuances here; keep general rules in the main prompt body.
-->
