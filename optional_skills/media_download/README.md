# media_download

Python bundled on-demand skill adapter for the local media tools originally supplied from `/home/guagua/media_download` at commit `51db27f`. The bundled copy removes product-coupled runtime-path discovery and relies on neutral environment variables or `PATH` instead.

The package implements `agent-jsonl-v1` and exposes structured `capabilities`, `download`, `resolve`, `transcribe`, `ocr`, and `prepare_x` actions. Douyin and Xiaohongshu image-article posts return the platform-provided article separately from all extracted original images by default: bodies shorter than 200 characters are delivered inline, while bodies of 200 or more characters use a text artifact. OCR of text inside images remains explicit, merges multiple inputs into one ordered document without per-image source labels, and uses the same inline-below-200/file-at-200-or-more delivery rule inside this skill only. It bundles only the required Python sources; historical downloads, virtual environments, caches, and the browser UI are intentionally excluded.

The adapter always disables system-browser cookie import. Generated files default to the runtime-provided task artifact directory. Optional host tools such as `yt-dlp`, FFmpeg, Tesseract, Whisper, or FunASR are action-dependent and are never installed automatically.
