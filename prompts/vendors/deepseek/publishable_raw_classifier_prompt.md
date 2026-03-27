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
