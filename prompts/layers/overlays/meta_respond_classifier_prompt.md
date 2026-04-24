<!--
Purpose: legacy meta-respond classifier prompt retained for audit/reference only.
Component: historical overlay no longer wired into the active `clawd` finalize path; live behavior is now covered by `delivery_text_classifier_prompt.md` in `crates/clawd/src/semantic_judge.rs`.
Version: 2026-04-20.1
Placeholders: __TEXT__
-->

> Legacy note: this prompt is no longer used by the active `clawd` runtime. Keep it only as historical/audit reference until the prompt inventory is pruned.


You classify whether a candidate `respond` text is a meta-instruction fragment (for planner/executor) rather than user-facing final content.

Return exactly one JSON object:
{"is_meta_instruction":true|false,"reason":"...","confidence":0.0}

Input text:
__TEXT__

Decision policy:
1) `is_meta_instruction=true` when the text is primarily process guidance about how to analyze prior output / what to consider / how to continue execution, and not a direct user-facing result.
1.1) `is_meta_instruction=true` when the text merely restates the task the assistant should perform next, especially imperative placeholders such as asking to read / inspect / compare / summarize content rather than actually giving the result.
1.2) Template-like wrappers that mainly embed runtime placeholders such as `{{last_output}}`, `{{s1.output}}`, or similar raw execution variables are usually meta scaffolding unless the surrounding text already forms a real final answer after substitution.
1.3) Clarification/confirmation reopeners are meta-like when they mainly ask for a path, filename, target, scope, or permission to execute instead of directly answering the task. This includes texts whose main communicative role is “please provide the path/target” or “should I execute now?” rather than a final result.
2) `is_meta_instruction=false` when the text is substantive user-facing content, actionable final answer, concrete file token (`FILE:` / `IMAGE_FILE:`), or explicit completion result.
3) Judge by semantics and communicative role, not by fixed keyword matching.
4) Be conservative: if uncertain, prefer `false` (do not suppress a potentially valid user-facing response).
5) `reason` should be short, e.g. `process_guidance_fragment`, `user_facing_result`, `delivery_token`, `ambiguous_keep`.

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
- Chinese process fragments such as `下一步我会`、`请稍等我先分析`、`我将继续执行` are often meta-instruction-like rather than final user-facing content.
- Chinese imperative restatements such as `请阅读 ... 并总结`、`请检查 ... 后告诉我结果`、`请比较 ...` are usually meta-instruction placeholders when they tell the assistant what to do instead of giving the requested answer.
- Chinese template wrappers like `基于上面结果总结如下：{{last_output}}` are often meta scaffolding when they merely paste raw execution output instead of presenting a transformed final answer.
- Chinese reopeners such as `请提供完整路径`、`请确认是否执行`、`我现在执行...吗` are meta-like when they reopen target/permission collection instead of directly answering the task.
- Chinese concise results such as `已发送`、`没找到`、`当前分支是 main`、`FILE:/path/to/file` may still be valid user-facing final content and should not be over-filtered as meta.
- Judge Chinese text by communicative role, not by whether it sounds formal or system-like.
