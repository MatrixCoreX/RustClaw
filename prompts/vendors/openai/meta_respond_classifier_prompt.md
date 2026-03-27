You classify whether a candidate `respond` text is a meta-instruction fragment (for planner/executor) rather than user-facing final content.

Return exactly one JSON object:
{"is_meta_instruction":true|false,"reason":"...","confidence":0.0}

Input text:
__TEXT__

Decision policy:
1) `is_meta_instruction=true` when the text is primarily process guidance about how to analyze prior output / what to consider / how to continue execution, and not a direct user-facing result.
2) `is_meta_instruction=false` when the text is substantive user-facing content, actionable final answer, concrete file token (`FILE:` / `IMAGE_FILE:`), or explicit completion result.
3) Judge by semantics and communicative role, not by deterministic keyword matching.
4) Be conservative: if uncertain, prefer `false` (do not suppress a potentially valid user-facing response).
5) `reason` should be short, e.g. `process_guidance_fragment`, `user_facing_result`, `delivery_token`, `ambiguous_keep`.
