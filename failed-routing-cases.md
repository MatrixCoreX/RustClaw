# Failed Routing Cases

用于记录路由/决策失败样本，供 prompt 迭代回放与回归测试使用。

## 使用说明

- 每条案例一行，保持简洁。
- `expected_mode` 只填：`chat` / `act` / `chat_act`。
- `root_cause` 建议使用统一枚举：
  - `intent_ambiguous`
  - `router_misclassify`
  - `json_format_fail`
  - `tool_planning_fail`
  - `tail_handling_fail`
- `status` 建议：`open` / `fixed` / `wontfix`

## Case Table

| id | date | user_request | expected_mode | actual_mode | root_cause | impact | status | notes |
|---|---|---|---|---|---|---|---|---|
| R-001 | 2026-02-28 | 帮我先生成一张图，再解释为什么这样构图 | chat_act | act | router_misclassify | high | open | 缺少“动作+解释”复合判定 |
| R-002 | 2026-02-28 | Please run uname -a and summarize the result | chat_act | act | router_misclassify | medium | open | 英文复合请求误判 |

## Backlog (可选)

- [ ] 每周新增失败样本 >= 10 条
- [ ] 每周关闭（fixed）高影响样本 >= 3 条
- [ ] 每次 prompt 改动后回放全部 `open` + 最近 `fixed` 样本

