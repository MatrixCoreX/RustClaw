<!--
Purpose: classify a candidate text for (1) meta-instruction vs user-facing content, (2) publishable vs filler.
Component: clawd (`crates/clawd/src/semantic_judge.rs`) `DELIVERY_TEXT_CLASSIFIER_PROMPT_TEMPLATE` (finalize-tier judge, §3.4)
Version: 2026-04-17.1
-->

You classify a candidate text for two purposes at once:
1) whether it is a meta-instruction fragment rather than user-facing final content
2) whether it is suitable to be shown directly to users as meaningful final-facing content

Return exactly one JSON object:
{"is_meta_instruction":true|false,"meta_reason":"...","meta_confidence":0.0,"publishable":true|false,"publishable_reason":"...","publishable_confidence":0.0}

Input text:
__TEXT__

Decision policy:
1) `is_meta_instruction=true` when the text is primarily process guidance, execution placeholder text, or reopening target/permission collection instead of directly answering the user.
2) `is_meta_instruction=false` when the text is substantive user-facing content, a concrete result, grounded not-found outcome, file token, or explicit completion result.
3) `publishable=true` when the text carries meaningful user-facing information: concrete result, explanation, extracted value(s), structured output, file token, or actionable outcome.
4) `publishable=false` when the text is mostly trivial acknowledgement/status filler, planner/internal artifact, or non-informative completion noise.
5) Judge by semantics and communicative role, not by fixed keyword matching.
6) Be conservative:
   - if uncertain about `is_meta_instruction`, prefer `false`
   - if uncertain about `publishable`, prefer `true`
7) Keep reasons short and stable.
   - `meta_reason`: examples `process_guidance_fragment`, `user_facing_result`, `delivery_token`, `ambiguous_keep`
   - `publishable_reason`: examples `meaningful_result`, `trivial_ack`, `planner_artifact`, `possibly_useful_keep`

Consistency guidance:
- A text may be both `is_meta_instruction=false` and `publishable=true` for normal final answers.
- A text may be `is_meta_instruction=true` and `publishable=false` for placeholders like “please read X and summarize it”.
- A text may be `is_meta_instruction=false` and `publishable=false` for very low-information filler.
- Delivery tokens are user-facing delivery artifacts; classify them as `is_meta_instruction=false` and `publishable=true` unless surrounding text clearly proves otherwise.

## Multilingual Reinforcement
### zh-CN
- Chinese process fragments that announce future work are meta rather than final user-facing content.
- Chinese imperative restatements are meta placeholders when they tell the assistant what to do instead of giving the requested answer.
- Chinese reopeners are meta-like when they reopen target/permission collection instead of directly answering the task.
- Chinese short but concrete results may still be valid user-facing publishable content.
- Judge Chinese text by communicative role, not by whether it sounds formal or system-like.
