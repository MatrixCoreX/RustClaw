#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RUNNER="${RUNNER:-$ROOT_DIR/target/release/skill-runner}"
REPORT_PATH="${REPORT_PATH:-$ROOT_DIR/logs/regression_skills_upgrade_$(date +%Y%m%d_%H%M%S).md}"

PASS=0
FAIL=0
SKIP=0
RESULT_LINES=()
CURRENT_CASE=""

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "Missing command: $1"
    exit 2
  }
}

need_cmd jq

if [[ ! -x "$RUNNER" ]]; then
  echo "skill-runner not found: $RUNNER"
  echo "Build first: cargo build -p skill-runner --release"
  exit 2
fi

TMP_DIR="$(mktemp -d /tmp/skills-upgrade-regression-XXXXXX)"
trap 'rm -rf "$TMP_DIR"' EXIT

log_case() {
  echo
  echo "== $1 =="
  CURRENT_CASE="$1"
  RESULT_LINES+=("")
  RESULT_LINES+=("## $1")
}

pass() {
  PASS=$((PASS+1))
  echo "PASS: $1"
  RESULT_LINES+=("- PASS: $1")
}

fail() {
  FAIL=$((FAIL+1))
  echo "FAIL: $1"
  RESULT_LINES+=("- FAIL: $1")
}

skip() {
  SKIP=$((SKIP+1))
  echo "SKIP: $1"
  RESULT_LINES+=("- SKIP: $1")
}

run_skill() {
  local skill="$1"
  local args_json="$2"
  local request_id="reg-$(date +%s)-$RANDOM"
  jq -nc \
    --arg rid "$request_id" \
    --arg skill "$skill" \
    --argjson args "$args_json" \
    '{
      request_id: $rid,
      user_id: 1,
      chat_id: 1,
      skill_name: $skill,
      args: $args,
      context: null
    }' | "$RUNNER"
}

payload_from_resp() {
  local resp="$1"
  echo "$resp" | jq -cer '.text | fromjson'
}

assert_jq() {
  local json="$1"
  local expr="$2"
  local msg="$3"
  if echo "$json" | jq -e "$expr" >/dev/null; then
    pass "$msg"
  else
    fail "$msg"
    echo "  expr: $expr"
    echo "  json: $json"
  fi
}

log_case "reference_resolver resolved/not_found"
rr_args_resolved='{
  "action":"resolve_reference",
  "request_text":"上个回复里的那个文件",
  "target_type":"file",
  "language_hint":"zh-CN",
  "recent_turns":[{"role":"assistant","turn_id":"a1","text":"已保存到 /tmp/report.md"}],
  "recent_results":[]
}'
rr_resp="$(run_skill "reference_resolver" "$rr_args_resolved")"
rr_payload="$(payload_from_resp "$rr_resp")"
assert_jq "$rr_payload" '.status=="resolved"' "reference_resolver should resolve single clear file reference"

rr_args_nf='{
  "action":"resolve_reference",
  "request_text":"那个依赖",
  "target_type":"dependency",
  "recent_turns":[],
  "recent_results":[]
}'
rr_resp_nf="$(run_skill "reference_resolver" "$rr_args_nf")"
rr_payload_nf="$(payload_from_resp "$rr_resp_nf")"
assert_jq "$rr_payload_nf" '.status=="not_found"' "reference_resolver should return not_found when no candidates"

log_case "doc_parse md"
DOC_MD="$TMP_DIR/sample.md"
cat >"$DOC_MD" <<'EOF'
# Title

Alpha section content.

| name | score |
| ---- | ----- |
| A    | 10    |
| B    | 20    |
EOF

doc_args="$(jq -nc --arg p "$DOC_MD" '{
  action:"parse_doc",
  path:$p,
  include_metadata:true,
  table_mode:"basic",
  max_chars:20000
}')"
doc_resp="$(run_skill "doc_parse" "$doc_args")"
doc_payload="$(payload_from_resp "$doc_resp")"
assert_jq "$doc_payload" '.status=="ok"' "doc_parse should parse markdown"
assert_jq "$doc_payload" '.metadata.type=="md"' "doc_parse metadata.type should be md"
assert_jq "$doc_payload" '(.sections|length)>=1' "doc_parse should emit sections"

