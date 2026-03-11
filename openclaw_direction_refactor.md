# Clawd 向 OpenClaw 方向改造方案

## 目标

当前 `clawd` 的主要问题不是“能力不够”，而是前置 LLM 判定链过长、结果分层不够清晰、`chat` 参与主结果收尾过深，导致：

- 简单请求也要串行请求多次 LLM
- 上下文绑定容易漂移
- 已经拿到结果后，仍可能因为收尾文案失败而整任务失败
- 同一份结果在执行层、交付层、日志层重复承载

本方案的目标是把 `clawd` 逐步改造成更接近 OpenClaw 的方向：

- 以单一 agent loop 为中心
- 把“上下文理解 + 下一步决策 + 工具选择”合并到同一轮主推理
- 降低前置分类 prompt 数量
- 让工具结果优先直接交付
- 让 `chat` 从必经主链路降级为可选表达层

这不是 1 次重写，而是分阶段迁移。

## 现状问题

当前 `ask` 链路大致是：

1. `resume_followup_intent`
2. `context_resolver`
3. `schedule_intent`
4. `intent_router`
5. `chat_response` 或 `single_plan_execution`
6. 必要时再进入 `loop_incremental_plan`

这套设计的问题：

### 1. LLM 调用次数过多

一个简单请求也可能走 4 次前置判定，再进入真正执行。

### 2. 上下文理解被拆得太细

`resume`、`context`、`schedule`、`route` 分开后：

- 容易相互覆盖
- 容易出现“局部判断没错，但整体路由错”
- 短补充句更容易被误绑到旧失败任务

### 3. `chat` 技能职责过重

现在 `chat` 有时不仅负责纯聊天，还会在 act 任务末尾承担“最终自然语言交付”职责。  
这会导致：

- 真实结果已经拿到
- 但 `chat` 空回包后，整任务仍被判失败

### 4. 结果分层不清

同一份内容可能同时出现在：

- 工具执行结果
- `respond`
- `task_call_end`
- 某些情况下还会进 `progress_messages`

导致用户和排障都容易误判为“重复执行”。

## OpenClaw 风格的核心思路

OpenClaw 方向的重点不是“更多 prompt”，而是“一个主 agent loop + 统一模型抽象”。

核心思想：

1. 当前轮主 LLM 直接理解上下文语义
2. 当前轮主 LLM 直接决定下一步
3. 如果需要能力，就调用工具
4. 工具结果回流后，再进入下一轮决策
5. 不额外拆很多前置语义分类器

可以抽象成：

`用户请求 -> Agent Loop -> 选择工具/直接回复 -> 工具结果回流 -> 下一步`

而不是：

`resume -> context -> schedule -> route -> planner -> tool -> chat -> final`

## 目标架构

建议把新架构收敛成 4 层。

### 1. Channel Layer

负责：

- Telegram / WhatsApp / UI / Script 接入
- 投递统一 task
- 只消费最终交付结果

不负责：

- 猜最终结果正文
- 消费内部 trace

### 2. Agent Runtime Layer

这是改造后的核心。

职责：

- 维护当前 turn 状态
- 组织 system prompt / history / tools / memory
- 发起主 LLM 调用
- 执行 loop
- 决定是否继续下一轮

这一层取代今天的“前置多段 LLM 判路由链”。

### 3. Tool / Skill Layer

职责：

- 执行真实动作
- 返回结构化结果

原则：

- 工具输出优先保持原始、可交付
- 不要求每次都再走一次 `chat`

### 4. Delivery Layer

只做三类内容分层：

- `progress`: 过程提示
- `delivery`: 最终交付
- `trace`: 排障日志

其中：

- 用户侧只消费 `delivery`
- `trace` 永远不参与对外发送

## 建议的新执行链

### 普通请求

1. 收到用户请求
2. Agent Runtime 组织当前上下文
3. 一次主 LLM 决策：
   - 直接回复
   - 或调用一个工具
4. 若调用工具：
   - 执行工具
   - 工具结果写入 loop state
5. 主 LLM 再决定：
   - 直接把已有结果交付
   - 或在必要时继续下一步

### 关键变化

旧设计：

- 先分类
- 再分类
- 再分类
- 再路由
- 再执行

新设计：

- 先进入 agent loop
- 当前轮一次性理解上下文和下一步

## `chat` 的新定位

`chat` 不建议取消，但必须降级。

### 应保留的场景

- 纯聊天
- 讲笑话
- 用户明确要求改写、润色、总结、变口吻
- 基于已有工具结果生成一句自然语言

### 不应继续承担的职责

- act 任务的统一收尾器
- 已有结果后的必经最终步骤
- 决定整任务成败的最后依赖

### 新原则

1. 已有直接可交付结果时，优先直接交付
2. 用户明确要求语气化表达时，才调用 `chat`
3. `chat` 失败时，fallback 到原始结果，不应把主任务判失败

一句话：

`chat` 应该是 decoration，不应该是 dependency。

## 结果分层规范

建议把所有输出固定分成三层。

### 1. progress

只允许放：

- 已开始执行
- 正在调用工具
- 已拆分出几步
- 正在等待确认

禁止放：

- 完整命令输出
- 完整文件内容
- 最终答复正文

### 2. delivery

只允许放最终交付物：

- 文本答复
- `FILE:...`
- `IMAGE_FILE:...`
- `VOICE_FILE:...`

这是通信端唯一应该对外发的层。

### 3. trace

只给系统排障用：

- 原始工具输出
- 规划原文
- task_call_end 归档
- LLM 原始响应

这一层不能被 adapter 当成用户结果。

## Prompt 设计调整

### 需要保留的 prompt

