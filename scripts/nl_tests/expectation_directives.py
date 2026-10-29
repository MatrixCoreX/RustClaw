"""Separate assertion directives without changing embedded JSON payloads."""
import json


_JSON_PAYLOADS = {"step_contract_json", "capability_result_json", "skill_outcome_json", "workspace_file_cycle",
                  "result_text_json_records_eq"}
_JSON_EQUALS = {"result_text_json_eq", "result_text_json_one_of", "json_eq"}


def split_expectation_directives(spec: str) -> list[str]:
    decoder = json.JSONDecoder()
    directives = []
    start = 0
    while start < len(spec):
        while start < len(spec) and (spec[start].isspace() or spec[start] == ";"):
            start += 1
        if start == len(spec):
            break
        end = spec.find(";", start)
        end = len(spec) if end < 0 else end
        colon = spec.find(":", start, end)
        if colon >= 0:
            kind = spec[start:colon]
            payload = colon + 1
            if kind in _JSON_EQUALS:
                equals = spec.find("=", payload, end)
                payload = equals + 1 if equals >= 0 else end
            if kind in _JSON_PAYLOADS | _JSON_EQUALS:
                while payload < len(spec) and spec[payload].isspace():
                    payload += 1
                if payload < len(spec) and spec[payload] in '{["':
                    try:
                        _, boundary = decoder.raw_decode(spec, payload)
                    except ValueError as exc:
                        raise ValueError("invalid_expectation_json") from exc
                    while boundary < len(spec) and spec[boundary].isspace():
                        boundary += 1
                    if boundary < len(spec) and spec[boundary] != ";":
                        raise ValueError("invalid_expectation_json_boundary")
                    end = boundary
        directives.append(spec[start:end].strip())
        start = end + 1
    return directives


def separate_confirmation(spec: str) -> tuple[str, str]:
    kept, confirmation = [], ""
    for directive in split_expectation_directives(spec):
        if directive.startswith("confirm:"):
            confirmation = directive[len("confirm:"):].strip()
        else:
            kept.append(directive)
    return ";".join(kept), confirmation
