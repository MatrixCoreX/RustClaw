# RustClaw Skill Development Rules / RustClaw 技能开发规则

本文件给所有参与本仓库的 agent 使用。目标是统一技能接入流程，确保“代码上传后编译通过即可允许启用”。
This file is for all agents working in this repository. The goal is to standardize skill integration so that “once code is pushed and compilation passes, the skill can be considered allowed/enabled”.

## Design Context

### Users
- 主要用户是“不懂技术的普通小白”。
  The primary user is a non-technical beginner.
- 他们希望通过可视化界面完成部署后的日常使用、状态查看、渠道接入和基础排障，而不是阅读日志、编辑配置文件或依赖命令行。
  They want to operate RustClaw through a visual console for daily use, status checks, channel setup, and basic troubleshooting instead of reading logs, editing config files, or using the command line.
- UI 应优先降低理解门槛，让用户先建立“我看得懂、我敢点、我不会弄坏”的信心。
  The UI should reduce cognitive load first and build the feeling of “I understand this, I can click this, and I probably will not break it.”

### Brand Personality
- 气质关键词：可靠、克制、简洁简单。
  Personality keywords: reliable, restrained, simple.
- 产品语气应稳定、直接、友好，不卖弄技术感，不制造压迫感。
  The product voice should feel steady, direct, and friendly without showing off technical complexity or creating intimidation.
- 即使底层是 agent runtime / multi-channel / task orchestration，前端表达也应尽量像“清晰的控制面板”，而不是“工程师调试工具”。
  Even though the backend is an agent runtime with multi-channel orchestration, the frontend should feel like a clear control console rather than an engineer-only debugging tool.

### Aesthetic Direction
- 采用双主题。
  Support both light and dark themes.
- 默认设计判断优先服务非技术用户的可读性、层级清晰度和表单易用性，而不是追求“酷炫”或“黑客感”。
  Design decisions should prioritize readability, hierarchy clarity, and approachable forms over “coolness” or “hacker vibes.”
- 避免高饱和霓虹色、大面积纯黑终端风、过强攻击性的红绿对比，以及会让用户联想到复杂运维工具的视觉语言。
  Avoid neon saturation, large pure-black terminal aesthetics, overly aggressive red/green contrast, and visual language that feels like an intimidating ops-only tool.
- 可以保留适度的专业感与系统感，但必须通过明确的分区、卡片层级、状态标签、说明文字来化解技术门槛。
  A measured sense of professionalism and system control is good, but it must be softened through clear sections, card hierarchy, status labels, and supportive copy.

### Design Principles
- 先解释，再操作：重要动作、状态、渠道概念都要先让用户看懂，再让用户点击。
  Explain before action: users should understand important actions, states, and channel concepts before being asked to interact.
- 默认安全且可恢复：危险操作要弱化，关键操作要有明确反馈，失败时要告诉用户下一步。
  Default to safe and recoverable flows: de-emphasize dangerous actions, provide clear feedback for key actions, and always tell the user what to do next when something fails.
- 面向任务，而不是面向实现：页面结构应围绕“我要登录”“我要绑定渠道”“我要看服务是否正常”来组织，而不是围绕底层模块名。
  Organize around user tasks, not implementation details: structure pages around “I want to log in,” “I want to bind a channel,” and “I want to check service health,” not backend module names.
- 渐进披露复杂度：默认只展示最必要的信息，把日志、原始 JSON、底层细节放在第二层。
  Use progressive disclosure: show only the most necessary information by default, with logs, raw JSON, and low-level details in secondary layers.
- 任何新增 UI 改动都要自查：一个从未接触过 RustClaw 的普通用户，第一次打开时能否理解这个页面在做什么、能做什么、下一步该做什么。
  Every new UI change should be checked against this question: can a first-time, non-technical RustClaw user understand what this page does, what it is for, and what to do next?

## 1) Communication Flow / 通讯链路（技能、路由、主程序）

1. 用户请求进入 `clawd`：`POST /v1/tasks`，`kind=ask|run_skill`。
   User requests enter `clawd` via `POST /v1/tasks`, with `kind=ask|run_skill`.
