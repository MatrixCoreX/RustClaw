# Dynamic Guard Checkpoint (2026-03-28 17:28 CST)

## 1) 本轮已改动（未编译）

### 代码（需要你编译+重启 clawd 后才生效）
- `crates/clawd/src/prompt_utils.rs`
  - 新增 `parse_llm_json_raw_or_any_with_repair(...)`
  - 新增 JSON 容错修复：`repair_unescaped_inner_quotes(...)`
  - 用于修复 normalizer 输出里字符串内未转义引号导致的 parse_failed
- `crates/clawd/src/intent_router.rs`
  - normalizer / resume_followup 解析改为 `parse_llm_json_raw_or_any_with_repair(...)`
  - parse recovery 日志改为 `parse_recovery=extract_or_repair`
  - 已有的 content-evidence mode 兜底逻辑保留
- `crates/clawd/src/delivery_utils.rs`
  - `take_first_sentence` 的标签行判定支持 markdown 包裹（如 `**核心重点：**`）
  - 增加对应单测（未运行）

### 提示词（已生效，不依赖编译）
- 所有 vendor 的 `intent_normalizer_prompt.md` 已同步新增规则：
  - `names only` 默认解释为直接条目名（文件+目录），不是仅目录
  - `requires_content_evidence=true + locator` 必须走可执行模式（act/chat_act）
  - “上一句/最后一句/一句话讲重点”优先锚定最近 assistant 回复
  - `FILE:<path>` 交付后的内容解读请求，必须绑定该 path 并走 content-evidence 执行
  - JSON 字符串必须合法，内部引号需转义（避免 parse_failed）

## 2) 本轮关键测试产物

### 三条 focus case（完整）
- `scripts/nl_suite_logs/dynamic_guard_context_focus_failed/20260328_165318`
  - 修复前后对比参考基线（case2 仍有旧问题）

### 单 case（case1-only，完整，验证 FILE follow-up 修复）
- `scripts/nl_suite_logs/dynamic_guard_context_focus_case1_only/20260328_170953`
  - case1 三轮全部正确，T3 已能正确给用途总结

### 三条 focus case（最新联合回归，手动中断）
- `scripts/nl_suite_logs/dynamic_guard_context_focus_failed/20260328_171436`
  - case1 全通过
  - case2 跑完但仍不稳定（T1 倾向反问“要不要保存对应关系”）
  - case3 跑到 T2 前被中断（你要求“结束了先”后我已 kill 进程）

## 3) 当前剩余问题（重启后优先处理）

1. `context_ultra_cn_readme_alias` 的 T1 偶发不按“记住别名”执行，变成确认式回复。
2. 这类口语别名绑定（“记住哈，那玩意...”）仍存在模型波动，需要再强化 normalizer 对“显式映射句”的约束。
3. 本轮新加的代码容错（未转义引号 repair）尚未通过你的编译重启验证。

## 4) 重启后建议恢复步骤

1. 你先编译并重启 clawd（让代码修复生效）。
2. 跑三条 focus：
   - `bash /home/guagua/git_upload/scripts/nl_tests/run_multi_turn_suite.sh --suite context_chain --case-file /home/guagua/git_upload/scripts/nl_tests/cases/nl_cases_dynamic_guard_context_failed_focus_20260328.txt --log-root /home/guagua/git_upload/scripts/nl_suite_logs/dynamic_guard_context_focus_failed`
3. 若 case2 T1 仍不稳，再只跑 case2（可单独拆一个 case 文件）做 prompt 微调迭代。

## 5) 进程状态

- 测试脚本进程已停止（`run_multi_turn_suite.sh` 已无残留）。
