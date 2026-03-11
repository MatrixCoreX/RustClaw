# Clawd 向 OpenClaw 方向改造：给 Codex 的实施清单

## 目标

把当前 `clawd` 从“多段前置 LLM 路由链”逐步改造成“单一 agent loop 驱动”的结构，同时做到：

- 降低简单请求的 LLM 次数
- 减少上下文误绑定
- 让工具结果优先直接交付
- 让 `chat` 从主依赖降级为可选表达层
- 清理 `progress / delivery / trace` 的边界

本清单要求：

- 优先做最小、可验证的结构调整
- 不一次性推翻所有旧逻辑
- 每一步都保留可回退路径

## 总体原则

1. 不要先重写全部系统。
2. 先收敛结果分层，再调整执行链。
3. 先让一小类请求走新 loop，再逐步扩大。
4. `chat` 不能再作为 act 成功与否的唯一收尾依赖。
5. 工具真实结果优先直接交付。

## Phase 1：结果分层收敛

### 目标

明确三类输出：

- `progress`
- `delivery`
- `trace`

并保证通信端只消费 `delivery`。

### 需要处理的文件

- `/home/guagua/RustClaw/crates/clawd/src/agent_engine.rs`
- `/home/guagua/RustClaw/crates/clawd/src/main.rs`
- `/home/guagua/RustClaw/crates/telegramd/src/main.rs`

### 具体任务

1. 找出当前所有会把内容写入 `progress_messages`、`messages`、`text`、`task_call_end` 的位置。
2. 列一份映射表：
   - 哪些属于过程提示
   - 哪些属于最终交付
   - 哪些只是内部 trace
3. 调整规则：
   - `progress_messages` 只允许过程提示
   - `messages` 为最终交付主入口
   - `text` 仅作为 `messages` 为空时的 fallback
   - `task_call_end result=...` 不再承载整段用户可见正文
4. 检查 `telegramd`：
   - 只发送最终交付
   - 不把 trace 当正文发出

### 验收标准

对 `ls -l`、`pwd`、`查币安持仓` 这类任务：

- 日志里仍可见 trace
- 用户侧只收到一次最终结果
- 不再出现“像执行了 3 次”的体验

## Phase 2：弱化 chat 技能

### 目标

把 `chat` 从 act 主链路依赖，改成可选表达层。

### 需要处理的文件

- `/home/guagua/RustClaw/crates/clawd/src/agent_engine.rs`
- `/home/guagua/RustClaw/crates/clawd/src/main.rs`
- `/home/guagua/RustClaw/crates/skills/chat/src/main.rs`
- 相关 planner prompt 文件

### 具体任务

1. 梳理所有 `call_skill(chat)` 被 planner 生成的场景。
2. 区分两类：
   - 纯聊天主任务
   - act 之后的格式化/风格化需求
3. 调整执行策略：
   - 若工具/技能已经拿到可直接交付的结果，允许直接结束
   - 只有用户明确要求“解释/改写/总结/口吻化”时，才调用 `chat`
4. 定义 fallback：
   - `chat` 失败时，如果已有真实工具结果，则优先交付原始结果
   - 不要把整任务直接打成失败

### 验收标准

对 `查一下我币安持仓`：

- `crypto.positions` 成功后，应可直接交付
- 即使 `chat` 空回包，也不能把整任务打成失败

对 `先执行 pwd，再用一句江湖口吻播报结果`：

- 允许 `chat` 参与，因为用户明确要求风格化表达

## Phase 3：引入新的 agent loop 决策 schema

### 目标

新增统一的主 LLM 决策结构，替代 today 的多段前置判断。

### 新 schema 建议

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

### 需要处理的文件

- `/home/guagua/RustClaw/crates/clawd/src/agent_engine.rs`
- `/home/guagua/RustClaw/crates/clawd/src/main.rs`
- 新增或重构主 loop prompt

### 具体任务

1. 新增一个 `agent_loop_prompt`
   - 负责理解当前语义
   - 决定下一步是回复、调工具、调技能还是澄清
2. 新增一个统一解析结构 `AgentTurnDecision`
3. 在 Runtime 中实现一条新的 loop-first 执行路径
4. 先不要删老路由链
   - 新 loop 先作为 feature flag 或新入口存在

### 验收标准

最少以下场景能走新 loop：

- `ls -l`
- `pwd`
- `查一下我币安持仓`
- `写到哪个目录去了`
- `帮我发 HelloWorld.java`

## Phase 4：把前置多 prompt 路由链并入 loop

### 目标

逐步淡化以下 prompt 的主职责：

- `resume_followup_intent`
- `context_resolver`
- `schedule_intent`
- `intent_router`

把它们的语义合并到主 agent loop 决策中。

### 需要处理的文件

- `/home/guagua/RustClaw/crates/clawd/src/intent_router.rs`
- `/home/guagua/RustClaw/crates/clawd/src/main.rs`
- prompt 目录下上述相关文件

