"""Replay a captured verifier request with revised rules, without executing tools."""
import argparse
import copy
import hashlib
import json
import os
import re
from pathlib import Path
import tomllib
import urllib.request
import urllib.error
from jsonschema import Draft202012Validator


def parse_verdict(content):
    clean = content.rsplit("</think>", 1)[-1].strip()
    if clean.startswith("```json\n") and clean.endswith("```"):
        clean = clean[len("```json\n"):-3].strip()
    elif clean.startswith("```\n") and clean.endswith("```"):
        clean = clean[4:-3].strip()
    parsed = json.loads(clean)
    if not isinstance(parsed, dict) or type(parsed.get("pass")) is not bool:
        raise ValueError("invalid_verifier_verdict")
    return clean, parsed


def matches_expectation(parsed, expected_pass, expected_issue, schema=None):
    if schema is not None and not Draft202012Validator(schema).is_valid(parsed):
        return False
    if parsed.get("pass") is not expected_pass:
        return False
    if expected_pass:
        for check in parsed.get("operation_checks", []):
            dispatches = check.get("required_dispatches") or []
            evidence = check.get("evidence_step_ids") or []
            if not dispatches and not evidence:
                if (check.get("applicable", True) is False
                        or check.get("method_observed")
                        or check.get("blocked", False)):
                    return False
                continue
            if check.get("applicable", True) is False:
                if (check.get("method_observed") is not False
                        or check.get("result_observed") is not True
                        or check.get("blocked", False) is not False
                        or not check.get("evidence_step_ids")):
                    return False
            elif check.get("blocked", False):
                if not check.get("evidence_step_ids"):
                    return False
            elif (check.get("method_observed") is not bool(check.get("required_dispatches", [None]))
                  or check.get("result_observed") is not True
                  or not check.get("evidence_step_ids")):
                return False
        return True
    issues = parsed.get("missing_evidence_fields")
    return isinstance(issues, list) and expected_issue in issues


def section(text, start, end):
    if text.count(start) != 1 or text.count(end) != 1:
        raise ValueError("prompt_section_not_unique")
    first = text.index(start) + len(start)
    last = text.index(end, first)
    return first, last


def receive_response(request, log, payload):
    try:
        with urllib.request.urlopen(request, timeout=300) as response:
            raw = json.loads(response.read())
    except urllib.error.HTTPError as error:
        body = error.read().decode("utf-8", errors="replace")
        try:
            raw = json.loads(body)
        except ValueError:
            raw = body
        failure = {"status": "provider_http_error", "http_status": error.code,
                   "raw_response": raw, "request_payload": payload}
        log.seek(0)
        log.truncate()
        log.write(json.dumps(failure, ensure_ascii=False))
        log.flush()
        raise
    log.seek(0)
    log.truncate()
    log.write(json.dumps({"request_payload": payload, "raw_response": raw,
                          "status": "response_received"}, ensure_ascii=False))
    log.flush()
    return raw


def render_full_template(content, template):
    sections = [
        ("__USER_REQUEST__", "User request:\n", "Request language hint:"),
        ("__REQUEST_LANGUAGE_HINT__", "Request language hint:\n", "Evidence policy context:"),
        ("__EVIDENCE_POLICY_CONTEXT__", "Evidence policy context:\n", "Output contract:"),
        ("__OUTPUT_CONTRACT__", "Output contract:\n", "Observed execution evidence:"),
        ("__EXECUTION_EVIDENCE__", "Observed execution evidence:\n", "Current task context:"),
        ("__CURRENT_CONTEXT__", "Current task context:\n", "Agent/runtime identity:"),
        ("__CANDIDATE_ANSWER__", "Candidate final answer:\n", "Judgment fields:"),
    ]
    values = {}
    for key, start, end in sections:
        left, right = section(content, start, end)
        values[key] = content[left:right].strip()
    marker = "- The agent runtime identity is `"
    if content.count(marker) != 1:
        raise ValueError("runtime_identity_not_unique")
    values["__AGENT_RUNTIME_IDENTITY__"] = content.split(marker, 1)[1].split("`", 1)[0]
    return re.sub(r"__[A-Z_]+__", lambda match: values[match.group()], template)


def revised_request(record, template, candidate=None, execution_snapshot=None, output_protocol=None,
                    full_template=False, compact_evidence=False):
    payload = copy.deepcopy(record["request_payload"])
    if payload.get("stream") is not False or len(payload.get("messages", [])) != 1:
        raise ValueError("unsupported_captured_request_shape")
    message = payload["messages"][0]
    start, end = section(message["content"], "Hard rejection checklist:\n", "\nRules:\n")
    left, right = section(template, "Hard rejection checklist:\n", "\nRules:\n")
    message["content"] = message["content"][:start] + template[left:right] + message["content"][end:]
    if candidate is not None:
        start, end = section(message["content"], "Candidate final answer:\n", "\n\nJudgment fields:")
        message["content"] = message["content"][:start] + candidate + message["content"][end:]
    if execution_snapshot is not None:
        data = execution_snapshot.get("data", {})
        if data.get("task_id") != record.get("task_id"):
            raise ValueError("execution_snapshot_task_mismatch")
        trace = (data.get("result_json") or {}).get("task_journal", {}).get("trace", {})
        steps = trace.get("step_results")
        if not isinstance(steps, list) or not steps:
            raise ValueError("execution_snapshot_steps_missing")
        keys = ("step_id", "executed_skill", "status", "requested_action_type",
                "requested_capability", "requested_action_ref", "resolved_capability")
        operations = [{key: step.get(key) for key in keys} for step in steps
                      if step.get("executed_skill") not in ("respond", "think", "synthesize_answer", "answer_verifier")]
        start, end = section(message["content"], "Observed execution evidence:\n", "\nCurrent task context:")
        evidence = json.loads(message["content"][start:end])
        evidence["executed_operations"] = {"source": "runtime_step_results", "ordered": True,
                                           "truncated": False, "operations": operations}
        message["content"] = (message["content"][:start] + json.dumps(evidence, ensure_ascii=False)
                              + "\n" + message["content"][end:])
    if full_template:
        message["content"] = render_full_template(message["content"], template)
    if compact_evidence:
        start, end = section(message["content"], "Observed execution evidence:\n", "\nCurrent task context:")
        evidence = compact_evidence_projection(json.loads(message["content"][start:end]))
        message["content"] = (message["content"][:start] + json.dumps(evidence, ensure_ascii=False, separators=(",", ":"))
                              + "\n" + message["content"][end:])
    if output_protocol:
        message["content"] += "\n\nFinal verification output protocol:\n" + output_protocol
    return payload


