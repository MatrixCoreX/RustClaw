# Failed Routing Cases

用于记录路由/决策失败样本，供 prompt 迭代回放与回归测试使用。

## 使用说明

- 每条案例一行，保持简洁。
- `expected_mode` 建议填写：`chat` / `act` / `chat_act`；若是“必须先澄清”场景可填 `ask_clarify`。
- `expected_profile` / `selected_profile` 建议使用：`chat_general` / `action_general` / `chat_act` / `image_tasks` / `schedule_tasks` / `safety_sensitive`。
- `root_cause` 建议使用统一枚举：
  - `intent_ambiguous`
  - `router_misclassify`
  - `json_format_fail`
  - `tool_planning_fail`
  - `tail_handling_fail`
- `status` 建议：`open` / `fixed` / `wontfix`

## Case Table

| id | date | user_request | expected_mode | actual_mode | expected_profile | selected_profile | root_cause | impact | status | notes |
|---|---|---|---|---|---|---|---|---|---|---|
| R-001 | 2026-02-28 | 帮我先生成一张图，再解释为什么这样构图 | chat_act | act | image_tasks | action_general | router_misclassify | high | open | 缺少“动作+解释”复合判定 |
| R-002 | 2026-02-28 | Please run uname -a and summarize the result | chat_act | act | chat_act | action_general | router_misclassify | medium | open | 英文复合请求误判 |
| R-003 | 2026-03-01 | Ejecuta `df -h` y luego resume los riesgos de espacio en disco | chat_act | act | chat_act | action_general | router_misclassify | high | open | 西语“执行+总结”被判纯动作 |
| R-004 | 2026-03-01 | このコマンドを実行して、結果を短く説明して: `ls -la` | chat_act | act | chat_act | action_general | router_misclassify | high | open | 日语复合意图漏判 |
| R-005 | 2026-03-01 | 이 명령 실행하고 결과를 요약해줘: `free -h` | chat_act | act | chat_act | action_general | router_misclassify | high | open | 韩语复合意图漏判 |
| R-006 | 2026-03-01 | 先列出当前目录文件，然后告诉我哪些可能是日志文件 | chat_act | act | chat_act | action_general | router_misclassify | high | open | 中文“先做再解释”漏判 |
| R-007 | 2026-03-01 | Please generate an image of a red fox and explain the composition choices | chat_act | act | image_tasks | action_general | router_misclassify | high | open | 图像任务中的复合意图漏判 |
| R-008 | 2026-03-01 | 分析这段输出是什么意思：`Permission denied` | chat | act | chat_general | action_general | router_misclassify | medium | open | 仅解释请求被误判为执行 |
| R-009 | 2026-03-01 | Continue | chat | act | chat_general | action_general | intent_ambiguous | medium | open | 缺少可解析最近目标时应澄清而非执行 |
| R-010 | 2026-03-01 | 继续，把那个删掉 | ask_clarify | act | safety_sensitive | action_general | intent_ambiguous | high | open | 指代不清，误执行风险高 |
| R-011 | 2026-03-01 | Resume all schedules and then tell me which ones changed | chat_act | act | schedule_tasks | action_general | router_misclassify | high | open | 日程场景复合意图漏判 |
| R-012 | 2026-03-01 | 请把这张图改成水彩风格，并说明你改了哪些细节 | chat_act | act | image_tasks | action_general | router_misclassify | high | open | 改图+说明复合意图漏判 |

## 回放统计口径（建议）

- 统计范围：默认统计 `status=open` 的全部案例；每次 prompt 变更后再加一轮 `recent fixed` 回放。
- `mode_accuracy` = `actual_mode == expected_mode` 的案例数 / 统计总数。
- `profile_accuracy` = `selected_profile == expected_profile` 的案例数 / 统计总数。
- `high_impact_recall` = 高影响（`impact=high`）案例中命中正确的比例（可分别算 mode/profile）。
- `root_cause_top` = 按 `root_cause` 分组计数并排序，优先处理 Top1/Top2。
- `chat_act_precision_focus`：单独看 `expected_mode=chat_act` 子集的命中率（复合意图专项指标）。
- `clarify_safety_focus`：单独看 `expected_mode=ask_clarify` 子集是否被错误执行（安全专项指标）。

建议每次回放输出固定摘要：
- `samples_total`:
- `mode_accuracy`:
- `profile_accuracy`:
- `high_impact_mode_accuracy`:
- `high_impact_profile_accuracy`:
- `chat_act_accuracy`:
- `ask_clarify_misexecute_count`:
- `root_cause_top3`:

快速计算（默认仅统计 `status=open`）：
- `python3 scripts/routing_replay_metrics.py`
- `python3 scripts/routing_replay_metrics.py --status all --json`

## Backlog (可选)

- [ ] 每周新增失败样本 >= 10 条
- [ ] 每周关闭（fixed）高影响样本 >= 3 条
- [ ] 每次 prompt 改动后回放全部 `open` + 最近 `fixed` 样本

