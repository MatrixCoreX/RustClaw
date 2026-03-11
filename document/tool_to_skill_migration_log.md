# Tool → Skill 统一改造实施记录

## Phase 1 已完成

### 1. Parser / Normalize（main.rs）

- **normalize_agent_action_shape**：
  - 输入 `{"type":"call_tool","tool":"run_cmd",...}` → 输出 `{"type":"call_skill","skill":"run_cmd",...}`（任意 tool 名均转为 skill）
  - 无 `type` 仅有 `tool` 时，一律输出 `call_skill`（skill = normalized_tool）
  - bare 形态 `{"type":"run_cmd",...}` 用 `is_builtin_skill_name` 判断后输出 `call_skill`
- **is_builtin_skill_name**：包含 run_cmd、read_file、write_file、list_dir，作为统一 builtin skill 白名单。
- **is_builtin_tool_name**：已移除；原逻辑由 `is_builtin_skill_name` 统一承担。

### 2. 执行层（agent_engine.rs）

- **AgentAction::CallTool**：改为 **LEGACY 分支**，仅来自旧 plan/历史；注释标明「仅兼容，normalizer 现已只出 call_skill」。
- 该分支内：参数解析与 path/alias 重写后，统一走 `execution_adapters::run_skill(...)`，与 CallSkill 主路径一致。
- 主执行路径仅为 `AgentAction::CallSkill`；CallTool 不再作为推荐或主干形式。

### 3. Prompt 层统一（本次完成）

- **主 spec**：所有 `agent_tool_spec.md`（根、default、openai、google、claude、qwen、deepseek、grok、minimax）：
  - 不再使用「## Tools」；统一为「## Skills」，其下「### Base (builtin)」列出 read_file、write_file、list_dir、run_cmd。
  - 文案：仅暴露 `call_skill`，明确「Do not use call_tool」「Output only call_skill steps」；「Tool behavior notes」改为「Skill behavior notes (file/path)」。
- **Planner**：所有 `loop_incremental_plan_prompt.md`、`single_plan_execution_prompt.md`（根 + 各 vendor）：
  - AgentAction 只列 `call_skill` 与 `respond`；不再列出 `call_tool` 作为选项。
  - 「run command then save output」等规则改为「call_skill with skill=\"run_cmd\"」。
- **Runtime**：所有 `agent_runtime_prompt.md`：
  - Schema 中移除 `call_tool` 行；仅保留 `think`、`call_skill`、`respond`。
  - 约束 3.x 改为「Use only skills」「Choose call_skill」；不再提及 call_tool 为推荐。

### 4. 能力视图与策略统一

- **Policy 默认**（main.rs）：`coding` 默认 `skill:*`；`minimal` 默认 `skill:read_file`/`skill:list_dir`；policy token 与错误文案统一为 skill。
- **execute_builtin_skill**（原 execute_builtin_tool）：仅用于 base (builtin) skills；命名统一为 builtin skill。
- **agent_engine**：CallTool 的 fingerprint / plan_step_label 统一为 `skill:…`；CallTool 仅 LEGACY COMPATIBILITY。
- **execution_adapters::run_tool**：LEGACY COMPATIBILITY ONLY，主链不调用。

### 5. 配置与策略层收口（本轮）

- **配置校验**：仅接受 `*` 或 `skill:` 前缀；不再把 `tool:` 当作合法前缀。错误提示为「expected '*' or prefix 'skill:'」并注明 legacy `tool:` 在加载时自动转换为 `skill:`。
- **显式兼容转换**：`normalize_capability_pattern()` 在 from_config 时将所有 `tool:*` / `tool:name` 转为 `skill:*` / `skill:name` 再写入 allow/deny，内部与对外语义均不再保留 `tool:`。
- **主配置/策略层**：不再公开接受 `tool:*`；旧配置文件中的 `tool:` 仅在加载时被转换一次。

### 6. 基础能力归属方案（统一）

- **Base (builtin) skills**：`run_cmd`、`read_file`、`write_file`、`list_dir`、`make_dir`、`remove_file` 为正式基础 skill，与 registry / prompt / capability 视图一致。
  - 执行：`run_skill` → `run_skill_with_runner` → `execute_builtin_skill`（进程内执行）。
  - Prompt：`agent_tool_spec.md` 中「## Skills」→「### Base (builtin)」列上述六项；文件系统基础能力全部收口为独立 base skill。
  - 策略：使用 `skill:*` 或 `skill:read_file` 等 token；minimal 策略包含六项 base skill + system_basic。
- **system_basic**：仅保留 **info**（系统自检）；文件/命令/目录能力已全部收口为上述六项独立 base skill，不得用 system_basic 做文件/目录/命令操作。各 vendor 的 system_basic.md 已统一为「system introspection only」，并说明为何不再包含 make_dir/remove_file 等动作。

- **文件系统基础能力收口（make_dir / remove_file）**：make_dir、remove_file 已收口为独立 base skill；builtin 在 `execute_builtin_skill` 中实现；agent_tool_spec 与各 skill 文档中「文件系统基础能力」边界已统一，无悬空归属。

### 7. 当前保留（仅 legacy 兼容）

- **AgentAction::CallTool**：仅用于反序列化旧 JSON；执行时走 legacy 分支并委托 `run_skill`，不参与主链。
- **run_tool**：仅作内部兼容适配器，主链不调用；标注 `#[doc(hidden)]` 与 LEGACY COMPATIBILITY ONLY。
- 旧 `call_tool`：parser 仍接受并归一化为 call_skill；执行通过 legacy 分支，不再作为推荐或主形式。

---

## 验收要点（当前状态）

- 主配置与策略层不再公开接受 `tool:*`；仅接受 `*` 或 `skill:`，旧配置中 `tool:` 在加载时显式转换为 `skill:`。
- 主 prompt / normalizer / capability view 仅使用 skill 语义；基础能力归属方案明确（六项 base builtin skill + system_basic 仅 info）。
- `AgentAction::CallTool` 与 `run_tool` 仅作 legacy compatibility；正常主链仅走统一 skill dispatcher（run_skill → run_skill_with_runner → execute_builtin_skill 或 runner）。
- 不再存在新的主流程强化 tool 概念。

---

## 后续 Phase 建议

### Phase 2：Prompt 与注册表动态化

- 基础能力也有独立 contract 文件，从 registry + prompt 动态加载。

### Phase 3：执行分发与日志统一

- 单一 skill dispatcher；日志/fingerprint 对 legacy CallTool 可统一打为 skill:…。

### Phase 4：移除 CallTool 枚举（可选）

- 若不再需要兼容旧 JSON，可移除 `AgentAction::CallTool`，仅保留 `CallSkill`。

---

## 回归验证建议

- 执行 pwd、ls -l、读取 README、写入 hello.txt、list_dir、查 doge 价格、画图、讲笑话、混合（pwd + 笑话）
- 检查：planner 输出是否均为 `call_skill`；run_cmd/read_file/write_file/list_dir 是否仍正常执行；旧 `call_tool` 输入是否仍被规范并执行