log_case "transform nested/group/aggregate"
tf_args='{
  "action":"transform_data",
  "strict":true,
  "null_policy":"keep",
  "output_format":"json",
  "data":[
    {"user":{"name":"A","city":"sz"},"score":"10"},
    {"user":{"name":"B","city":"sz"},"score":"20"},
    {"user":{"name":"C","city":"sh"},"score":"30"}
  ],
  "ops":[
    {"op":"filter","field":"score","cmp":"gte","value":15},
    {"op":"group","by":["user.city"],"aggregations":[
      {"op":"count","name":"cnt"},
      {"op":"avg","field":"score","name":"avg_score"}
    ]}
  ]
}'
tf_resp="$(run_skill "transform" "$tf_args")"
tf_payload="$(payload_from_resp "$tf_resp")"
assert_jq "$tf_payload" '.status=="ok"' "transform should run successfully"
assert_jq "$tf_payload" '(.result|length)>=1' "transform should output grouped rows"
assert_jq "$tf_payload" '.stats.input_count==3' "transform stats.input_count should be correct"

log_case "kb ingest/search"
KB_DIR="$TMP_DIR/kb_docs"
mkdir -p "$KB_DIR"
cat >"$KB_DIR/a.txt" <<'EOF'
alpha beta gamma
EOF
cat >"$KB_DIR/b.md" <<'EOF'
# Note
gamma delta epsilon
EOF

KB_NS="reg_ns_$(date +%s)"
kb_ingest_args="$(jq -nc --arg ns "$KB_NS" --arg p "$KB_DIR" '{
  action:"ingest",
  namespace:$ns,
  paths:[$p],
  overwrite:true,
  file_types:["txt","md"],
  chunk_size:400
}')"
kb_ingest_resp="$(run_skill "kb" "$kb_ingest_args")"
kb_ingest_payload="$(payload_from_resp "$kb_ingest_resp")"
assert_jq "$kb_ingest_payload" '.status=="ok"' "kb ingest should succeed"

kb_search_args="$(jq -nc --arg ns "$KB_NS" '{
  action:"search",
  namespace:$ns,
  query:"alpha gamma",
  top_k:5
}')"
kb_search_resp="$(run_skill "kb" "$kb_search_args")"
kb_search_payload="$(payload_from_resp "$kb_search_resp")"
assert_jq "$kb_search_payload" '.status=="ok"' "kb search should succeed"
assert_jq "$kb_search_payload" '(.hits|length)>=1' "kb search should return hits"

log_case "web_search_extract backend error boundary"
ws_args='{
  "action":"search",
  "query":"playwright",
  "backend":"unsupported_backend",
  "top_k":3
}'
ws_resp="$(run_skill "web_search_extract" "$ws_args")"
ws_payload="$(payload_from_resp "$ws_resp")"
assert_jq "$ws_payload" '.status=="error"' "web_search_extract should return explicit error for unsupported backend"
assert_jq "$ws_payload" '.error_code=="SEARCH_FAILED"' "web_search_extract should expose SEARCH_FAILED"

log_case "browser_web basic invocation (network dependent)"
bw_args='{
  "action":"open_extract",
  "urls":["https://example.com"],
  "wait_until":"domcontentloaded",
  "save_screenshot":false,
  "capture_images":false,
  "max_text_chars":1000,
  "fail_fast":false
}'
bw_resp="$(run_skill "browser_web" "$bw_args")"
bw_status="$(echo "$bw_resp" | jq -r '.status // "unknown"')"
if [[ "$bw_status" == "ok" ]]; then
  # browser_web skill may return success with partial failures in text payload; treat as pass if response shape is valid.
  if echo "$bw_resp" | jq -e '.text | fromjson | has("summary")' >/dev/null 2>&1; then
    pass "browser_web should return structured payload"
  else
    fail "browser_web response missing structured summary"
    echo "  resp: $bw_resp"
  fi
else
  skip "browser_web invocation skipped (runner/skill returned non-ok status): $bw_status"
fi

echo
echo "==== Regression Summary ===="
echo "PASS: $PASS"
echo "FAIL: $FAIL"
echo "SKIP: $SKIP"

mkdir -p "$(dirname "$REPORT_PATH")"
{
  echo "# Skills Upgrade Regression Report"
  echo
  echo "- Time: $(date '+%Y-%m-%d %H:%M:%S %Z')"
  echo "- Runner: \`$RUNNER\`"
  echo "- PASS: $PASS"
  echo "- FAIL: $FAIL"
  echo "- SKIP: $SKIP"
  echo
  for line in "${RESULT_LINES[@]}"; do
    echo "$line"
  done
} >"$REPORT_PATH"
echo "Report saved: $REPORT_PATH"

if [[ "$FAIL" -gt 0 ]]; then
  exit 1
fi
exit 0