2. `ask` 任务在 `crates/clawd/src/main.rs` 的 `worker_once()` 中执行：
   `ask` tasks are handled in `worker_once()` in `crates/clawd/src/main.rs`:
   - 先做上下文解析与路由（`intent_router`）。
     First resolve context and route mode (`intent_router`).
   - `FirstLayerDecision::PlannerExecute` / `AskMode::planner_execute_*` 时进入 `agent_engine::run_agent_with_tools()`。
     For `FirstLayerDecision::PlannerExecute` / `AskMode::planner_execute_*`, execution enters `agent_engine::run_agent_with_tools()`.
3. `agent_engine` 输出动作 JSON（`call_capability/call_tool/call_skill/synthesize_answer/respond`）；推荐新规划优先输出 `call_capability`，由 runtime resolver 映射到具体 tool/skill。
   `agent_engine` emits action JSON (`call_capability/call_tool/call_skill/synthesize_answer/respond`); new planner-facing flows should prefer `call_capability`, which the runtime resolver maps to concrete tools/skills.
4. 执行前由 `CapabilityResolver` / `PlanVerifier` 做能力解析、可见性、必填参数、风险/效果与确认/验证检查；不要为了单个自然语言 case 在 `clawd` 主流程加按技能名或固定短语的选择分支。
   Before execution, `CapabilityResolver` / `PlanVerifier` handles capability resolution, visibility, required arguments, risk/effect, confirmation, and validation checks; do not add per-skill or fixed-phrase selection branches to the `clawd` main flow for one natural-language case.
5. `call_skill` 通过 `execution_adapters::run_skill()` -> `run_skill_with_runner()`。
   `call_skill` goes through `execution_adapters::run_skill()` -> `run_skill_with_runner()`.
6. `run_skill_with_runner()` 启动 `skill-runner` 子进程，STDIN/STDOUT 传一行 JSON。
   `run_skill_with_runner()` launches `skill-runner`, passing one-line JSON over STDIN/STDOUT.
7. `skill-runner` 根据 `skill_name` 按约定推导具体技能二进制（默认 `foo_bar` -> `target/release/foo-bar-skill`；若 registry 配了 `runner_name` 则优先用它）。
   `skill-runner` derives the concrete skill binary from `skill_name` by convention (default `foo_bar` -> `target/release/foo-bar-skill`; if registry sets `runner_name`, that takes precedence).
8. 技能进程读取请求 JSON，输出响应 JSON，回传 `clawd` 汇总为任务结果。
   The skill process reads request JSON, writes response JSON, and returns it to `clawd` for task result aggregation.

## 2) Skill Process Protocol (Required) / 技能进程协议（必须遵守）

技能二进制是“单行 JSON stdin -> 单行 JSON stdout”模式。
Skill binaries must use “single-line JSON stdin -> single-line JSON stdout”.

- 输入（来自 `skill-runner`）最小字段 / Minimum input fields (from `skill-runner`):
  - `request_id: string`
  - `args: object`
  - `context: object|null`
  - `user_id: i64`
  - `chat_id: i64`
- 输出最小字段 / Minimum output fields:
  - `request_id: string`
  - `status: "ok" | "error"`
  - `text: string`
  - `error_text: string|null`
  - 可选 / optional: `buttons`, `extra`

要求 / Requirements:

- 不允许输出多行或非 JSON。
  Do not output multi-line content or non-JSON.
- 失败必须返回 `status=error` 和可读 `error_text`。
  On failure, return `status=error` and a readable `error_text`.
- 不得阻塞不退出（遵循 `SKILL_TIMEOUT_SECONDS` 预期）。
  Do not hang indefinitely; respect `SKILL_TIMEOUT_SECONDS` expectations.
- 基础 skill 的 `text/extra/error_text` 响应约定、推荐字段名与当前门禁范围，见 [docs/base_skill_response_contract.md](docs/base_skill_response_contract.md)。
  For base-skill response conventions, preferred `extra` field names, and the current gated set, see [docs/base_skill_response_contract.md](docs/base_skill_response_contract.md).

