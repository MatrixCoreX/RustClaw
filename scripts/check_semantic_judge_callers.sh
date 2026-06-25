#!/usr/bin/env bash
# §3.4 调用面守卫：semantic_judge 的 LLM 入口 (is_meta_respond_instruction /
# is_publishable_raw) 只允许 finalize 层调用。
#
# 白名单：
#   - crates/clawd/src/finalize/loop_reply.rs
#   - crates/clawd/src/finalize/loop_reply_contract_enforce.rs (finalize-tier contract pruning)
#   - crates/clawd/src/finalize/loop_reply_observed_contract.rs (finalize-tier observed answer gate)
#   - crates/clawd/src/agent_engine/observed_output.rs (observed_answer_fallback 兜底)
#
# 用法：
#   bash scripts/check_semantic_judge_callers.sh
#   exit code 0 = 干净；非 0 = 有违规调用。
#
# 接入 CI 建议：在 cargo fmt/clippy 之前跑一次。

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

WHITELIST=(
    "crates/clawd/src/finalize/loop_reply.rs"
    "crates/clawd/src/finalize/loop_reply_contract_enforce.rs"
    "crates/clawd/src/finalize/loop_reply_observed_contract.rs"
    "crates/clawd/src/agent_engine/observed_output.rs"
)

# 找 src/ 下所有调用 is_meta_respond_instruction 或 is_publishable_raw 的文件，
# 排除 semantic_judge.rs 本身（定义点 + 函数注释允许出现这些标识符）。
HITS=$(rg -l 'semantic_judge::(is_meta_respond_instruction|is_publishable_raw)\b' \
    crates/clawd/src \
    --glob '!**/semantic_judge.rs' \
    || true)

if [ -z "$HITS" ]; then
    echo "[check_semantic_judge_callers] OK: no LLM-tier semantic_judge callers found."
    exit 0
fi

VIOLATIONS=()
while IFS= read -r file; do
    [ -z "$file" ] && continue
    found=0
    for w in "${WHITELIST[@]}"; do
        if [ "$file" = "$w" ]; then
            found=1
            break
        fi
    done
    if [ $found -eq 0 ]; then
        VIOLATIONS+=("$file")
    fi
done <<< "$HITS"

if [ ${#VIOLATIONS[@]} -eq 0 ]; then
    echo "[check_semantic_judge_callers] OK: all callers in finalize whitelist."
    echo "  whitelisted files containing calls:"
    while IFS= read -r f; do
        [ -n "$f" ] && echo "    - $f"
    done <<< "$HITS"
    exit 0
fi

echo "[check_semantic_judge_callers] FAIL: §3.4 violation detected."
echo ""
echo "The following files import semantic_judge LLM-tier functions"
echo "(is_meta_respond_instruction / is_publishable_raw) but are NOT in the"
echo "finalize whitelist:"
echo ""
for f in "${VIOLATIONS[@]}"; do
    echo "  - $f"
    rg -n 'semantic_judge::(is_meta_respond_instruction|is_publishable_raw)\b' "$f" \
        | sed 's/^/      /'
done
echo ""
echo "Per Phase 3 §3.4: these functions may only be called from the finalize"
echo "tier. Other layers should rely on planner contracts, observed facts,"
echo "and structured runtime guards rather than calling this LLM classifier."
echo ""
echo "If you genuinely need to extend the whitelist, edit this script and"
echo "document the rationale in docs/ or the calling site comment."
exit 1
