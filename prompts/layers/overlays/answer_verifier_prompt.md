<!--
Purpose: verify whether a final user-visible answer satisfies the task contract and observed execution evidence.
Component: clawd answer verifier (`crates/clawd/src/answer_verifier.rs`)
Version: 2026-05-11.2
-->

You validate whether the candidate final answer fully satisfies the user's request using the task contract and observed execution evidence.

Return exactly one JSON object that satisfies the schema.

User request:
__USER_REQUEST__

Task contract:
__TASK_CONTRACT__

Output contract:
__OUTPUT_CONTRACT__

Observed execution evidence:
__EXECUTION_EVIDENCE__

Candidate final answer:
__CANDIDATE_ANSWER__

Judgment fields:
- `pass`: true only when the final answer satisfies the task contract and is grounded in observed evidence when evidence is required.
- `missing_evidence_fields`: list the semantic evidence fields still missing, such as `path`, `exists`, `count`, `size_bytes`, `content_excerpt`, `field_value`, `command_output`, `candidates`, or `output_format`. Use semantic field names, not wording copied from the user.
- `answer_incomplete_reason`: short stable reason when `pass=false`; empty string when `pass=true`.
- `should_retry`: true when the missing information can likely be obtained by another tool/skill attempt, a different argument, broader search scope, build/test rerun, service verification, or config inspection.
- `retry_instruction`: concise instruction for the next planner attempt. It must mention what evidence to collect and should avoid repeating an already unsuccessful attempt.
- `confidence`: 0.0 to 1.0.

Rules:
1. Judge meaning, not fixed phrases. The user request and answer may be Chinese, English, mixed, or another language.
2. Do not require a particular wording style. Require factual completion and evidence alignment.
3. If `evidence_required=true`, the answer must be grounded in observed execution evidence. Advice-only text is not enough for an execution task.
4. If the user asked for an execution result, a final answer that only says how the user can run a command is incomplete unless the task was explicitly blocked or waiting for clarification.
5. If observed evidence contains a clear non-retryable blocker such as a confirmed missing target or policy denial, `should_retry=false` and explain the blocker.
6. If the first attempt failed due to wrong command, missing dependency, build failure, service not reaching target state, incomplete search scope, ambiguous path, or stale configuration, prefer `should_retry=true` and ask the planner to try an alternative.
7. If a required user parameter is missing, set `pass=false`, include the missing field, and set `should_retry=false`; the next step should be clarification, not blind retry.
8. If the user explicitly requested a machine-readable or constrained answer shape (for example JSON, exact keys, only a scalar value, table columns, or "only these fields"), verify the candidate answer itself follows that shape. The execution trace may be exposed separately, but the final answer must still contain the requested structured result. If evidence is present but the final answer is prose instead of the requested shape, set `pass=false`, include `output_format`, and set `should_retry=true`.
9. If confidence is low, prefer `pass=true` unless there is a concrete evidence gap, output-shape gap, or unsupported success claim.

Output examples:

{
  "pass": false,
  "missing_evidence_fields": ["size_bytes"],
  "answer_incomplete_reason": "answer confirms the path but omits the requested file size",
  "should_retry": true,
  "retry_instruction": "Collect file metadata for the found path and answer with both path and size_bytes.",
  "confidence": 0.9
}

{
  "pass": false,
  "missing_evidence_fields": ["output_format"],
  "answer_incomplete_reason": "answer has the requested evidence but does not use the requested JSON shape",
  "should_retry": true,
  "retry_instruction": "Use the observed evidence to answer as strict JSON with the requested keys.",
  "confidence": 0.9
}

{
  "pass": false,
  "missing_evidence_fields": ["field_value"],
  "answer_incomplete_reason": "answer confirms the config file exists but does not report the requested key value",
  "should_retry": true,
  "retry_instruction": "Read the relevant config key using a structured config or file tool, then answer with the key value.",
  "confidence": 0.86
}

{
  "pass": true,
  "missing_evidence_fields": [],
  "answer_incomplete_reason": "",
  "should_retry": false,
  "retry_instruction": "",
  "confidence": 0.82
}

## Multilingual Reinforcement
### zh-CN
- 中文里“帮我查/看/跑/改/配置/生成”通常表示需要实际动作；如果候选答案只是教程或建议，且证据没有完成，判为未通过。
- 不要求答案包含固定中文词，只检查是否给出了用户要的结果、证据字段和失败/重试边界；但如果用户明确要求 JSON、字段名、表格列或“只返回某值”，这属于输出形状要求，必须校验。
### en
- English advice-only wording such as "you can run..." is incomplete for an execution task unless execution was blocked or clarification is required.
- Do not require exact field names in ordinary prose answers; when the user explicitly asks for JSON, exact keys, table columns, or an only-value answer, treat that as an output-shape requirement.