## 3) New Skill Integration Checklist / 新技能接入清单（全部完成才算可用）

新增技能 `foo_bar` 时，必须同时改这些点：
When adding a new skill `foo_bar`, all of the following are required:

外部提交技能（`external_skills/foo_bar`）走 `extension_manager` 时，验证/编译通过后的注册流程必须自动写入工作区、技能 registry，并把 `configs/config.toml` 的 `skill_switches.foo_bar` 记录为 `true`；普通新增 skill 不应再为了接入去改 `clawd` 主流程代码。
For externally submitted skills (`external_skills/foo_bar`) handled by `extension_manager`, registration after validation/compilation must automatically write the workspace entry, skill registry entry, and `configs/config.toml` `skill_switches.foo_bar = true`; normal new skills should not require changes to the `clawd` main flow.

1. 新建 crate：`crates/skills/foo_bar`（二进制名建议 `foo-bar-skill`）。
   Create crate: `crates/skills/foo_bar` (binary name recommended: `foo-bar-skill`).
2. 加入工作区：`Cargo.toml` 的 `[workspace].members`。
   Add to workspace: `[workspace].members` in `Cargo.toml`.
3. 使用约定式 runner 二进制名：默认将 `foo_bar` 编译为 `foo-bar-skill`；只有当二进制名不符合约定时，才在 `configs/skills_registry.toml` 中配置 `runner_name`。
   Use the conventional runner binary name: by default `foo_bar` should compile to `foo-bar-skill`; only configure `runner_name` in `configs/skills_registry.toml` when the binary name does not follow the convention.
4. 注册执行别名（可选但建议）：优先在 `configs/skills_registry.toml` 的 `aliases` 中配置；`crates/clawd/src/main.rs` 的 `canonical_skill_name()` 仅作无 registry 的兼容回退。
   Register aliases (optional but recommended): prefer `aliases` in `configs/skills_registry.toml`; `canonical_skill_name()` in `crates/clawd/src/main.rs` is compatibility fallback only when no registry is used.
5. 如果技能需要进入 planner 常规自然语言执行流，优先在 `configs/skills_registry.toml` 声明 `planner_capabilities`（能力名、action、effect、required/optional、risk_level），让 `call_capability` 通过 resolver/verifier 接入；不要为了接入去改 `clawd` 主流程代码。
   If the skill should be used by the planner for normal natural-language execution, declare `planner_capabilities` in `configs/skills_registry.toml` first (capability name, action, effect, required/optional fields, risk_level) so `call_capability` can flow through resolver/verifier; do not modify the `clawd` main flow just to integrate it.
6. 加入 agent 技能认知 / Add agent skill awareness:
 - `crates/skills/foo_bar/INTERFACE.md`
 - 运行 `python3 scripts/sync_skill_docs.py`，生成/更新 `prompts/layers/generated/skills/foo_bar.md`
- 在 `configs/skills_registry.toml` 中为该技能配置 `prompt_file = "prompts/skills/foo_bar.md"`（逻辑路径；运行时主体读取 `prompts/layers/generated/skills/foo_bar.md`，如有模型差异再叠加 `prompts/layers/vendor_patches/<vendor>/skills/foo_bar.md`）
 - 技能参数契约写入 `INTERFACE.md`，并由 `sync_skill_docs.py` 生成 skill 专属 prompt；不要为单个普通 skill 修改全局 `prompts/layers/overlays/agent_tool_spec.md`
7. 配置基线 / Config baseline:
   - `crates/claw-core/src/config.rs` 的默认 `skills_list`（按需要）
     Default `skills_list` in `crates/claw-core/src/config.rs` (as needed)
   - `configs/config.toml` / `configs/config_copy/*.toml`（按现有规范）
     `configs/config.toml` / `configs/config_copy/*.toml` (follow current conventions)
   - 外部技能通过 `extension_manager register_external_skill` 自动写入 `configs/config.toml` 的 `skill_switches.<skill>=true`；不要再手工维护一套重复开关流程，除非自动化失败需要排障。
     External skills should use `extension_manager register_external_skill` to automatically write `skill_switches.<skill>=true` into `configs/config.toml`; do not maintain a duplicate manual switch flow unless debugging an automation failure.
