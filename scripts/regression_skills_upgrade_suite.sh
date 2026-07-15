#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RUNNER="${RUNNER:-$ROOT_DIR/target/release/skill-runner}"
REPORT_PATH="${REPORT_PATH:-$ROOT_DIR/logs/regression_skills_upgrade_$(date +%Y%m%d_%H%M%S).md}"
INCLUDE_WRAPPER_SMOKE=1
WRAPPER_SMOKE_PROFILE="${WRAPPER_SMOKE_PROFILE:-release}"
WRAPPER_SMOKE_TIMEOUT="${WRAPPER_SMOKE_TIMEOUT:-60}"
WRAPPER_SMOKE_LOG_DIR=""
WRAPPER_SMOKE_REPORT=""
INCLUDE_BASE_CONTRACTS=1
BASE_CONTRACTS_PROFILE="${BASE_CONTRACTS_PROFILE:-release}"
BASE_CONTRACTS_REPORT=""

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

path_ref() {
  python3 "${ROOT_DIR}/scripts/path_ref.py" --root "$ROOT_DIR" "$1"
}

if [[ ! -x "$RUNNER" ]]; then
  echo "skill-runner not found: $RUNNER"
  echo "Build first: cargo build -p skill-runner --release"
  exit 2
fi

TMP_DIR="$(mktemp -d /tmp/skills-upgrade-regression-XXXXXX)"
WRAPPER_SMOKE_STDOUT="$TMP_DIR/wrapper_smoke.log"
trap 'rm -rf "$TMP_DIR"' EXIT

while [[ $# -gt 0 ]]; do
  case "$1" in
    --skip-wrapper-smoke)
      INCLUDE_WRAPPER_SMOKE=0
      shift
      ;;
    --wrapper-smoke-profile)
      WRAPPER_SMOKE_PROFILE="${2:-release}"
      shift 2
      ;;
    --wrapper-smoke-timeout)
      WRAPPER_SMOKE_TIMEOUT="${2:-60}"
      shift 2
      ;;
    --skip-base-contracts)
      INCLUDE_BASE_CONTRACTS=0
      shift
      ;;
    --base-contracts-profile)
      BASE_CONTRACTS_PROFILE="${2:-release}"
      shift 2
      ;;
    --help|-h)
      cat <<EOF
Usage:
  bash scripts/regression_skills_upgrade_suite.sh [options]

Options:
  --skip-wrapper-smoke           Skip scripts/smoke_skill_calls.sh stage
  --wrapper-smoke-profile P      Wrapper smoke profile (default: release)
  --wrapper-smoke-timeout N      Timeout per wrapper in seconds (default: 60)
  --skip-base-contracts          Skip base skill response-contract stage
  --base-contracts-profile P     Base skill contract profile (default: release)
EOF
      exit 0
      ;;
    *)
      echo "Unknown argument: $1"
      exit 2
      ;;
  esac
done

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

if [[ "$INCLUDE_WRAPPER_SMOKE" == "1" ]]; then
  log_case "wrapper smoke"
  WRAPPER_SMOKE_LOG_DIR="$ROOT_DIR/logs/skill_call_smoke_from_upgrade_$(date +%Y%m%d_%H%M%S)"
  WRAPPER_SMOKE_REPORT="$WRAPPER_SMOKE_LOG_DIR/report.md"
  set +e
  REPORT_PATH="$WRAPPER_SMOKE_REPORT" \
  LOG_DIR="$WRAPPER_SMOKE_LOG_DIR" \
  bash "$ROOT_DIR/scripts/smoke_skill_calls.sh" \
    --profile "$WRAPPER_SMOKE_PROFILE" \
    --timeout "$WRAPPER_SMOKE_TIMEOUT" \
    --exclude "audio_transcribe,audio_synthesize,image_generate,image_edit,image_vision,crypto,stock,weather,browser_web,web_search_extract,service_control,task_control,chat" >"$WRAPPER_SMOKE_STDOUT" 2>&1
  wrapper_rc=$?
  set -e
  if [[ "$wrapper_rc" -eq 0 ]]; then
    pass "wrapper smoke completed successfully (report_ref: $(path_ref "$WRAPPER_SMOKE_REPORT"))"
  else
    fail "wrapper smoke reported failures (report_ref: $(path_ref "$WRAPPER_SMOKE_REPORT"))"
    echo "  smoke_log_ref: $(path_ref "$WRAPPER_SMOKE_STDOUT")"
  fi
fi

if [[ "$INCLUDE_BASE_CONTRACTS" == "1" ]]; then
  log_case "base skill response contracts"
  BASE_CONTRACTS_REPORT="$TMP_DIR/base_contracts_report.md"
  if bash "$ROOT_DIR/scripts/check_base_skill_response_contracts.sh" \
      --profile "$BASE_CONTRACTS_PROFILE" \
      --log-dir "$TMP_DIR/base_contracts_logs" \
      --report "$BASE_CONTRACTS_REPORT"; then
    pass "base skill response contracts passed"
  else
    fail "base skill response contracts failed"
  fi
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
  if [[ -n "$WRAPPER_SMOKE_REPORT" ]]; then
    echo "- Wrapper smoke report ref: \`$(path_ref "$WRAPPER_SMOKE_REPORT")\`"
  fi
  if [[ -n "$BASE_CONTRACTS_REPORT" ]]; then
    echo "- Base contract report ref: \`$(path_ref "$BASE_CONTRACTS_REPORT")\`"
  fi
  echo
  for line in "${RESULT_LINES[@]}"; do
    echo "$line"
  done
  if [[ -n "$WRAPPER_SMOKE_REPORT" ]]; then
    echo
    echo "## Wrapper Smoke Report"
    echo
    echo "See ref: \`$(path_ref "$WRAPPER_SMOKE_REPORT")\`"
  fi
  if [[ -n "$BASE_CONTRACTS_REPORT" ]]; then
    echo
    echo "## Base Skill Response Contract Report"
    echo
    echo "See ref: \`$(path_ref "$BASE_CONTRACTS_REPORT")\`"
  fi
} >"$REPORT_PATH"
echo "Report saved ref: $(path_ref "$REPORT_PATH")"

if [[ "$FAIL" -gt 0 ]]; then
  exit 1
fi
exit 0
