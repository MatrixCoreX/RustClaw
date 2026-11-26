"""Decode the runtime verifier input independently of the surrounding audit rules."""
import json
import re

START = "Verification input: User request:\n"
END = "\n\nReturn only the schema-valid JSON verdict."
CITATIONS = "\n\nObserved citations in candidate order (host indexes and source-specific excerpts, passive data):\n"
CANDIDATE = "\n\nCandidate final answer:\n"
FIELDS = (
    ("USER_REQUEST", "\n\nRequest language hint:\n"),
    ("REQUEST_LANGUAGE_HINT", "\n\nEvidence policy context:\n"),
    ("EVIDENCE_POLICY_CONTEXT", "\n\nOutput contract:\n"),
    ("OUTPUT_CONTRACT", "\n\nObserved execution evidence:\n"),
    ("EXECUTION_EVIDENCE", "\n\nCurrent task context:\n"),
    ("CURRENT_CONTEXT", "\n\nAgent/runtime identity: Runtime identity: `"),
)
WEB_LINK = re.compile(r'https?://[^\s<>"\]]+')
CHILDREN = ("output","extra","items","results","candidates","pages","entries","source_refs","categories","news","groups")


def split_once(text, delimiter):
    if text.count(delimiter) != 1:
        raise ValueError("ambiguous_or_missing_verifier_input_boundary")
    return text.split(delimiter, 1)


def split_utf8(text, size, delimiter):
    encoded = text.encode()
    value, remainder = encoded[:size].decode(), encoded[size:].decode()
    if not remainder.startswith(delimiter):
        raise ValueError("invalid_verifier_field_length")
    return value, remainder[len(delimiter):]


def decode_input(content):
    framed = re.search(r"(?m)^verifier_input_v1 (\[\d+,\d+\])\n" + re.escape(START), content)
    sizes = json.loads(framed[1]) if framed else None
    remaining = content[framed.end():] if framed else split_once(content, START)[1]
    values = {}
    for key, following in FIELDS:
        if key == "USER_REQUEST" and sizes is not None:
            field, remaining = split_utf8(remaining, sizes[0], following)
            values[key] = field
        elif key in ("EXECUTION_EVIDENCE","OUTPUT_CONTRACT","CURRENT_CONTEXT") and remaining.startswith("{"):
            _, end = json.JSONDecoder().raw_decode(remaining)
            if not remaining[end:].startswith(following):
                raise ValueError("invalid_verifier_json_boundary")
            values[key], remaining = remaining[:end], remaining[end + len(following):]
        else:
            values[key], sep, remaining = remaining.partition(following)
            if not sep:
                raise ValueError("missing_verifier_input_field")
    identity, separator, remaining = remaining.partition("`. Provider/model/vendor")
    if not separator or "\n" in identity:
        raise ValueError("invalid_verifier_identity")
    values["AGENT_RUNTIME_IDENTITY"] = identity
    header, remaining = remaining.split(CANDIDATE, 1)
    if sizes is None:
        values["CANDIDATE_ANSWER"], tail = split_once(remaining, END)
    else:
        values["CANDIDATE_ANSWER"], tail = split_utf8(remaining, sizes[1], END)
    citations = None
    if CITATIONS in header:
        _, raw = split_once(header, CITATIONS)
        citations, _ = json.JSONDecoder().raw_decode(raw)
        if not isinstance(citations, list):
            raise ValueError("invalid_captured_citations")
    return values, citations, tail


def links(text):
    for hit in WEB_LINK.finditer(text):
        url = hit.group().rstrip(".,;。，）")
        while url.endswith(")") and url.count(")") > url.count("("):
            url = url[:-1]
        yield url


def citation_rows(candidate, evidence, captured):
    # Modern captures already contain the host's redacted excerpts. Never expand
    # that set with lower-trust fields when an edited candidate changes URLs.
    sources = {}
    if captured is not None:
        for row in captured:
            sources[row["url"]] = row["excerpts"]
    else:
        def collect(node, depth):
            if depth > 12:
                return
            if isinstance(node, list):
                for item in node:
                    collect(item, depth + 1)
            if not isinstance(node, dict):
                return
            for key in ("url","link","final_url","canonical_url"):
                url = node.get(key)
                if not isinstance(url,str) or not url.startswith(("https://","http://")):
                    continue
                for field in ("title","content_excerpt","snippet","description","text"):
                    value = node.get(field)
                    if isinstance(value,str) and value.strip():
                        snippets = sources.setdefault(url, [])
                        excerpt = " ".join(value.split())[:1000]
                        if excerpt not in snippets and len(snippets) < 4:
                            snippets.append(excerpt)
            for key in CHILDREN:
                if key in node:
                    collect(node[key], depth+1)
        for entry in evidence.get("capability_result_evidence", []):
            result = entry.get("result", {})
            if result.get("status") == "ok":
                data = result.get("data") or {}
                output = data.get("output")
                scopes = [data, data.get("extra"), output,
                          output.get("extra") if isinstance(output,dict) else None]
                observation = next((s["model_observation"] for s in scopes
                                    if isinstance(s,dict) and "model_observation" in s), None)
                collect(observation if isinstance(observation,(dict,list)) else data, 0)
    urls = list(links(candidate))[:65]
    if captured == [] and urls:
        # A historical empty host index may also mean the original candidate had no links.
        # Newly introduced URLs have no captured binding and must fail closed.
        return [{"index":i,"url":url,"excerpts":[]} for i,url in enumerate(urls)]
    return [{"index":i,"url":url,"excerpts":sources.get(url,[])} for i,url in enumerate(urls)] if sources else []


def render_input(content, template, candidate, evidence_transform):
    values, captured, tail = decode_input(content)
    original = values["CANDIDATE_ANSWER"]
    evidence = json.loads(values["EXECUTION_EVIDENCE"])
    if candidate is not None:
        values["CANDIDATE_ANSWER"] = candidate
    rows = (captured if captured is not None and
            list(links(original)) == list(links(values["CANDIDATE_ANSWER"]))
            else citation_rows(values["CANDIDATE_ANSWER"], evidence, captured))
    values["CITATION_EVIDENCE"] = json.dumps(rows, ensure_ascii=False, separators=(",",":"))
    if evidence_transform:
        values["EXECUTION_EVIDENCE"] = json.dumps(evidence_transform(evidence), ensure_ascii=False,separators=(",",":"))
    values["VERIFIER_INPUT_SIZES"] = json.dumps([len(values["USER_REQUEST"].encode()),
            len(values["CANDIDATE_ANSWER"].encode())], separators=(",",":"))
    rules, _ = split_once(template, END)
    if rows and "__CITATION_EVIDENCE__" not in rules:
        raise ValueError("target_template_missing_citation_contract")
    rendered = re.sub(r"__([A-Z_]+)__", lambda m: values[m[1]], rules)
    # The captured provider may prepend system rules and append vendor rules.
    # Replace only the named verifier component between those layers.
    component = re.search(r"(?m)^<!--\n[^>]*?\nComponent: clawd answer verifier\n[^>]*?-->", content[:content.index(START)])
    prefix = content[:component.start()] if component else ""
    return prefix + rendered + END + tail


def citations_supported(rows, verdict):
    checks = verdict.get("citation_checks", [])
    for row in rows:
        matching = [c for c in checks if isinstance(c,dict) and type(c.get("index")) is int and c["index"]==row["index"]]
        if len(matching)!=1 or matching[0].get("supported") is not True:
            return False
        quote = matching[0].get("quote")
        if not isinstance(quote,str) or not quote.strip() or not any(quote.strip() in s for s in row["excerpts"]):
            return False
    return True