8. 如果技能需独立配置，补 `configs/*.toml` 与 README 说明，并在 `INTERFACE.md` 里新增 `## Config Entry Points`，写清楚真实配置入口（配置文件 / 环境变量 / 本地数据库或 API / 登录态 / 依赖）。
   If the skill needs dedicated config, add `configs/*.toml` and README docs, and add `## Config Entry Points` to `INTERFACE.md` so the real setup path is explicit (config file / environment variable / local DB or API / login state / dependency).
9. 新技能必须附带接口说明文档，便于生成给 LLM 判断/路由用的技能 md。
   New skills must include an interface spec document so that skill markdown for LLM judgment/routing can be generated reliably.
   - 建议路径 / Recommended path: `crates/skills/<skill>/INTERFACE.md`
   - 最小内容 / Minimum content:
     - 功能概述 / Capability summary
     - `action` 列表 / `action` list
     - 每个 action 的必填参数、可选参数、类型、默认值
       Required/optional params, types, defaults per action
     - 错误码或错误文本约定 / Error contract
     - 2~3 个请求/响应 JSON 示例 / 2-3 request/response JSON examples
   - 若是基础 skill 或计划做机器可读响应，额外参考 [docs/base_skill_response_contract.md](docs/base_skill_response_contract.md)。
     If this is a base skill or is expected to expose machine-readable success payloads, also follow [docs/base_skill_response_contract.md](docs/base_skill_response_contract.md).
10. 使用自动同步脚本维护技能文档：`python3 scripts/sync_skill_docs.py`。
   Use the auto-sync script to maintain skill docs: `python3 scripts/sync_skill_docs.py`.
   - 技能发现目录 / Skill discovery roots:
     - `crates/skills/*`（内建技能 / built-in skills）
     - `external_skills/*`（外部提交技能 / externally submitted skills）
   - 新 skill 目录（`crates/skills/<skill>`）出现时，自动创建：
     - `crates/skills/<skill>/INTERFACE.md`
    - `prompts/layers/generated/skills/<skill>.md`
   - 新外部 skill 目录（`external_skills/<skill>`）出现时，自动创建：
    - `prompts/layers/generated/skills/<skill>.md`（前提：开发者已提供 `external_skills/<skill>/INTERFACE.md`）
   - 对外部技能强制门禁 / Hard gate for external skills:
     - 若缺少 `external_skills/<skill>/INTERFACE.md`，同步脚本会报错并返回非 0（可直接用于 CI 阻断）。
     - If `external_skills/<skill>/INTERFACE.md` is missing, sync exits non-zero and can be used as a CI blocker.
  - skill 目录删除时，自动删除 `prompts/layers/generated/skills/<skill>.md`。
   - skill 仅关闭（`skill_switches=false`）时，不删除任何 md（保持提示词兼容与回滚能力）。
  - `prompts/layers/generated/skills/<skill>.md` 采用受控自动生成模式：包含 `<!-- AUTO-GENERATED: sync_skill_docs.py -->` 标记的文件会在同步时自动更新；无标记文件视为手工维护，不会被覆盖。
    `prompts/layers/generated/skills/<skill>.md` uses controlled auto-generation: files containing `<!-- AUTO-GENERATED: sync_skill_docs.py -->` are updated on sync; files without the marker are treated as manually maintained and are not overwritten.
  - 模型差异若确有必要，只允许放在 `prompts/layers/vendor_patches/<vendor>/skills/<skill>.md`，不要再新增旧的 vendor skill 全量副本。
   - 托管迁移命令 / Adopt commands:
     - `python3 scripts/sync_skill_docs.py --adopt <skill>`：将单个 skill 的 prompt md 迁移为自动托管（覆盖该文件）。
       Migrate one skill prompt into managed mode (overwrites that prompt file).
     - `python3 scripts/sync_skill_docs.py --adopt-all`：将全部 skill prompt md 迁移为自动托管（覆盖全部）。
       Migrate all skill prompts into managed mode (overwrites all prompt files).

