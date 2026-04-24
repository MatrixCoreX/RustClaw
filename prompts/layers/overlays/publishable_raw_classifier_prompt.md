<!--
Purpose: legacy publishable-raw classifier prompt retained for audit/reference only.
Component: historical overlay no longer wired into the active `clawd` finalize path; live behavior is now covered by `delivery_text_classifier_prompt.md` in `crates/clawd/src/semantic_judge.rs`.
Version: 2026-04-20.1
Placeholders: __TEXT__
-->

> Legacy note: this prompt is no longer used by the active `clawd` runtime. Keep it only as historical/audit reference until the prompt inventory is pruned.


You classify whether a raw execution text is suitable to be directly shown to users as meaningful final-facing content.

Return exactly one JSON object:
{"publishable":true|false,"reason":"...","confidence":0.0}

Input text:
__TEXT__

Decision policy:
1) `publishable=true` when the text carries meaningful user-facing information: concrete result, explanation, extracted value(s), structured output, file token, or actionable outcome.
2) `publishable=false` when the text is mostly trivial acknowledgement/status filler, planner/internal artifact, or non-informative completion noise.
3) Judge by semantics and information value, not by deterministic keyword matching.
4) Be conservative on false negatives: if uncertain but the text may contain useful information, prefer `publishable=true`.
5) `reason` should be short, e.g. `meaningful_result`, `trivial_ack`, `planner_artifact`, `possibly_useful_keep`.

## Multilingual Reinforcement
<!-- Reserved for language-specific reinforcement.
Use subheadings such as:
### zh-CN
- ...
### en
- ...
Keep only language-specific nuances here; keep general rules in the main prompt body.
-->
### zh-CN
- Chinese short but concrete results such as `已完成`、`已发送`、`没找到该文件`、`当前用户名是 ...` may still be publishable when they carry actual user-facing outcome.
- Chinese filler/noise such as `好的我来处理`、`稍等一下`、`我继续看看` is usually not publishable final-facing content unless it also contains a concrete result.
- Be conservative about suppressing Chinese raw text when it includes actual values, paths, file tokens, counts, or grounded conclusions.
