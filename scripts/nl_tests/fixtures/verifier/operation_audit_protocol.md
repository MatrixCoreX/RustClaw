This output contract extends the six existing verdict fields with `operation_checks`.
Return that array first, followed by the six verdict fields. This is a concise
evidence audit, not private reasoning or a plan for new work.

Create one check for every operation explicitly requested by the user. Each check
has exactly these fields:
- `requested_operation`: a brief description of the requested operation.
- `evidence_step_ids`: IDs of actual successful matching dispatches, or [] when
  no matching operation was executed. Do not cite a writer, metadata observation,
  plan, candidate claim or equivalent result as proof of a different method.
- `method_observed`: true only when that requested method actually ran.
- `result_observed`: true only when its requested result is supported.

Do not add checks for summarizing, rewriting, translating, formatting, or
presenting already-fetched or already-transcribed results. Those belong in the
verdict fields. An outcome-only check with empty `evidence_step_ids` is not an
execution audit. If such a row cites existing successful result steps, keep
`required_dispatches=[]` and `method_observed=false`; the host checks those
result steps only and does not treat `method_observed` as a new dispatch.

Audit the requested procedure before considering the candidate's labels. When a
request specifies a method, a logically equivalent value does not substitute for
performing it: metadata alone is not a content read even for an empty file.
Do not infer a missing operation from the candidate's headings. For outcome-only
requests, do not add an unrequested method requirement.

`pass` must be false if any check has `method_observed=false` or
`result_observed=false`. Use `requested_result` for a missing required operation.
Preserve completed effects and refer only to existing evidence IDs. A verified
provider/permission blocker can explain uncompleted work but does not make a
missing operation observed. All other existing safety and answer-quality rules
remain in force.
