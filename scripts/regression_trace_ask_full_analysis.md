# regression_trace_ask 全量测试结果分析

**日志文件**: `scripts/regression_trace_ask_full.log`  
**运行配置**: BASE_URL=http://127.0.0.1:8787, WAIT=240s, CASES=71  
**结束标志**: `DONE: all cases finished`

---

## 1. 总体统计

| 指标 | 数值 |
|------|------|
| **总用例数** | 71 |
| **succeeded** | 64 |
| **failed** | 7 |
| **通过率（按终态）** | 64/71 ≈ 90.1% |
| **启发式告警** | 2 条（见下） |

---

## 2. 七个 status=failed 的用例（均为预期失败）

这 7 个用例都是「多步执行中故意失败 + 暴露 resume_context」的设计，失败时均带有 **`[resume_context] yes`**，行为符合预期。

| 用例名 | 说明 |
|--------|------|
| **mid_fail_then_summary_new** | 先 pwd → 不存在的命令 → 总结；第 2 步失败，带“可回复继续”提示 |
| **mid_fail_then_continue_target** | echo BEFORE_BREAK → 不存在的命令 → echo AFTER_BREAK_67890；第 2 步失败 |
| **en_mid_fail_then_summary** | 英文版：pwd → 不存在的命令 → summarize |
| **followup_explain_failure_then_resume** | echo BREAK_A → 不存在的命令 → “说一下哪一步失败、还剩什么，先不要继续”；第 2 步失败 |
| **en_followup_explain_failure_then_resume** | 英文版同上 |
| **followup_resume_with_change** | BEFORE_CHANGE → 不存在的命令 → AFTER_CHANGE_OLD；失败后用户可说“继续”并改最后一步 |
| **en_followup_resume_with_change** | 英文版同上 |

**结论**: 无意外失败；多步中断与 resume_context 机制按设计工作。

---

## 3. 启发式 [CHECK] 告警（2 条）

### 3.1 “继续” 未落到 AskClarify/Act

- **用例**: `followup_continue_new`
- **提示**: 「"继续" 未落到 AskClarify/Act，需检查 follow-up 路由。」
- **实际情况**: 该 case **status=succeeded**，最终输出为 `AFTER_BREAK_67890`，说明用户说“继续”后走了 **resume 路径**，正确执行了剩余步骤（echo AFTER_BREAK_67890）。
- **分析**: 脚本启发式期望「继续」被路由到 AskClarify 或 Act；当前实现把「继续」识别为 resume 并执行剩余步骤，从行为上是对的。可视为**启发式与设计不一致**，非功能 bug；若希望“继续”也作为澄清/显式 Act 入口，可再调路由或提示词。

### 3.2 ChatAct 创作类需人工关注

- **用例**: `en_reply_only_synonym_1`
- **提示**: 「该 case 走了 ChatAct，需人工关注后续创作是否被前序 act 输出污染。」
- **说明**: 该用例为「先执行再创作」（如 pwd 后写一句诗），脚本对「创作 + ChatAct」类做了保守提示，建议人工抽查前序 act 输出是否过度影响创作质量。
- **建议**: 若有该 case 的黄金答案或评审标准，可做一次人工抽检；无则仅作提示即可。

---

## 4. 路由与执行概况（从 TRACE/LLM 归纳）

- **Chat**: 纯聊天（冷笑话、天气等）→ `routed_mode=Chat`，无 executor_step。
- **Act**: 隐藏文件、数文件、ls/df 等 → `routed_mode=Act`，有 `executor_step_execute` + `executor_result_ok`。
- **ChatAct**: 先执行再总结/写诗/播报 → `routed_mode=ChatAct`，先工具再 chat 技能。
- **Resume**: 「继续」/“continue” 等正确触发 `decision=resume`、`bind_resume_context=true`，并执行剩余步骤或给出总结（如 AFTER_BREAK_67890、AFTER_CHANGE_OLD_EN、AFTER_PATCHED_STEP 等）。

中英文、同义表达（继续/接着/Go on/pick up from there 等）的 follow-up 用例均按预期完成。

---

## 5. 小结与建议

| 项目 | 结论 |
|------|------|
| **是否全部跑完** | 是，71 个 case 均执行完毕并输出 `DONE: all cases finished`。 |
| **失败用例** | 7 个均为「中间故意失败 + resume_context」的回归用例，行为符合设计。 |
| **非预期失败** | 0。 |
| **启发式告警** | 2 条：1）follow-up「继续」路由与启发式预期不同（功能正确）；2）ChatAct 创作类建议人工关注。 |
| **建议** | 1）若需严格区分「继续」走 AskClarify/Act vs resume，可调整路由或启发式规则；2）对 `en_reply_only_synonym_1` 做一次人工抽检（可选）。 |

**整体**: 本次 71 例 regression_trace_ask 全量测试通过，无需要修的功能性问题。