## 4) Skill Switch Rules / 技能开关规则（当前仓库约定）

- 运行时允许集由 `[skills].skills_list` + `[skills].skill_switches` 叠加得出。
  Runtime allowed skills are computed from `[skills].skills_list` + `[skills].skill_switches`.
- `skill_switches` 优先级高于 `skills_list` / `skill_switches` has higher priority than `skills_list`:
  - `true`：强制开启 / force enable
  - `false`：强制关闭 / force disable
- 外部技能在验证/编译通过并执行 `register_external_skill` 后，默认自动记录为 `true`；需要停用时再显式改为 `false`。
  External skills are automatically recorded as `true` after validation/compilation and `register_external_skill`; set them to `false` only when explicitly disabling them.
- 关闭技能后 / When a skill is disabled:
  - 二层提示词会显示 disabled 简化提示
    The second-layer prompt uses a disabled simplified hint
  - 命中需求时应回复“技能未开启”
    If user intent requires it, respond with “skill not enabled”
  - 运行时调用会被 `clawd` 拦截
    Runtime invocation is blocked by `clawd`

## 5) Admission Criteria (“compile => allowed”) / “编译即可允许” 的准入标准

PR 合并前至少满足：
Before merge, at least the following must pass:

1. `cargo check -p clawd -p skill-runner -p <new-skill-crate>`
2. 若改了 UI：在 `UI/` 下执行 `npm run lint && npm run build`
   If UI changed: run `npm run lint && npm run build` under `UI/`
3. 能通过 `run_skill` 路径打通（最少一次 happy path）
   End-to-end `run_skill` path must work (at least one happy path):
   - `POST /v1/tasks`，`kind=run_skill`，`payload.skill_name=<skill>`
4. 失败路径有清晰 `error_text`（不允许静默失败）。
   Failure path must return clear `error_text` (no silent failure).
5. 外部技能注册动作必须自动完成配置写入：`configs/config.toml` 中出现 `skill_switches.<skill>=true`，且 registry / workspace 映射完整。
   External skill registration must automatically complete config writing: `configs/config.toml` contains `skill_switches.<skill>=true`, and registry/workspace mappings are complete.

只有当“映射完整 + 编译通过 + 路径可跑通”同时成立，才允许把该技能视为可用。
A skill is considered available only when “mapping complete + compile pass + runnable path” are all satisfied.

## 6) Execution Principles (for agents) / 实施原则（给 agent）

- **`prompts/` 下所有真正的 prompt markdown 文件**在文件末尾保留统一的 **Multilingual Reinforcement** 区块（固定标题与注释模板），用于 zh-CN / en 等语言特有补充；通用规则仍写在正文。说明类 README（如 `prompts/layers/README.md`）不追加该区块，仅文档化规范。
  **All real prompt markdown files under `prompts/`** keep a unified **Multilingual Reinforcement** block at EOF (fixed heading + comment template) for language-specific nuance (e.g. zh-CN / en); general rules stay in the main body. Explainer READMEs (e.g. `prompts/layers/README.md`) do not get the block—document the convention only.
- 优先增量改动，不重构无关模块。
  Prefer incremental changes; avoid unrelated refactors.
- 新增或强化技能选择时，优先改 registry metadata、`INTERFACE.md`、生成提示词或必要的 vendor patch；Rust 主流程只负责协议校验、resolver/verifier、权限安全、runner 派发和输出契约，不承载固定自然语言 case。
  When adding or improving skill selection, prefer registry metadata, `INTERFACE.md`, generated prompts, or necessary vendor patches; Rust main-flow code should own protocol validation, resolver/verifier, safety policy, runner dispatch, and output contracts, not fixed natural-language cases.
- 先补协议与映射，再补提示词与 UI，最后跑编译。
  Implement protocol/mapping first, then prompts/UI, then compile checks.
- 不改已有技能行为，除非需求明确要求。
  Do not change existing skill behavior unless explicitly required.
- 不提交 secrets、token、私钥等内容。
  Never commit secrets, tokens, or private keys.
