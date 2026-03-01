# Prompt Regression Cases

本清单用于验证以下目标：
- 路由稳定性（`chat/act/chat_act/ask_clarify`）
- 结构化输出稳定性（JSON 可解析、字段完整）
- 多语一致性（中英及其他语言）
- 回退行为可观测（parse failed / fallback）

## 1) 文本路由回归（intent_router + context_resolver）

每个用例至少跑 3 次（不同 provider 或不同会话），统计一致率。

| case_id | 输入 | 关键上下文 | 期望 mode | 期望约束 |
|---|---|---|---|---|
| route_chat_only_explain | 请解释这段命令输出是什么意思 | recent: 无待执行动作 | chat | 不触发工具调用 |
| route_act_image_gen | 帮我生成一张赛博朋克海报 | recent: 空 | act | 不要求额外解释 |
| route_chat_act_cmd_with_summary | please run uname -a and tell me result | recent: 空 | chat_act | 动作+结果叙述 |
| route_ask_clarify_ambiguous_continue | 继续 | recent: 无可解析目标 | ask_clarify | 返回澄清导向 |
| route_act_followup_continue | 继续 | recent#1: run_cmd pending | act | 绑定最近未完成动作 |
| route_act_bulk_schedule_delete | 全部删除 | recent#1: schedule list 多任务 | act | 识别为批量调度动作 |
| route_chat_identity | 你是谁 | recent: 空 | chat | 纯对话 |

## 2) 图像动作回归（image_vision）

每个 action 至少验证：
- 返回 JSON（可被 `serde_json` 解析）
- 必填字段存在
- 不确定信息走 `uncertainties`

| case_id | action | 输入 | 期望字段 |
|---|---|---|---|
| image_describe_basic | describe | 单图（UI 截图） | `summary,objects,visible_text,uncertainties` |
| image_compare_two_versions | compare | 两张相似图 | `summary,similarities,differences,notable_changes,uncertainties` |
| image_screenshot_summary | screenshot_summary | 单张后台报错截图 | `purpose,critical_text,warnings,next_actions,uncertainties` |
| image_extract_with_schema | extract | schema 指定发票字段 | 匹配外部 schema |
| image_extract_default | extract | 无 schema | 至少为合法 JSON 对象 |

## 3) 语音模式与语音对话回归（telegramd）

| case_id | 输入文本/转写 | 期望 |
|---|---|---|
| voice_mode_switch_voice | 切换成语音回复 | `voice_mode_intent=voice` |
| voice_mode_switch_text | switch back to text mode | `voice_mode_intent=text` |
| voice_mode_switch_both | 都要 | `voice_mode_intent=both`（结合最近询问上下文） |
| voice_mode_show | 现在是什么回复模式 | `voice_mode_intent=show` |
| voice_chat_same_language | 中文转写文本 | 回复保持中文 |
| voice_chat_noisy_clarify | 噪声大且语义不完整转写 | 返回一个简短澄清问句 |

## 4) 定时意图回归（schedule_intent）

| case_id | 输入 | 期望 |
|---|---|---|
| schedule_create_daily | 每天 9:30 提醒我复盘 | `kind=create`,`type=daily`,`time=09:30` |
| schedule_create_once_relative | 明天早上8点提醒我开会 | `kind=create`,`type=once`,`run_at` 合法 |
| schedule_list | 查看定时任务 | `kind=list` |
| schedule_delete_one | 删除定时任务 job_9e289b4c73 | `kind=delete`,`target_job_id=job_9e289b4c73` |
| schedule_delete_bulk | 删除所有定时任务 | `kind=delete`,`target_job_id=""` |
| schedule_pause_bulk_followup | 先列出任务 -> 用户: 全部暂停 | `kind=pause`,`target_job_id=""` |
| schedule_none | 今天天气怎么样 | `kind=none` |

## 5) 量化指标（建议阈值）

每次回归记录以下指标：

- `json_parse_success_rate`
  - 定义：要求 JSON 的场景中，可解析次数 / 总次数
  - 目标：`>= 99%`

- `required_field_completeness`
  - 定义：必填字段全部存在的响应数 / 总响应数
  - 目标：`>= 99%`

- `route_consistency_rate`
  - 定义：同一 case 多次运行得到相同 mode 的比例
  - 目标：`>= 95%`

- `multilingual_consistency_rate`
  - 定义：多语同义请求在 mode 与核心语义上的一致比例
  - 目标：`>= 95%`

- `fallback_trigger_rate`
  - 定义：日志中 parse_failed / fallback 默认路径触发次数 / 总请求数
  - 目标：持续下降；单批回归建议 `< 5%`

## 6) 执行建议

- 与现有 `document/regression_llm_first/run_all.sh` 集成：新增 prompt 回归入口脚本，按上述 case_id 批量跑。
- 每次 prompt 改动后至少跑：
  - 路由 7 条
  - 图像 5 条
  - 语音 6 条
  - 定时 7 条
- 结果输出统一写入 `logs/model_io.log` 与独立回归报告（JSON/Markdown 均可），便于对比前后版本。