def compact_evidence_projection(evidence):
    result = copy.deepcopy(evidence)
    for item in result.get("capability_result_evidence", []):
        envelope = item.get("result") or {}
        item.setdefault("step_id", envelope.get("provenance", {}).get("step_id"))
        data = envelope.get("data") or {}
        output = data.get("output")
        if isinstance(output, dict) and data.get("extra") is not None and output.get("extra") == data["extra"]:
            del output["extra"]
            output["extra_reference"] = "data.extra"
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-log", type=Path, required=True)
    parser.add_argument("--offset", type=int, required=True)
    parser.add_argument("--task-id", required=True)
    parser.add_argument("--template", type=Path, required=True)
    parser.add_argument("--config", type=Path, required=True)
    parser.add_argument("--vendor", required=True)
    parser.add_argument("--api-key-env", required=True)
    parser.add_argument("--candidate", type=Path)
    parser.add_argument("--execution-snapshot", type=Path)
    parser.add_argument("--output-protocol", type=Path)
    parser.add_argument("--full-template", action="store_true")
    parser.add_argument("--compact-evidence", action="store_true",
                        help="Apply the runtime's lossless duplicate-evidence projection to captured evidence")
    parser.add_argument("--schema", type=Path,
                        help="Validate the raw verdict against this schema before accepting it")
    parser.add_argument("--expect-pass", choices=["true", "false"], required=True)
    parser.add_argument("--expect-issue", default="unsupported_claims")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    with args.source_log.open("rb") as handle:
        handle.seek(args.offset)
        record = json.loads(handle.readline())
    if record.get("task_id") != args.task_id or "answer_verifier_prompt.md" not in record.get("prompt_source", ""):
        raise ValueError("source_record_mismatch")
    payload = revised_request(record, args.template.read_text(),
                              args.candidate.read_text().strip() if args.candidate else None,
                              json.loads(args.execution_snapshot.read_text()) if args.execution_snapshot else None,
                              args.output_protocol.read_text() if args.output_protocol else None,
                              args.full_template, args.compact_evidence)
    provider = tomllib.loads(args.config.read_text())["llm"][args.vendor]
    if payload["model"] != provider["model"] or not provider["base_url"].startswith("https://"):
        raise ValueError("provider_binding_invalid")
    body = json.dumps(payload, ensure_ascii=False).encode()
    request = urllib.request.Request(provider["base_url"].rstrip("/") + "/chat/completions",
        data=body, headers={"Content-Type": "application/json",
                           "Authorization": "Bearer " + os.environ[args.api_key_env]})
    args.output.parent.mkdir(parents=True, exist_ok=True)
    # Reserve output first; a replay must never overwrite earlier evidence.
    with args.output.open("x") as log:
        log.write(json.dumps({"request_payload": payload, "status": "requesting"}, ensure_ascii=False))
        log.flush()
        raw = receive_response(request, log, payload)
        choice = raw["choices"][0]
        content = choice["message"]["content"]
        clean, parsed = parse_verdict(content)
        expected = args.expect_pass == "true"
        schema = json.loads(args.schema.read_text()) if args.schema else None
        accepted = matches_expectation(parsed, expected, args.expect_issue, schema)
        result = {"schema_version": 1, "llm_call_ref": "LLM#1", "provider": args.vendor,
                  "model": payload["model"], "prompt_label": record.get("prompt_source"),
                  "logical_prompt_path": record.get("prompt_source"), "source_log": str(args.source_log.resolve()),
                  "source_offset": args.offset, "source_task_id": args.task_id,
                  "execution_snapshot": str(args.execution_snapshot.resolve()) if args.execution_snapshot else None,
                  "output_protocol": str(args.output_protocol.resolve()) if args.output_protocol else None,
                  "full_template": args.full_template,
                  "compact_evidence": args.compact_evidence,
                  "schema_path": str(args.schema.resolve()) if args.schema else None,
                  "schema_valid": Draft202012Validator(schema).is_valid(parsed) if schema else None,
                  "template_sha256": hashlib.sha256(args.template.read_bytes()).hexdigest(),
                  "request_sha256": hashlib.sha256(body).hexdigest(), "request_payload": payload,
                  "response_text": clean, "parsed_json": parsed, "raw_response": raw,
                  "finish_reason": choice.get("finish_reason"), "usage": raw.get("usage"),
                  "error": None, "accepted": accepted, "expected_pass": expected,
                  "expected_issue": None if expected else args.expect_issue}
        log.seek(0)
        log.truncate()
        log.write(json.dumps(result, ensure_ascii=False, indent=2) + "\n")
    print(json.dumps({k: v for k, v in result.items() if k not in ("request_payload", "raw_response")}, ensure_ascii=False))
    return 0 if accepted else 1


if __name__ == "__main__":
    raise SystemExit(main())
