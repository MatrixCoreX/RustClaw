# Client-Like NL Expectations

This directory stores optional JSONL expectation files for
`scripts/nl_tests/evaluate_client_like_run.py`.

Each row targets one `case` number from a client-like continuous run and may
assert only the fields that matter for that regression:

```json
{"case": 12, "status": "succeeded", "first_layer": "planner_execute", "capability_any": ["filesystem.list_entries"], "executed_any": ["fs_basic"], "final_shape": "list"}
```

Supported fields:

- `status`
- `first_layer`
- `first_layer_any`
- `routed_mode`
- `routed_mode_any`
- `route_gate`
- `route_gate_any`
- `capability_any`
- `executed_any`
- `verifier_approved`
- `verifier_issue_any`
- `verifier_issue_all`
- `needs_confirmation`
- `finalizer_stage`
- `finalizer_fallback`
- `finalizer_grounded_ok`
- `finalizer_used_evidence_ids_min`
- `final_contains`
- `final_shape`

Use the evaluator to generate a starting baseline from a known-good run:

```bash
python3 scripts/nl_tests/evaluate_client_like_run.py \
  scripts/nl_suite_logs/client_like_continuous/<run_id> \
  --write-baseline /tmp/rustclaw-client-like-baseline.jsonl
```

Then copy only stable, meaningful assertions into a source-controlled
expectation file. Do not lock brittle full replies or provider-specific wording
unless the user-facing text is the regression under test.
For cases where multiple routes are acceptable, prefer the `*_any` variants
over forcing one exact first-layer or route-gate value.