新架构里 prompt 数量要显著减少，建议先只保留：

1. `agent_loop_prompt`
- 主 LLM 决策 prompt
- 负责上下文理解、是否调用工具、下一步动作

2. `chat_response_prompt`
- 纯聊天或明确风格化表达时使用

3. 可选的 `memory_summary_prompt`
- 只做后台记忆压缩
- 不参与用户交付

### 建议删除或并入

逐步下线或并入主 loop 的：

- `resume_followup_intent`
- `context_resolver`
- `schedule_intent`
- `intent_router`

这些能力不是消失，而是并入主 agent loop 的当前轮决策。

## 上下文语义处理原则

OpenClaw 方向下，不再单独搞多个“语义判断器”，而是由主 LLM 在单轮里结合：

- 最近用户消息
- 最近 assistant 回答
- 当前未完成任务
- 工具结果
- memory

一起判断：

- 当前请求是不是延续上文
- 是继续执行，还是新问题
- 是否要调工具

但要明确优先级：

1. 当前轮和最近几轮对话
2. 当前 task state
3. 最近工具结果
4. memory
5. 更老的失败任务

这样可以明显减少“短补充句误绑到旧失败任务”。

## Task State 设计建议

建议显式维护以下状态，而不是靠 scattered 日志拼：

- `run_id`
- `user_request`
- `current_goal`
- `active_context`
- `tool_history`
- `delivery_messages`
- `trace_events`
- `stop_reason`

其中：

- `active_context` 表示当前主题，不等于最近失败任务
- `tool_history` 用于避免重复调用
- `delivery_messages` 是最终交付候选
- `trace_events` 只用于排障

## 工具调用原则

主 agent loop 每轮只做一件事：

- 直接回复
- 或调用一个明确工具

如果确实需要多步，也尽量采用“执行一小步 -> 看结果 -> 再决定”的方式，而不是一开始把长计划锁死。

好处：

- 减少计划漂移
- 减少重复步骤
- 减少 `respond` 重复包装

## 对现有 clawd 的迁移建议

建议分 4 个阶段迁移。

### Phase 1：先改结果分层

目标：

- 固定 `progress / delivery / trace`
- 通信端只消费 `delivery`
- 把重复发送问题先压住

这是最先做、风险最低的一步。

### Phase 2：弱化 `chat`

目标：

- `chat` 从主依赖变成可选表达层
- 已有工具结果时可直接交付
- `chat` 失败时允许 fallback

这是解决“结果已拿到却整任务失败”的关键一步。

### Phase 3：合并前置路由

目标：

- 把 `resume/context/schedule/router` 并入单一 `agent_loop_prompt`
- 保留旧链路做 fallback

这是最重要的结构改造。

### Phase 4：完全切换到 loop-first

目标：

- 普通 ask 默认直接进 agent loop
- 老的前置多 prompt 路由链仅保留兼容入口或逐步下线

到这一步，才算真正接近 OpenClaw 风格。

## 最小可落地版本

如果不想一次改太大，建议先做这个“最小 OpenClaw 化版本”：

1. 保留现有 tools / skills
2. 保留现有 adapter
3. 新增一个统一 `agent_loop_prompt`
4. 让普通 ask 先走新 loop
5. `chat` 变成 optional formatter
6. 旧 `resume/context/router` 暂时只给 legacy path 使用

这样能最快看到收益：

- LLM 次数下降
- 上下文误绑减少
- 结果交付更稳

## 风险

### 1. 单轮主 LLM 负担变重

把之前多层判断合并后，单轮 prompt 会更复杂。  
解决方式：

- 控制上下文长度
- 保留明确 schema
- 减少无关 memory 注入

### 2. 模型依赖更集中

旧架构的问题是 prompt 太多；新架构的问题是更依赖主模型质量。  
解决方式：

- 把主 loop schema 做稳
- 把工具 contract 写死
- 用 trace 记录每轮决策输入输出

### 3. 迁移期双架构并存

一段时间内会存在：

- 旧 `clawd` 路由链
- 新 agent loop

需要明确 feature flag，避免结果源混乱。

## 推荐的第一批落地点

如果马上开始做，我建议顺序是：

1. 先写一份新的 `agent_loop_prompt`
2. 定义新的 `AgentTurnDecision` schema
3. 把 `chat` 调整为 optional formatter
4. 固定 `progress / delivery / trace`
5. 让一小类请求先走新 loop：
   - 查持仓
   - 路径查询
   - 简单文件交付
   - 单步命令

不要第一天就覆盖：

- 交易确认流
- 定时任务
- 复杂多轮 resume

这些应该最后迁。

## 推荐的决策 schema

可以先把主 LLM 决策收敛成这种结构：

```json
{
  "decision": "reply|call_tool|call_skill|clarify|stop",
  "reason": "string",
  "tool": "optional",
  "skill": "optional",
  "args": {},
  "response_mode": "direct|formatted",
  "confidence": 0.0
}
```

这个 schema 的好处是：

- 足够简单
- 能覆盖大多数当前 ask 场景
- 比现在多个前置分类 prompt 更容易观察和调试

## 总结

这次改造的核心不是“再优化某个 prompt”，而是改变 `clawd` 的主思路：

- 从“多段前置分类”
- 转到“单一 agent loop 驱动”

同时明确两条底线：

1. 已经拿到真实结果时，优先直接交付
2. `chat` 失败不能再轻易拖垮一个已成功的 act 任务

如果沿这个方向推进，`clawd` 会更接近 OpenClaw 的优点：

- 更少的前置 LLM 调用
- 更统一的上下文语义处理
- 更稳定的工具驱动 loop
- 更清晰的交付边界
