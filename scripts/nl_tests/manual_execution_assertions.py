"""Same-invocation assertions over runtime-attested structured tool evidence."""
import json


def field_values(step, field):
    items = step.get("observed_evidence", {}).get("items", [])
    for item in items:
        if not isinstance(item, dict) or item.get("field") != field or item.get("redacted"):
            continue
        kind, excerpt = item.get("kind"), item.get("excerpt")
        if kind == "string" and isinstance(excerpt, str):
            yield excerpt
        elif kind in {"bool", "number", "null"} and isinstance(excerpt, str):
            try:
                yield json.loads(excerpt)
            except ValueError:
                pass


def step_contract_assertion(spec, calls):
    detail = {"kind": "step_contract_json", "ok": False, "matched_step_ids": []}
    try:
        contract = json.loads(spec)
        if not isinstance(contract, dict):
            return detail
        skill, status = contract["skill"], contract["status"]
        fields = contract["fields"]
        present = contract.get("present", [])
        count = contract.get("count", 1)
        if (not isinstance(skill, str) or not skill or status not in {"ok", "error"}
                or not isinstance(fields, dict) or not fields or type(count) is not int or count < 1
                or not isinstance(present, list) or any(not isinstance(x, str) for x in present)):
            return detail
        for step in calls:
            if step.get("executed_skill") != skill or step.get("status") != status:
                continue
            observed = step.get("observed_evidence", {}).get("items", [])
            available = {x.get("field") for x in observed if isinstance(x, dict) and not x.get("redacted")}
            if not set(present) <= available:
                continue
            if all(any(type(value) is type(wanted) and value == wanted
                       for value in field_values(step, field)) for field, wanted in fields.items()):
                detail["matched_step_ids"].append(step.get("step_id"))
        detail["ok"] = len(detail["matched_step_ids"]) == count
    except (ValueError, KeyError, TypeError, AttributeError):
        pass
    return detail