### 具体任务

1. 先保留老逻辑不删。
2. 在新 loop 路径中，不再强制串行调用上述 4 个 prompt。
3. 通过当前轮上下文直接判断：
   - 是否是新请求
   - 是否是当前主题补充
   - 是否要继续旧任务
   - 是否要澄清
   - 是否直接调用工具
4. 老 prompt 先退化为 fallback 或兼容路径。

### 验收标准

以下场景不再依赖老式串行前置链也能正确处理：

- `我是ubuntu系统`
- `在201上`
- `路径是 /home/...`
- `继续`
- `上一个失败原因是什么`

## Phase 5：缩小新 loop 的试点范围并逐步扩展

### 初始试点任务类型

先只让下面几类走新 loop：

1. 简单单步命令
2. 查路径 / 查文件
3. 查持仓 / 查价格
4. 文件交付类
5. 简单 follow-up

### 暂缓迁移的任务类型

先不要进新 loop 主路径：

1. 交易确认流
2. 高风险交易提交
3. 定时任务
4. 复杂多轮 resume 执行
5. 特殊多媒体任务

### 验收标准

新 loop 试点开启后：

- 简单请求的 LLM 次数下降
- 错误定位更集中
- 日志能清楚区分当前轮决策与工具结果

## Prompt 改造任务

### 目标

把 prompt 数量从“很多前置分类器”收缩到“少量主干 prompt”。

### Codex 需要做的事

1. 新建统一主 prompt：
   - `agent_loop_prompt.md`
2. 保留：
   - `chat_response_prompt.md`
   - `memory_summary_prompt.md`（若已有）
3. 逐步降低以下 prompt 的主职责：
   - `resume_followup_intent_prompt.md`
   - `context_resolver_prompt.md`
   - `schedule_intent_prompt.md`
   - `intent_router_prompt.md`
4. 对所有 vendor：
   - 保持同一套决策 schema
   - 允许少量 vendor tuning
   - 不要再让不同 vendor 各自长成不同架构

### 验收标准

- 不同 vendor 的主 loop prompt 结构一致
- 只有少量语气/格式兼容差异

## 日志与调试层改造

### 目标

把 LLM 调试层单独抽出来，避免“模型问题”和“交付问题”混在一起。

### 需要记录的内容

1. 当前轮输入上下文摘要
2. 主 LLM 原始返回
3. 解析后的 `AgentTurnDecision`
4. 工具调用
5. 工具结果
6. 最终交付内容

### Codex 需要做的事

1. 为新 loop 增加清晰 trace 字段
2. 保证一眼能看出：
   - 当前轮为什么选这个动作
   - 有没有继续下一轮
   - 最终交付是什么
3. 不要再让 `task_call_end` 承载大段正文

### 验收标准

排查一条失败任务时，可以快速判断：

- 是 LLM 决策错了
- 是工具错了
- 还是交付层错了

## 代码层具体要求

### 数据结构

Codex 需要引入或整理这些结构：

- `AgentTurnDecision`
- `LoopState`
- `DeliveryPayload`
- `TraceEvent`

### 推荐字段

`LoopState` 至少包含：

- `run_id`
- `user_request`
- `active_context`
- `tool_history`
- `delivery_messages`
- `trace_events`
- `stop_reason`

`DeliveryPayload` 至少包含：

- `progress_messages`
- `messages`
- `text`

### 设计要求

1. 不要让 trace 结构反向污染 delivery。
2. 不要让 `chat` 改写真实工具结果的事实内容。
3. 不要让 memory 块直接变成新的可执行指令。

## 回归用例

Codex 每次做完一个阶段，至少用这些用例验证：

1. `执行 ls -l`
2. `执行 pwd`
3. `查一下我币安持仓`
4. `写到哪个目录去了`
5. `我是ubuntu系统`
6. `上一个失败原因是什么`
7. `继续`
8. `把 HelloWorld.java 发我`
9. `先执行 pwd，再用一句江湖口吻播报结果`

重点检查：

- 是否重复交付
- 是否误绑旧失败任务
- 是否已拿到结果却仍被 `chat` 失败拖垮

## 不该做的事

1. 不要第一步就删老架构。
2. 不要一次性重写所有 prompt。
3. 不要把新 loop 一上来就覆盖所有高风险任务。
4. 不要继续增加更多前置 LLM 分类器。
5. 不要让 `chat` 继续当 act 任务的默认收尾必经步骤。

## 最终目标

当以下条件都满足时，可以认为改造方向正确：

1. 普通请求默认进 agent loop，而不是前置多段分类链。
2. 简单请求的 LLM 次数明显下降。
3. `chat` 只在需要表达时介入。
4. 已有真实结果时能直接交付。
5. 用户侧不再看到重复结果。
6. 短补充句不再轻易误绑到旧失败任务。
7. 排障时可以清楚区分决策、执行、交付三层。
