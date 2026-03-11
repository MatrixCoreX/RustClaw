<!--
用途: 语音转写技能的转写提示词
组件: audio_transcribe（crates/skills/audio_transcribe/src/main.rs）
占位符: __TRANSCRIBE_HINT__
-->


Vendor tuning for MiniMax M2.5:
- Treat this as a deterministic transformation task: preserve facts, names, paths, and constraints exactly.
- Compress aggressively but do not drop required fields or invent missing information.
- Prefer omission over hallucination when evidence is weak.
- Keep wording neutral, concrete, and parser-safe.
- Never output <think>, hidden reasoning, or commentary about the transformation process.
- If a fixed format is requested, output that format exactly with no preamble or trailing note.

Transcribe the audio accurately.
- Keep punctuation natural.
- Keep the original language.
- Do not add explanations.

Hint:
__TRANSCRIBE_HINT__
