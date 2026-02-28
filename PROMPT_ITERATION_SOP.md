# RustClaw Prompt 迭代 SOP

## 1. 目标

建立一套可持续优化流程，让路由与执行相关提示词在真实使用中持续提升：

- 命中率更高（少漏判）
- 误判率更低（少错判）
- 输出更稳定（可解析、可执行）

本 SOP 适用于：

- `prompts/intent_router_prompt.md`
- `prompts/agent_runtime_prompt.md`
- `prompts/image_tail_routing_prompt.md`
- 以及后续新增的决策型提示词

---

## 2. 迭代原则

- **单点改动**：一次只改一类问题（例如只改路由误判，不同时改工具规划）。
- **样本驱动**：必须基于真实失败样本，不凭感觉大改。
- **结构优先**：先强化输出 schema 与约束，再补 few-shot。
- **可回滚**：每次迭代都可快速回退到上一版提示词。

---

## 3. 失败样本收集规范

每条样本记录以下字段（建议存成表格或 JSON）：

- `id`: 样本编号
- `time`: 时间
- `user_request`: 用户原文
- `expected_mode`: 期望模式（chat/act/chat_act）
- `actual_mode`: 实际模式
- `impact`: 影响级别（high/medium/low）
- `root_cause`: 失败原因归类（见下）
- `notes`: 补充说明

失败原因建议归类：

- `intent_ambiguous`：请求语义含混
- `router_misclassify`：路由模式判断错误
- `json_format_fail`：JSON 不合规或解析失败
- `tool_planning_fail`：有动作但规划不到位
- `tail_handling_fail`：收尾逻辑触发错误

---

## 4. 周期与节奏（建议每周一次）

### 周一：样本归集

- 汇总最近一周失败/边界样本
- 按原因归类并排序（先处理 high impact）

### 周二：小步改稿

- 每次只改 1~2 个提示词
- 保持输出 schema 不变（避免引入兼容问题）

### 周三：离线回放

- 用样本集回放评估
- 统计指标（见第 5 节）

### 周四：灰度上线

- 观察日志、解析失败率、误判类型变化

### 周五：复盘与固化

- 记录本次有效改动与无效改动
- 更新 SOP 的“高价值规则库”

---

## 5. 评估指标（最少保留这 4 个）

- **Route Accuracy**：路由正确率
- **JSON Parse Success**：JSON 可解析率
- **Action Success**：需要动作场景的任务成功率
- **Fallback Rate**：回退路径触发率（越低越好）

建议目标：

- JSON 解析成功率 >= 99%
- 高影响样本路由正确率每周稳步提升

---

## 6. Prompt 编写模板（决策型）

建议固定四段结构：

1) **角色与任务**：你是谁、只做什么  
2) **模式定义**：各模式边界  
3) **优先级规则**：冲突时怎么判  
4) **输出约束**：JSON-only + 固定 schema + 禁止额外文本

强约束模板要点：

- 明确“只输出一个 JSON 对象”
- 给出严格合法示例
- 明确禁止 markdown/code fence/额外 key
- 不确定时给保守策略（例如偏 `chat_act`）

---

## 7. Few-shot 使用准则

- 每个提示词保留 3~6 条高价值样例
- 样例必须覆盖：
  - 容易混淆的边界请求
  - 多语言请求（中英 + 其他语言）
  - “既要动作又要解释”的复合请求
- 样例要短，避免污染主规则

---

## 8. 回归测试清单（每次改 prompt 必跑）

- 纯聊天请求 -> `chat`
- 纯动作请求 -> `act`
- 动作 + 解释请求 -> `chat_act`
- 多语言等价请求（至少 3 种语言）模式一致
- 非法输出防护：无 markdown、无多余 key、JSON 可解析

---

## 9. 发布与回滚

发布前：

- 记录本次变更文件与目标指标
- 保留上一版 prompt 副本（或通过 git tag/commit 定位）

发布后：

- 观察 24h 指标与关键失败样本
- 若核心指标恶化，立即回滚到上一版

---

## 10. 当前项目建议的下一步

- 给 `intent_router_prompt.md` 增加少量高价值 few-shot（3~5 条）
- 给 `image_tail_routing_prompt.md` 增加“反例规则”（识图不触发收尾）
- 建立 `failed-routing-cases.md` 样本池并周更

---

## 11. 版本记录（模板）

每次迭代追加：

- `date`:
- `changed_prompts`:
- `hypothesis`:
- `metrics_before`:
- `metrics_after`:
- `regression_found`:
- `rollback_needed`:

