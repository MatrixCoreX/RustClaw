# RustClaw Mainstream Agent Contract Matrix Plan / 主流 Agent 契约矩阵改造计划

创建日期：2026-05-23

状态：推进中

最新进展（2026-05-23）：

- 已新增 `configs/task_contract_matrix.toml`，覆盖当前全部 `OutputSemanticKind`。
- 已新增 `crates/clawd/src/contract_matrix.rs`，支持 matrix 加载、版本 hash、semantic/generic profile 匹配、action policy、registry/action 引用自检。
- `TaskContract::from_route_result()` 的 required evidence 已先读 matrix，读不到时才回退旧 hardcoded 映射。
- agent turn analysis prompt 已追加 compact `contract_matrix` 行，包含 matrix version/hash、匹配 contract、required evidence、final answer shape、allowed/forbidden actions。
- skill/tool 执行前已接入 contract action gate：结构化任务选择 forbidden / not-allowed action 会返回 `contract_action_rejected` 结构化错误，进入现有重规划/失败处理链路。
- 已补 route-specific evidence augmentation，保留 `quantity_comparison + path/filename/current_workspace` 对 `exists/kind` 的要求。

## 1. 目标

把 RustClaw 的结构化任务处理方式向主流 agent / tool calling 架构靠拢，同时保留 RustClaw 自己的确定性约束层。

目标不是让模型完全自由选择动作，也不是继续为每个失败 case 手工补分支，而是形成这条主路径：

```text
user intent
  -> intent normalization
  -> task contract
  -> tool policy / contract matrix
  -> planner proposes tool calls
  -> runtime verifies allowed action
  -> tool execution
  -> observation evidence check
  -> final answer shape enforcement
```

核心原则：

- 对外协议靠拢主流：tool registry、JSON schema、tool call、observation、trace、eval。
- 对内行为保持可控：contract matrix 决定允许动作、必需证据、禁止动作和最终回答形状。
- 模型只提出候选计划，代码负责准入和验收。
- MiniMax、OpenAI 或其他模型都只是 planner provider，不应决定系统边界。

## 2. 主流化后的职责划分

### 2.1 模型负责什么

模型负责语义理解和候选动作生成：

- 判断用户想完成什么任务。
- 把自然语言归一为结构化 intent / task contract。
- 在允许的能力集合中提出 tool call。
- 当 verifier 返回缺失证据或执行失败时，重新规划。
- 用观察结果生成自然语言表达，但不能绕过 final answer contract。

### 2.2 Runtime 负责什么

Runtime 负责协议、边界和验收：

- 检查 planner 输出是否符合 JSON schema。
- 根据 contract matrix 判断 action 是否允许。
- 判断参数是否满足 tool schema 和 task contract。
- 执行 tool / skill。
- 检查 observation 是否包含 required evidence。
- 判断最终回答是否满足 final answer shape。
- 记录 trace，给 eval 和问题归因使用。

### 2.3 Contract Matrix 负责什么

Contract matrix 是 RustClaw 的确定性策略层：

```text
semantic_kind
  -> allowed_actions
  -> preferred_actions
  -> forbidden_actions
  -> required_evidence
  -> final_answer_shape
  -> retry_policy
  -> eval_templates
```

它不是 prompt，也不是某个模型的私有规则，而是 runtime、verifier、finalizer、eval generator 共享的机器可读契约。

## 3. 新增核心产物

### 3.1 `configs/task_contract_matrix.toml`

新增统一契约矩阵文件。

示例：

```toml
[[contracts]]
semantic_kind = "existence_with_path"
description = "判断文件或目录是否存在"

intent_signals = ["target_path"]

preferred_actions = ["fs_basic.stat_paths"]
allowed_actions = ["fs_basic.stat_paths", "fs_basic.find_entries"]
forbidden_actions = ["fs_basic.read_text_range", "doc_parse.parse_doc"]

required_evidence = ["path", "exists"]
final_answer_shape = "scalar_existence"

retry_policy = "repair_missing_evidence"
missing_target_behavior = "clarify"

[[contracts.eval_templates]]
name = "missing_file_scalar"
expected_action = "fs_basic.stat_paths"
expected_evidence = ["path", "exists"]
expected_answer_shape = "scalar_existence"
```

### 3.2 Tool Registry 规范化

继续复用 `configs/skills_registry.toml`，但让它更像主流 tool registry：

- 每个 action 有稳定 name。
- 每个 action 有 input schema。
- 每个 action 有 output evidence schema。
- 每个 action 标注 risk/effect。
- 每个 action 标注 planner kind：`tool | skill | workflow`。
- 每个 action 可被 contract matrix 引用。

### 3.3 Trace / Eval 统一格式

每次结构化任务记录统一 trace：

```json
{
  "semantic_kind": "existence_with_path",
  "provider": "minimax",
  "planned_actions": ["doc_parse.parse_doc"],
  "selected_action": "doc_parse.parse_doc",
  "policy_result": "rejected",
  "rejection_reason": "forbidden_action",
  "required_evidence": {"all_of": ["target_path"], "one_of": ["exists_true", "exists_false"]},
  "observed_evidence": [],
  "final_shape": "scalar_existence",
  "runtime_snapshot": {
    "registry_hash": "…",
    "matrix_hash": "…",
    "prompt_layer_hash": "…"
  },
  "failure_category": "model_error"
}
```

问题归因固定为：

- `model_error`：模型提出了错误动作，但 policy/verifier 成功拦截。
- `schema_error`：模型输出经 schema 校验/恢复仍不合法，或 schema recovery 改变了语义。
- `code_gap`：contract 允许，代码执行、解析或 finalizer 失败。
- `contract_gap`：matrix 未覆盖该任务或 required evidence 不完整。
- `tool_gap`：缺少可表达该能力的 tool/action。
- `permission_denied`：权限、技能开关、ToolsPolicy、确认策略拒绝执行。
- `budget_exhausted`：agent guard、tool call、round、retry 等运行预算耗尽。
- `prompt_budget_error`：compact contract block 无法安全注入或被截断风险阻断。
- `delivery_error`：文件/媒体/渠道交付失败。
- `provider_error`：模型接口、超时、格式损坏等 provider 问题。

## 4. 当前代码落点

这次改造不应另起一套抽象，而应该沿着现有 RustClaw 代码继续收敛。

### 4.1 现有合同入口

当前结构化合同已经有雏形：

- `crates/clawd/src/pipeline_types.rs`
  - `OutputSemanticKind` 当前有 40 个语义类型。
  - `IntentOutputContract` 已包含 `response_shape`、`requires_content_evidence`、`delivery_required`、`locator_kind`、`semantic_kind`、`locator_hint`。
- `crates/clawd/src/task_contract.rs`
  - `TaskContract` 已包含 `intent_kind`、`targets`、`target_object`、`operation`、`evidence_required`、`required_evidence_fields`、`delivery_shape`、`failure_policy`。
  - `required_evidence_fields_for_output_contract()` 已经是 contract matrix 的最小原型。

强化方向：

- `configs/task_contract_matrix.toml` 不替代 `TaskContract`，而是成为 `TaskContract` 的外部化策略来源。
- `TaskContract::from_route_result()` 继续作为运行时聚合入口。
- `required_evidence_fields_for_output_contract()` 先改成读取 matrix；读不到时保留现有 match 作为兼容 fallback。
- 增加覆盖测试：每个 `OutputSemanticKind::as_str()` 都必须能在 matrix 中找到条目，除非显式标记为 `none_passthrough`。

### 4.2 现有 tool/action 来源

当前主流 tool registry 的基础已经在：

- `configs/skills_registry.toml`
  - `fs_basic`、`config_basic`、`process_basic`、`package_manager`、`archive_basic`、`db_basic`、`docker_basic`、`health_check`、`doc_parse`、`transform` 已经按 `planner_kind = "tool"` 暴露。
  - `fs_basic` 已有 `planner_capabilities`、`input_schema`、`semantic_tags` 和 observe/mutate 风险标注。
- `crates/clawd/src/capability_resolver.rs`
  - 已支持 `CallCapability` 到具体 skill/tool/action 的解析。
  - 已有 preferred/risk/planner_kind 排序。
- `crates/clawd/src/virtual_tools.rs`
  - 已把旧 `system_basic`、`fs_search`、`read_file`、`write_file`、`list_dir` 等规范化到 `fs_basic` / `config_basic`。
  - 已支持 `fs_basic.append_text` 这类 planner-facing action 到底层 runner 的 rewrite。

强化方向：

- matrix 中的 action 名称统一使用 planner-facing 名称，例如 `fs_basic.stat_paths`、`fs_basic.list_dir`、`config_basic.read_field`。
- 底层兼容名只出现在 `virtual_tools.rs` / resolver 中，不进入 matrix 主体。
- P1 校验脚本必须检查：matrix 引用的 `skill.action` 是否存在于 `skills_registry.toml` 的 `input_schema.action.enum` 或 `planner_capabilities.action`。

### 4.3 现有 planner 规则来源

当前 planner 收敛逻辑主要在：

- `crates/clawd/src/agent_engine/planning.rs`
  - 已有大量按 `OutputSemanticKind`、`requires_content_evidence`、locator、skill/action 做的 deterministic rewrite。
  - 已有防止无证据 route 用 `doc_parse/read_file` 伪满足的测试。
  - 已有把 planner 引入的 shell 命令改写为 `fs_basic` action 的逻辑，例如 `echo >> file` 到 `fs_basic.append_text`。

强化方向：

- 不再继续把新 case 写进 `planning.rs` 的大 match / if 链。
- 新 case 首先落到 matrix：
  - 如果是 action 选择问题，改 `allowed_actions/preferred_actions/forbidden_actions`。
  - 如果是证据问题，改 `required_evidence`。
  - 如果是输出形状问题，改 `final_answer_shape`。
- `planning.rs` 只保留三类逻辑：
  - schema / alias normalization。
  - legacy action canonicalization。
  - 基于 matrix 的 deterministic repair。

### 4.4 现有 verifier 与 evidence 来源

当前 verifier 分成两层：

- `crates/clawd/src/verifier.rs`
  - 主要做 plan-level 校验：skill 是否可见、参数是否缺失、风险预算、确认、recipe inspect/validate 约束。
- `crates/clawd/src/answer_verifier.rs`
  - 主要做 final answer / evidence completeness 相关验收。
- `crates/clawd/src/agent_engine/observed_output.rs`
  - 实际承载大量 semantic-specific observation 提取和确定性直出。
- `crates/clawd/src/finalize/loop_reply.rs`
  - 负责最终回答收敛，以及缺失目标、scalar、存在性等 fallback。

强化方向：

- `verifier.rs` 接 matrix 的 action policy：这个语义任务能不能执行这个 action。
- `observed_output.rs` 接 matrix 的 evidence extraction：这个 tool 输出能不能提供 required evidence。
- `answer_verifier.rs` 接 matrix 的 evidence completeness：证据够不够进入 final。
- `loop_reply.rs` 接 matrix 的 final answer shape：最终输出必须是什么形状。

### 4.5 现有缺口分类

从当前代码看，最容易继续变成“遇到才修”的缺口有四类：

- semantic kind 到 action policy 分散在 `planning.rs`。
- required evidence 分散在 `task_contract.rs` 和 `observed_output.rs`。
- final shape 分散在 `observed_output.rs`、`answer_verifier.rs`、`loop_reply.rs`。
- eval case 分散在 `plan/*.txt` 和单测里，没有从同一份 contract 生成。

所以本计划的重点不是“新增一个配置文件”本身，而是让这四类分散规则逐步回流到同一个 matrix。

### 4.6 当前 ask 执行链路的真实插入点

当前代码里结构化执行的大致顺序是：

```text
ask_flow / ask_pipeline
  -> post_route_policy
  -> TaskContract::from_route_result()
  -> agent_engine::run_agent_with_tools()
  -> planner output: PlanResult / PlanStep
  -> PlanStep::to_agent_action()
  -> capability_resolver::resolve_agent_actions_for_state()
  -> planning::normalize_planned_actions_with_original_and_context()
  -> verifier::verify_plan()
  -> skill execution / virtual tool rewrite
  -> task_journal push_step_result()
  -> observed_output / finalize_loop_reply
  -> answer_verifier
```

matrix 的插入点应按这个顺序落地：

1. `post_route_policy.rs`：只负责 route 后的 locator / finalize style / content evidence 兜底，不做 action policy。
2. `task_contract.rs`：构造 `TaskContract` 时读取 matrix，补齐 required evidence、operation、target object、delivery shape、failure policy。
3. `capability_resolver.rs`：继续把 `CallCapability` 解析成具体 `tool.action`；matrix policy 必须在解析后检查。
4. `planning.rs`：在 `normalize_planned_actions_with_original_and_context()` 中，`capability_resolver` 之后、legacy rewrite 之前或之后分两次检查：
   - 早期检查：把明显 forbidden 的 action 标成 repair reason，避免后续 rewrite 掩盖模型错误。
   - 晚期检查：所有 rewrite 完成后，对最终 action 做 enforce。
5. `verifier.rs`：在 `verify_plan()` 的 step loop 中加入 matrix policy issue，形成统一 `VerifyIssueKind`。
6. `task_journal.rs`：记录 contract id、policy decision、required evidence、observed evidence。
7. `observed_output.rs`：把成功 step output 归一成 evidence map。
8. `answer_verifier.rs`：优先用 evidence map 做结构化通过 / 缺口判断，LLM verifier 只作为兜底。
9. `finalize/loop_reply.rs`：根据 `final_answer_shape` 做确定性输出；只有 summary/explanation 类 shape 才允许 LLM 组织语言。

### 4.7 建议新增 Rust 数据结构

建议新增模块：

```text
crates/clawd/src/contract_matrix.rs
```

核心结构：

```rust
struct ContractMatrix {
    contracts: BTreeMap<String, SemanticContract>,
}

struct SemanticContract {
    semantic_kind: String,
    operation: Option<TaskOperation>,
    target_object: Option<TaskTargetObject>,
    delivery_shape: Option<TaskDeliveryShape>,
    failure_policy: Option<TaskFailurePolicy>,
    preferred_actions: Vec<ActionRef>,
    allowed_actions: Vec<ActionRef>,
    forbidden_actions: Vec<ActionRef>,
    required_evidence: Vec<EvidenceField>,
    observation_sources: Vec<ObservationSourceContract>,
    final_answer_shape: FinalAnswerShape,
}

struct ActionRef {
    skill: String,
    action: Option<String>,
}

struct ObservationSourceContract {
    action: ActionRef,
    provides: Vec<EvidenceField>,
}
```

`FinalAnswerShape` 建议先定义最小集合：

- `raw`
- `one_sentence`
- `scalar_value`
- `scalar_count`
- `scalar_existence`
- `strict_list`
- `path_list`
- `table`
- `json`
- `file_token`
- `summary`

实现原则：

- `OutputSemanticKind` 继续是入口 enum。
- matrix key 使用 `OutputSemanticKind::as_str()`。
- 第一阶段不删除 `task_contract.rs` 现有 match，matrix 读不到时 fallback。
- 所有 action 先 canonicalize 成 `skill.action` 再查 matrix。

### 4.8 需要新增的 policy 结果类型

建议新增：

```rust
enum ContractPolicyDecision {
    Allowed,
    PreferAlternative { preferred: Vec<ActionRef> },
    RejectedForbidden { action: ActionRef },
    RejectedNotAllowed { action: ActionRef },
    MissingRequiredArg { action: ActionRef, arg: String },
    ContractMissing { semantic_kind: String },
}
```

它要同时服务三处：

- `planning.rs`：作为 planner repair 提示。
- `verifier.rs`：作为 `VerifyIssueKind` / `VerifyIssue`。
- `task_journal.rs`：作为 trace / eval 记录。

不要只返回 bool。否则后续无法区分模型错误、代码缺口、contract 缺口。

### 4.9 进一步优化空间

这次复查后，计划还需要额外约束这些点，避免后续实现时走偏。

#### A. `semantic_kind=none` 不能简单 passthrough

当前代码里 `OutputSemanticKind::None` 不一定是“纯聊天”。它也可能是：

- `requires_content_evidence=true` 的开放式总结 / 解释任务。
- `delivery_required=true` 的文件交付任务。
- route 已经升级到 planner execute，但 semantic kind 尚未细分的任务。

所以 matrix 需要两级匹配：

```text
1. semantic_kind 精确匹配
2. generic profile 匹配：
   semantic_kind=none
   + requires_content_evidence
   + response_shape
   + locator_kind
   + delivery_required
```

建议新增：

```toml
[[generic_profiles]]
profile = "generic_path_content_summary"
semantic_kind = "none"
requires_content_evidence = true
locator_kinds = ["path", "filename", "current_workspace"]
allowed_actions = ["fs_basic.read_text_range", "doc_parse.parse_doc", "fs_basic.list_dir"]
required_evidence = ["content_excerpt"]
final_answer_shape = "summary"
```

`none_passthrough=true` 只能用于真正无证据要求的 direct answer，不应用到所有 `None`。

#### B. 不要过度依赖 `output_schema`

当前 `configs/skills_registry.toml` 里大量 tool 的 `output_schema` 仍是“有 text 字段”。实际 evidence 往往藏在 `text` 里的 JSON、字段、列表或结构化文本中。因此 matrix 不能只读 `output_schema` 推断 evidence。

优化方向：

- 短期：在 matrix 的 `observation_sources` 中显式声明 extractor。
- 中期：逐步让基础 tool 输出机器可读 `extra` 字段。
- 长期：把 `output_schema` 从“有 text”升级为“声明能提供哪些 evidence”。

#### C. Matrix 不替代 execution recipe

`execution_recipe.rs` 已经负责 action effect、inspect-before-mutate、validate-after-mutate 和服务健康验证。matrix 只负责“这个 semantic task 允许哪个 action、需要哪些 evidence、最终回答什么形状”。

建议执行优先级：

```text
schema / capability resolution
  -> contract matrix action policy
  -> verifier risk / confirmation
  -> execution_recipe sequencing
```

#### D. Matrix 必须缓存

不要在每个 task / 每个 step 里重新读 TOML。建议启动时加载到 `AppState` 或懒加载到 `OnceLock`，测试允许从指定路径加载临时 matrix，trace 中记录 `contract_matrix_version` 和文件 hash。

#### E. `TaskJournalStepTrace` 增字段要小心

`TaskJournalStepTrace` 在多个测试和模块里手工构造。新增 `observed_evidence` 等字段时必须给 `Default` 或 builder/helper，避免一次性改大量测试造成噪声。

#### F. Action policy 要支持无 action 的 tool

不是所有 tool 都有 `args.action`，例如 `health_check`。`ActionRef.action: Option<String>` 是必要的。matrix 表达上应同时支持：

```toml
allowed_actions = ["health_check", "process_basic.port_list"]
```

#### G. 失败归因不能只看最终失败

一次任务可能先有 `model_error`，然后 runtime repair 成功。最终用户看到成功，但开发 trace 仍应记录初始错误 action、policy 拦截、repair 后 action 和最终证据满足。这样才能比较 MiniMax / OpenAI 的真实 planner 质量，而不是只看最终成功率。

#### H. Prompt schema 与代码结构要一起管

当前 planner / gate / finalizer 并不是自由 JSON：

- `prompts/schemas/plan_result.schema.json` 对齐 `AgentAction` / `SinglePlanEnvelope`。
- `prompts/schemas/intent_normalizer.schema.json` 对齐 intent normalizer 输出。
- `prompts/schemas/direct_answer_gate.schema.json` 会影响是否跳过 planner、是否升级到执行。
- `prompts/schemas/answer_verifier.schema.json`、`prompts/schemas/finalizer_out.schema.json` 约束 verifier / finalizer 输出。
- `prompts/layers/manifest.toml` 控制 prompt layer / overlay 入口。

所以 matrix 改造不能只改 Rust 代码和 `agent_tool_spec`。如果新增 planner 字段、repair 字段、contract block 或 verifier 字段，必须同步 schema、prompt layer 和已有 drift test。否则 MiniMax 这类 provider 的 schema recovery 会把问题隐藏成“模型偶发输出不稳”。

原则：

- 不把全量 matrix 塞进 prompt。
- 每次只注入当前 task 的 compact contract：allowed actions、required evidence、final shape、forbidden actions、policy reason。
- 如果 `AgentAction` shape 不变，`plan_result.schema.json` 不必改；如果要暴露新字段，schema 和 drift test 必须同改。

#### I. Direct Answer Gate 是一条真实旁路

`ask_flow.rs` 中的 `direct_answer_gate` 会：

- 根据 schema 输出 `DirectAnswerGateOut`。
- 生成或改写 `IntentOutputContract`。
- 允许 direct answer。
- 将部分请求 promote 到 planner execute。
- 对 locatorless / deictic / recent context 场景做特殊处理。

matrix 必须在 `direct_answer_gate` 和 `post_route_policy` 之后重新看最终 route contract。不能只在 intent normalizer 之后读取一次 semantic kind，否则会漏掉“gate 后才升级为结构化执行”的任务。

建议顺序：

```text
intent_normalizer
  -> post_route_policy
  -> direct_answer_gate outcome
  -> final RouteResult / IntentOutputContract
  -> contract_for_route()
  -> planner/action policy
```

#### J. Contract Matrix 不是权限系统

`runtime/policy.rs::ToolsPolicy`、`AppState::task_allows_skill()`、`risk_ceiling`、`requires_confirmation`、`execution_recipe` 已经承担权限、风险和确认职责。matrix 不能反向放权。

约束：

- matrix 允许某个 action，只表示“语义上合适”，不表示“安全上允许执行”。
- `ToolsPolicy` deny / allow、用户角色、技能开关、确认策略仍然优先。
- `run_cmd`、文件写入、删除、配置修改、安装包、服务操作等必须继续经过现有风险层。
- policy trace 需要区分 `contract_rejected` 和 `permission_denied`，不要混成一个失败类别。

#### K. Evidence Trace 必须先做脱敏

如果 `TaskJournalStepTrace` 记录完整 `observed_evidence`，可能把文件内容、配置、token、日志敏感片段写进 trace。计划里必须把 evidence trace 分成“可用于判断”和“可展示/可落盘”两层。

建议字段：

```rust
struct EvidenceTrace {
    field: String,
    present: bool,
    source_step_id: Option<String>,
    source_action: Option<ActionRef>,
    value_kind: String,
    excerpt: Option<String>,
    sha256: Option<String>,
    redacted: bool,
}
```

规则：

- 默认只记录 evidence field、source、kind、hash、短 excerpt。
- `content_excerpt`、`field_value`、命令输出、配置内容要走统一 redactor。
- UI 默认不展示原始 evidence；开发详情也只展示脱敏内容。
- live replay 的最小复现不能写入 secrets、token、私钥或完整用户文件内容。

#### L. 新配置和测试资产要进入打包/CI 视野

新增 `configs/task_contract_matrix.toml` 后，不能只在本地仓库能跑：

- 孤立回归脚本会 `cp -R configs`，需要确认新文件被复制并被 `RUSTCLAW_CONFIG_PATH` 对应 workspace 找到。
- release / install / sync 脚本需要包含新配置文件。
- `plan/` 当前是 ignored 路径，适合保存工作计划，不适合承载 CI 必须读取的 fixture。
- matrix-driven case seed / replay fixture 若要进 CI，应放在可追踪目录，例如 `scripts/nl_tests/fixtures/`、`tests/fixtures/` 或专用 `crates/clawd/tests/fixtures/`。
- 如果需要示例配置或模板，确认 `configs/config_copy/`、安装脚本、release 包里的配置副本也能找到同一份 matrix。

这能避免“本地 100 case 能跑，打包后缺 matrix 文件”的问题。

#### M. Required Evidence 不能长期只用平铺数组

第一版可以用 `required_evidence = ["path", "exists"]` 这类平铺字段启动，但代码里很多任务的满足条件不是简单 all-of：

- “文件是否存在”：`exists=true` 和 `exists=false` 都是有效证据，区别只在最终答案。
- “找不到某文件”：`confirmed_absence` 是有效终态，不应被当成可无限 retry 的缺证据。
- “比较两个目录”：需要两个 target 各自都有 metadata，再加 comparison result。
- “写入并验证”：需要 mutation result + validation evidence。
- “任选一种来源即可”：例如 `fs_basic.read_text_range` 或 `doc_parse.parse_doc` 都能提供 content evidence。

因此 matrix 第二版应支持 evidence expression：

```toml
[contracts.file_existence.required_evidence]
all_of = ["target_path"]
one_of = ["exists_true", "exists_false", "confirmed_absence"]

[contracts.directory_compare.required_evidence]
all_of = ["left_path_metadata", "right_path_metadata", "comparison_summary"]
```

不要让 `required_evidence: Vec<String>` 变成新的 hardcode，只把它当 P1 过渡格式。

#### N. Registry / Matrix / Prompt Layer 必须是同一快照

当前 runtime 同时依赖：

- `configs/skills_registry.toml`
- prompt layer / overlay / vendor patch
- skill enable switches
- 将来新增的 `configs/task_contract_matrix.toml`

外部技能注册后会提示 reload / restart。matrix 接入后，不能出现“registry 已 reload、matrix 还是旧版本”或“prompt 暴露了新 tool、matrix 不认识”的混合状态。

建议：

- `AppState` 中保存 `RuntimeContractSnapshot`。
- snapshot 内含 registry hash、matrix hash、prompt layer manifest hash、loaded_at。
- 每个 task 开始时固定使用一个 snapshot，任务中途不切换。
- reload 失败要整体失败或保持旧 snapshot，不做半更新。

#### O. Evidence 发给 LLM 前必须再过一层隐私与预算控制

`answer_verifier.rs::execution_evidence_prompt_block()` 当前会把 step 的 `output_excerpt` / `error_excerpt` 放入 verifier prompt。matrix 接入 evidence map 后，不能默认把更结构化、更完整的 evidence 发送给外部 provider。

约束：

- 本地 deterministic verifier 优先；只有 summary / explanation 这类复杂判断才调用 LLM verifier。
- 发给 LLM 的 evidence 必须使用 redacted evidence view，而不是 trace 原始对象。
- `field_value`、配置内容、日志、命令输出、文件片段默认按敏感内容处理。
- provider prompt 中记录 `schema_normalized` / `raw_parse_ok`，schema recovery 不应被统计成完全成功。
- compact contract block 必须被放在不可截断区域；如果 prompt budget 不够放入 contract，应 fail closed 或进入 clarify/repair，而不是让模型在无 contract 的情况下执行。

#### P. Matrix Repair 必须受 agent guard 预算约束

`configs/agent_guard.toml` 已经定义 `max_steps`、`max_rounds`、`max_tool_calls`、`repeat_action_limit`、`no_progress_limit`、`answer_verifier_retry_limit` 等预算。matrix policy / evidence retry 不能绕开这些限制。

要求：

- 每次 contract rejection / evidence retry 都进入 attempt ledger。
- retry 时消耗正常 round / tool budget。
- 因 budget 终止时归因为 `budget_exhausted`，不要误判成 provider_error。
- no-progress 判断要理解“同一 action 但不同 evidence target”与“完全重复调用”的区别。
- `max_tool_calls` 被触发时，用户可见回答应说明任务被安全上限截断，而不是继续编造结果。

#### Q. 文件/媒体交付不是普通 final shape

`delivery_utils`、`finalize/helpers.rs`、`channel_send.rs` 已经对 `FILE:`、`IMAGE_FILE:`、微信媒体、不同渠道分片发送做了专门处理。matrix 的 `final_answer_shape=file_token` 不能替代这些 delivery adapter。

matrix 应新增或约定：

- `artifact_kind = "file|image|audio|url|text"`
- `delivery_required = true`
- `delivery_intent = file_single|file_batch|media`
- `channel_visibility = user_visible|trace_only`

执行原则：

- matrix 决定“是否必须产生可交付 artifact evidence”。
- finalizer 负责把 evidence 变成规范 token。
- delivery adapter 负责按 Telegram / WhatsApp / Feishu / Lark / WeChat / UI 能力发送。
- channel 发送失败应归因为 `delivery_error`，不能倒推为 planner/model 失败。

#### R. 外部技能必须有 Matrix Admission

`extension_manager register_external_skill` 当前会写 workspace、registry、config switch，并提示 reload。matrix 方案完成后，外部技能还需要结构化准入：

- `INTERFACE.md` 必须声明 action、参数、错误合同。
- 若该技能要服务结构化任务，必须声明能提供的 evidence fields。
- 注册时生成 matrix stub 或要求开发者补 `matrix_contract` 段。
- 未声明 evidence 的外部技能只能作为 raw skill / unstructured skill 使用，不能被 matrix 当作结构化证据来源。
- smoke test 不只看 `status=ok`，还要验证 `extra` 或 text extractor 能产出声明的 evidence。

这和 AGENTS.md 的“编译通过即可允许启用”不冲突：编译通过表示技能可启用；matrix admission 表示它可被结构化 planner 当作证据来源。

#### S. API / DB / UI 要保持向后兼容和大小可控

任务结果通过 `tasks.result_json` 保存，UI 又会读取其中的 `task_journal.summary/trace`。新增 contract/evidence 字段时要避免两个问题：

- 老任务没有这些字段，UI 不能崩。
- 新 trace 太大，导致 DB、接口响应和浏览器页面变慢。

约束：

- 所有新增 trace 字段都是 optional。
- UI 只依赖 summary 字段，trace detail 延迟展开。
- `result_json` 中 evidence trace 有大小上限，超过时保留 hash/count/truncated 标记。
- `TaskJournal::to_summary_json()` 只放用户/运维快速判断需要的字段；完整 contract detail 放 `trace`。
- API schema / TS 类型要容忍未知字段和字段缺失。

#### T. 多轮上下文、memory、observed facts 要带 scope 与 freshness

RustClaw 有 `conversation_state`、`followup_frame`、`observed_facts`、`memory_trace` 和 active task 续接逻辑。matrix evidence 不能把历史观察无条件当成当前证据。

建议 evidence 增加：

```rust
enum EvidenceScope {
    CurrentStep,
    CurrentTask,
    ActiveTask,
    Conversation,
    LongTermMemory,
}
```

并记录：

- source task id / step id。
- observed_at。
- target binding。
- 是否来自用户确认。

规则：

- 结构化文件/进程/系统状态默认需要 current task 或明确 active task 证据。
- long-term memory 只能做意图/context 辅助，不能直接满足实时系统状态。
- 用户补充 target 后，旧 pending contract 要重新绑定 target，再检查 matrix。

#### U. Action Policy 要分成 action gate 与 arg gate

planner 输出里有时含 `{{s1.output}}` 这类运行时占位符，`arg_resolver` 后参数才完整。matrix policy 需要两层：

```text
schema-normalized planner action
  -> capability / legacy canonicalization
  -> action gate: 这个 semantic 是否允许这个 tool/action
  -> arg_resolver
  -> input_schema validation
  -> arg gate: 参数是否满足 contract target / scope / risk
  -> ToolsPolicy / confirmation / execution_recipe
```

这样可以避免：

- action 对了但 target 错。
- placeholder 未解析就误判参数缺失。
- schema validation 和 contract validation 互相覆盖错误归因。

#### V. 覆盖率要按 matrix cell 计算，而不只是“每轮 100 条”

“100 条未测过 case”只能保证数量，不能保证结构覆盖。主流化后应输出 coverage report：

- semantic kind 覆盖率。
- generic profile 覆盖率。
- action ref 覆盖率。
- required evidence expression 覆盖率。
- final answer shape 覆盖率。
- negative case 覆盖率：forbidden action、missing evidence、permission denied、delivery failure、schema recovery、budget exhausted、prompt budget blocked。

每轮 100 条应从未覆盖 cell 优先采样，而不是随机扩写自然语言。

#### W. `ask`、`run_skill`、scheduled/admin 路径要分清

contract matrix 的主战场是 `ask -> planner -> tool/action -> evidence -> finalizer`。但 RustClaw 还有其他入口：

- `run_skill`：用户或系统直接指定技能，绕过 planner。
- scheduled job：最终还是产生 `ask` 或 `run_skill` task，但 channel / notification 会影响交付。
- admin / maintenance：可能有专用权限和审计要求。

要求：

- `ask` 路径必须走 matrix action / evidence / final shape。
- `run_skill` 路径不强行套 semantic kind，但必须继续遵守 base skill response contract、skill switch、runner protocol、delivery consistency。
- 如果 `run_skill` 输出被用于后续多轮上下文，仍要能抽取 evidence trace，但标记来源是 `direct_run_skill`。
- scheduled task 必须记录同样的 runtime snapshot，避免计划任务与即时任务行为不一致。
- admin / maintenance 路径只接入 trace/evidence 观察，不让 matrix 降低现有权限门槛。

#### X. Hard Rules 和动态规则仍是上层护栏

`configs/hard_rules/`、`configs/agent_guard.toml` 的 dynamic rules、route policy、self-extension policy 是更高层安全和流程控制。matrix 不能弱化这些规则。

约束：

- hard rule 拦截优先于 matrix allow。
- self-extension / temporary fix 可以作为能力补齐路径，但不能因为 matrix 缺 tool 就自动执行高风险脚本。
- matrix 发现 `tool_gap` 时，只能进入 capability gap / self-extension proposal，不应直接生成并执行新能力。
- 所有 self-extension 执行仍需要原有确认、隔离目录、构建/验证/注册流程。

#### Y. Base Skill Response Contract 要成为 Evidence 的来源规范

AGENTS.md 已要求基础 skill 的 `text/extra/error_text` 遵循 `docs/base_skill_response_contract.md`。matrix 接入后，应把这份文档变成 evidence extraction 的源规范：

- 能机器读取的 evidence 优先来自 `extra`。
- `text` 只用于用户可读摘要和 legacy extractor。
- `error_text` 需要结构化 error kind 时，统一转成 retry/failure category。
- 新增/外部 skill 如果声明结构化 evidence，必须在 `INTERFACE.md` 和 base response contract 中说明 `extra` 字段。

## 5. 分阶段改造计划

### P0：现状盘点

状态：已完成（首轮）

目标：

建立 RustClaw 当前结构化任务、tool action、evidence、final shape 的完整清单。

任务：

- [x] 枚举所有 `OutputSemanticKind`。
- [x] 枚举 registry 中的 planner-facing tool / skill action。
- [x] 从 `task_contract.rs` 导出当前 required evidence 映射，作为 matrix 初始值。
- [x] 从 `configs/skills_registry.toml` 导出 planner-facing action 清单，作为 matrix action 校验源。
- [x] 初步区分 planner-facing action 与 legacy/backing tool；matrix 引用会被 registry/action 自检约束。
- [ ] 从 `observed_output.rs` / `loop_reply.rs` 盘点确定性直出规则。
- [ ] 从 `post_route_policy.rs` 盘点哪些 semantic kind 会改变 finalize style 或强制 content evidence。
- [ ] 从 `execution_recipe.rs` 盘点 action effect、inspect-before-mutate、validate-after-mutate 规则，避免 matrix 与 recipe 冲突。
- [ ] 从 `task_journal.rs` 盘点 trace 已有字段，确定新增 contract trace 的最小字段。
- [ ] 从 `ask_flow.rs` 盘点 direct answer gate / promote-to-planner / executionless direct answer 旁路。
- [ ] 从 `runtime/policy.rs`、`runtime/state.rs` 盘点 ToolsPolicy、skill switch、用户权限、确认策略，不让 matrix 与权限层混淆。
- [ ] 从 `agent_guard.toml`、`attempt_ledger.rs` 盘点 retry / max tool calls / no-progress / repeat guard。
- [ ] 从 `configs/hard_rules/` 和 `agent_guard.dynamic_rules` 盘点上层护栏，不让 matrix allow 弱化 hard rule。
- [ ] 从 `delivery_utils/`、`finalize/helpers.rs`、`channel_send.rs` 盘点文件/媒体交付 token 与渠道差异。
- [ ] 从 `extension_manager`、`sync_skill_docs.py`、`INTERFACE.md` 盘点外部技能 matrix admission 入口。
- [ ] 从 `docs/base_skill_response_contract.md` 盘点基础 skill 的 `extra` evidence 字段规范。
- [ ] 从 `answer_verifier.rs::execution_evidence_prompt_block()` 盘点哪些 evidence 会发送给 provider。
- [ ] 从 `tasks.result_json`、`TaskJournal::attach_to_result()`、UI task detail 盘点 trace 体积和向后兼容边界。
- [ ] 从 `conversation_state`、`followup_frame`、`observed_facts`、`memory_trace` 盘点历史证据的 scope/freshness 规则。
- [ ] 从 `prompts/schemas/*.schema.json` 与对应 drift test 盘点 planner / gate / verifier / finalizer 的 schema 约束。
- [ ] 从 `prompts/layers/manifest.toml`、overlays、vendor patches 盘点 contract block 应注入的位置。
- [ ] 从 `scripts/`、安装脚本、release/sync 脚本盘点新增 matrix 配置和 fixture 的打包路径。
- [ ] 标记每个 action 的输入参数、输出 evidence、风险等级。
- [ ] 盘点当前 verifier / finalizer / observed output 中分散的结构化规则。
- [ ] 区分 `ask` planner 路径、`run_skill` 直接路径、scheduled task、admin/maintenance 路径的适用范围。
- [ ] 标记仍依赖 prompt 或 legacy rewrite 的路径。

产物：

- `plan/contract_matrix_inventory_20260523.md`
- 第一版 semantic kind 到 action 的映射草表。
- 第一版 semantic kind 到 required evidence / final shape 的映射草表。
- 第一版 legacy rewrite 分类表：`keep_as_schema_alias`、`move_to_matrix_policy`、`move_to_observation_source`、`delete_after_eval_green`。

### P1：建立 Contract Matrix 文件

状态：部分完成（P1 可运行底座已落地）

目标：

新增 `configs/task_contract_matrix.toml`，先作为只读规范和测试输入，不立刻重构 runtime 主路径。

任务：

- [x] 为每个 semantic kind 写入 contract。
- [x] 定义统一字段：`preferred_actions`、`allowed_actions`、`forbidden_actions`、`required_evidence`、`final_answer_shape`。
- [x] 增加 `operation`、`target_object`、`delivery_shape`、`failure_policy` 字段，对齐 `TaskContract` 现有结构。
- [ ] 增加 `observation_sources` 字段，标记哪些 tool 输出可以提供哪些 evidence。
- [ ] 增加 `evidence_expression`，支持 `all_of` / `one_of` / `any_of` / `negative_evidence`，平铺 `required_evidence` 只作为过渡。
- [ ] 增加 `evidence_scope` / `freshness`，区分 current step / current task / active task / conversation / long-term memory。
- [ ] 增加 `artifact_kind` / `channel_visibility`，把文件、图片、音频、URL 交付和普通文本 final shape 分开。
- [ ] 增加 `policy_mode = "observe|enforce"`，先观察再强制。
- [x] 增加 `schema_version` / `matrix_version`，并提供 matrix hash。
- [ ] 增加 `runtime_snapshot` 元数据：registry hash、matrix hash、prompt layer hash。
- [ ] 增加 `trace_policy` 或等价字段，声明 evidence 是否只记录 kind/hash/excerpt，默认禁止完整落盘。
- [x] 增加 `none_passthrough = true`，只允许 `semantic_kind=none` 这种无固定结构的合同显式绕过。
- [x] 增加 `generic_profiles`，覆盖 `semantic_kind=none` 但仍需要 content evidence / delivery 的任务。
- [ ] 为文件、进程、端口、包管理器、数据库、HTTP/health、workspace、JSON/CSV/table、memory、多轮引用分别补 contract。
- [ ] 为 `run_skill` 直接路径定义最小 trace/evidence 观察规范，但不强行套 semantic matrix。
- [x] 增加 matrix shape 校验，确保 contract 字段完整。
- [x] 增加 enum 覆盖检查，防止新增 semantic kind 没有 contract。
- [x] 增加 registry action 覆盖检查，防止 matrix 引用不存在的 `skill.action`。
- [ ] 增加 base skill response contract 覆盖检查，确保声明的 evidence 有 `extra` 字段或明确 legacy text extractor。
- [x] 增加 backing tool 禁用检查：matrix 主体不得引用 `system_basic`、`fs_search`、`read_file`、`list_dir`、`write_file` 这类 legacy/backing 名，除非放在 compatibility section。

验收：

- [x] matrix 覆盖全部已知 semantic kind。
- [x] matrix 中引用的 action 都能在 registry 中找到。
- [ ] matrix 可以表达“缺失/不存在也是有效证据”的终态。
- [x] `TaskContract::from_route_result()` 生成的 required evidence 与 matrix 一致。
- [ ] matrix 加载失败时 clawd 明确报配置错误；开发测试可走 fallback，但 release 不应静默忽略。
- [x] matrix 使用 `OnceLock` 缓存，prompt contract line 记录 version / hash。
- [ ] release / install / isolated regression workspace 能读取同一份 matrix 配置，必要时同步到 `configs/config_copy/`。
- [x] `cargo check -p clawd` 通过。

### P1.5：新增 matrix loader 与校验测试

状态：部分完成（loader、prompt 注入、执行前 action gate 已落地）

目标：

先把 matrix 作为可测试的配置资产接入，不改变线上行为。

任务：

- [x] 新增 `contract_matrix.rs`，支持从 `configs/task_contract_matrix.toml` 加载。
- [x] 提供 `semantic_contract(OutputSemanticKind)`。
- [x] 提供 `match_output_contract()`，用于处理 `semantic_kind=none` 的 generic profile。
- [ ] 在 `direct_answer_gate` / `post_route_policy` 后使用最终 `RouteResult` 做 `contract_for_route()`，不要使用过早的 semantic snapshot。
- [x] 提供 `ActionRef::from_skill_args()`，统一抽取 `skill.action`。
- [x] 提供 `action_policy_for_output_contract()`，用于 action gate。
- [ ] 提供 `arg_policy_decision(contract, resolved_args)`，用于 arg gate。
- [x] 提供 `required_evidence_for_output_contract()`，供 `TaskContract` 主路径和测试使用。
- [x] 提供 `matrix_version_hash()`，用于 prompt trace / 后续 eval。
- [ ] 提供 `runtime_contract_snapshot()`，把 registry/matrix/prompt layer 版本绑定在一个 task 上。
- [x] 新增 unit test：全部 `OutputSemanticKind` 覆盖。
- [x] 新增 unit test：`None + requires_content_evidence=true` 能命中 generic profile，不会误判 passthrough。
- [x] 新增 unit test：全部 action ref 可在 registry 中解析。
- [x] 新增 unit test：`task_contract.rs` fallback required evidence 与 matrix 一致。
- [ ] 新增 unit test：matrix schema 合法、prompt/schema drift test 不被新增字段破坏。
- [ ] 新增 unit test：matrix 允许的 action 仍会被 ToolsPolicy / skill switch 拦截时归因为 permission/policy，而不是 contract 通过即执行。
- [ ] 新增 unit test：占位符参数在 arg_resolver 前不误判，arg_resolver 后再检查 target/scope。
- [ ] 新增 unit test：prompt budget 不足以注入 compact contract 时 fail closed。

验收：

- [ ] 不改变 runtime 行为。
- [x] 所有已落地校验在 `cargo test -p clawd contract_matrix` 中可运行。
- [x] 失败信息能直接指出缺哪个 semantic kind 或哪个 action。

### P2：测试生成器接入 Matrix

状态：部分完成（matrix-driven contract case generator 已落地；live NL / MiniMax replay 未完成）

目标：

先让测试体系主流化：从 contract matrix 生成 eval case，而不是人工堆自然语言 case。

任务：

- [x] 新增 matrix-driven contract case generator，先在 `contract_matrix.rs` 单测里从 matrix 自动展开结构化 contract case。
- [x] 每轮可生成 100 条以上不重复 contract path case；当前覆盖 semantic / generic / allowed action / negative action / evidence / final shape。
- [x] 新增 `scripts/nl_tests/generate_contract_matrix_cases.py`，可输出 100 条 JSONL contract seed，并支持 `--batch` 轮转非强制 case。
- [x] case 覆盖全部已声明 `OutputSemanticKind` 和全部 `generic_profiles`，不是只测记忆链路。
- [x] 每条 generated contract case 断言 action policy、required evidence、final shape。
- [ ] 升级为 matrix-driven NL case generator，将 contract case 映射成可 replay 的自然语言输入。
- [ ] 每轮生成 100 条未测过 live NL case。
- [ ] 每条 live NL case 断言 plan/action/evidence/final shape。
- [ ] 每条 case 记录 provider、semantic kind、action、证据、最终答案。
- [x] 生成首版 coverage report：semantic kind、generic profile、phase、policy decision。
- [x] 100 case 采样先保留 semantic/generic coverage，再轮转非强制 action case。
- [ ] 把现有 `plan/nl_100_case_matrix_*`、`plan/nl_builtin_tool_100_cases_*`、`plan/structured_nl_contract_convergence_cases_*` 迁移成 generator seed，而不是继续手写扩展。
- [x] CI 需要复用的 generator 放到 tracked `scripts/nl_tests/`，不依赖 ignored 的 `plan/`。
- [x] generator 输出 case 时附带可追溯的 semantic/generic contract id 和 `expected_policy_decision`。
- [x] generator 输出 `matrix_hash` 到外部 replay JSONL，避免新旧 matrix 混用导致结果不可解释。
- [ ] replay 结果写入 task journal trace，便于自动分类失败。

覆盖轮转：

- 记忆链路。
- 文件读、写、追加、删除、存在性、目录枚举。
- 内置 tool。
- 进程、端口、系统状态。
- package manager。
- database。
- HTTP / health。
- workspace 状态。
- JSON / CSV / table 结构化转换。
- 多轮上下文引用。

验收：

- [x] 能稳定生成 100 条以上未重复 contract case。
- [x] 每条 generated contract case 都能追溯到 matrix contract。
- [ ] 能稳定生成 100 条未重复 live NL case。
- [ ] 失败能归因为 `model_error | schema_error | code_gap | contract_gap | tool_gap | permission_denied | budget_exhausted | prompt_budget_error | delivery_error | provider_error`。
- [ ] MiniMax live replay 的失败能自动写出最小复现：request、route contract、planned action、observed evidence、final answer。
- [ ] negative cases 覆盖 forbidden action、missing evidence、permission denied、delivery error、schema recovery、budget exhausted、prompt budget blocked。

### P3：Policy Gate 接入 Matrix

状态：待开始

目标：

把 contract matrix 变成 runtime action policy。Planner 可以提出候选动作，但执行前必须过 policy gate。

任务：

- [x] 在 planner normalization / capability resolution 之后、实际执行之前新增 action policy check。
- [x] policy gate 使用 loop 上最终 `output_contract`。
- [x] action gate 在 skill/legacy canonicalization 后执行；arg gate 仍待接入。
- [x] 根据 `semantic_kind` 查 matrix。
- [x] 拒绝 `forbidden_actions`。
- [x] 对不在 `allowed_actions` 的 action 返回结构化错误，不直接执行。
- [ ] 如果有 `preferred_actions`，优先提示 planner 改用 preferred action。
- [x] policy rejection 写入 attempt ledger，并在 `TaskJournal::to_trace_json()` 的 step 里暴露 `error_kind`、`failure_attribution`、`contract_policy`。
- [ ] 将 `planning.rs` 中已有的 action rewrite 逐步改为读取 matrix 的 deterministic repair。
- [ ] 保留 `virtual_tools.rs` 的 legacy canonicalization，作为 action policy 前置标准化层。
- [ ] 新增 `VerifyIssueKind::ContractPolicyViolation`、`ContractMissing`、`ContractPreferredActionAvailable`。
- [ ] `verifier.rs` 的 observe mode 先只记录 shadow issue；enforce mode 才阻断。
- [ ] 对 `PlanStep::action_type=call_capability`，必须先经过 `capability_resolver`；未解析能力继续走现有 `CapabilityUnavailable`。
- [x] policy 不负责确认风险，风险仍由现有 `risk_ceiling`、`requires_confirmation`、`execution_recipe` 处理。
- [x] policy 不绕过 `ToolsPolicy`、技能开关、角色权限；当前实现只做额外 preflight 拦截。
- [ ] contract rejection / evidence retry 计入 agent guard 的 round、tool call、no-progress、repeat budgets。
- [ ] 因预算终止时归因为 `budget_exhausted`。
- [ ] hard_rules / dynamic_rules / self_extension policy 的拒绝优先于 matrix allow。

验收：

- [ ] 文件存在性任务不能通过 `doc_parse.parse_doc` 满足。
- [ ] 删除任务不能被 read/list 伪满足。
- [ ] 结构化字段读取不能退化成全文读取后自由合成。
- [x] policy 拒绝能进入现有 skill failure / replanning 链路。
- [x] 对有结构化 contract 的任务，`run_cmd` 只有在 matrix 明确允许时才可执行；无结构化合同的自由任务不受误伤。
- [x] policy issue 能进入 `to_trace_json()` 的 step trace；`TaskJournalVerifySummary` 专用 issue kind 仍待补。
- [ ] 即使 repair 成功，trace 也保留被拦截的初始错误 action。

### P4：Evidence Verifier 接入 Matrix

状态：待开始

目标：

把“工具执行成功”与“任务证据满足”分开。Tool 成功不等于任务完成，必须满足 required evidence。

任务：

- [ ] observation 标准化为 evidence map。
- [ ] verifier 根据 matrix 检查 evidence expression。
- [ ] 缺少 evidence 时返回结构化 repair reason。
- [ ] 对 scalar / count / existence / list / field_value 等输出提供确定性直出条件。
- [ ] 将旧的分散 evidence 规则逐步迁移到 matrix。
- [ ] 将 `observed_output.rs` 中的 semantic-specific extractor 标注为 matrix `observation_sources`。
- [ ] 将 `system_basic` / `fs_basic` / `config_basic` / `db_basic` / `process_basic` / `package_manager` 输出统一归一成 evidence map。
- [ ] 对 `content_excerpt` 和 `field_value` 严格区分：需要字段值时不能用全文摘要代替。
- [ ] `TaskJournalStepTrace` 增加可选 `observed_evidence`，不要只保存 `output_excerpt`。
- [ ] `observed_evidence` 落 trace 前必须脱敏、截断、可选 hash，不能保存完整敏感内容。
- [ ] 发送给 LLM verifier 的 evidence 使用 provider-safe redacted view，本地 trace view 与 provider view 分离。
- [ ] 新增或复用 builder/helper 构造 `TaskJournalStepTrace`，避免手工构造点大面积破坏。
- [ ] `answer_verifier.rs::structurally_satisfies_answer_contract()` 优先读取 evidence map；LLM verifier 只判断复杂 summary 类。
- [ ] 对 `StepExecutionResult` 的 JSON 输出建立 extractor registry：每个 `observation_sources` 映射到一个 extractor。
- [ ] 对无法机器读取 `extra` 的 tool，建立 text extractor 过渡层，并在 matrix 里标记为 `extractor_kind = "text_legacy"`。

验收：

- [ ] 所有结构化任务都能看到 required evidence 和 observed evidence。
- [ ] 证据不足不会进入最终回答。
- [ ] 可恢复失败会进入 retry。
- [ ] 不可恢复失败给用户明确原因和下一步。
- [ ] `requires_content_evidence=false` 的 route 不会被 read/doc_parse observation 伪装成完成。
- [ ] `requires_content_evidence=true` 的 route 不会被 respond/synthesize 空回答伪装成完成。
- [ ] `answer_verifier_skipped_structural_satisfaction` 只能发生在 required evidence 完整时。

### P5：Finalizer 接入 Matrix

状态：待开始

目标：

最终答案形状由 matrix 决定，不由模型自由发挥。

任务：

- [ ] 定义 `final_answer_shape` 枚举。
- [ ] 定义 `artifact_kind` 与 delivery token handler，不把文件/媒体交付混成普通文本 shape。
- [ ] 将 scalar existence、scalar count、one sentence、strict list、table、json object 等形状标准化。
- [ ] finalizer 根据 shape 和 evidence 生成确定性回答。
- [ ] 只在开放式说明、总结、解释类任务中允许模型组织语言。
- [ ] 对中文用户保持稳定、直接、非技术化表达。
- [ ] language_policy 只能选择用户语言和表达语气，不能破坏 `scalar_value`、`strict_list`、`json`、`file_token` 等严格 shape。
- [ ] 将 `loop_reply.rs` 中的 missing target / scalar / existence fallback 迁移为 matrix shape handler。
- [ ] 将 `observed_output.rs` 中已有的 direct answer candidate 限定为 evidence + shape 双满足时才可直出。
- [ ] 对 `OutputResponseShape::Scalar` 增加强断言：不能附加解释、不能输出内部失败模板。
- [ ] 将 `post_route_policy::content_evidence_execution_finalize_style()` 与 matrix final shape 对齐：scalar/file token 走 plain，summary/explanation 走 chat-wrapped。
- [ ] `FinalizerDisposition::QualifiedCompletion` 必须记录使用的 `final_answer_shape`。
- [ ] 文件/媒体交付失败归因为 `delivery_error`，不污染 planner/model 归因。

验收：

- [ ] “是否存在”类任务只输出存在性，不暴露内部流程。
- [ ] “只输出值”类任务不附加解释。
- [ ] “列出文件名”类任务不夹杂不存在的文件。
- [ ] final answer shape 测试可自动断言。

### P6：Planner / Prompt 主流化

状态：待开始

目标：

让 planner prompt 和 action 输出更接近主流 tool calling，而不是 RustClaw 私有混杂格式。

任务：

- [ ] Planner 输出统一 tool call schema。
- [ ] Prompt 中明确：先读 task contract，再选择 allowed tool。
- [ ] Prompt 中不放大量 case 修补，只描述原则和 schema。
- [ ] MiniMax / OpenAI provider 只适配 tool call 格式，不承载业务规则。
- [ ] 保留必要 vendor patch，但不复制整套 skill 文档。
- [ ] `CallCapability` 作为主流 planner 输出方向，继续由 `capability_resolver.rs` 解析到具体 tool/action。
- [ ] `CallSkill` / `CallTool` 继续兼容，但新 prompt 优先暴露 capability 或 planner-facing tool action。
- [ ] planner prompt 中加入 compact contract block：allowed actions、required evidence、final shape、forbidden actions。
- [ ] compact contract block 放在不可截断区，并为 prompt truncation 增加测试。
- [ ] repair prompt 中加入 `ContractPolicyDecision`，不要只写自然语言错误。
- [ ] 不在 prompt 里复制 matrix 全量内容；只注入当前 task contract 相关片段。
- [ ] 若 planner 输出字段不变，保持 `prompts/schemas/plan_result.schema.json` 稳定；若新增字段，同步 schema 与 `plan_result_schema_drift`。
- [ ] direct answer gate、answer verifier、finalizer 若引用 matrix 字段，同步对应 schema 和 drift test。
- [ ] prompt layer / overlay / vendor patch 只放 provider 适配差异，不放业务规则全量副本。
- [ ] 修改 `prompts/` 下真实 prompt markdown 时，保留 AGENTS.md 要求的 `Multilingual Reinforcement` EOF 区块；说明类 README 除外。
- [ ] prompt 中的语言/语气规则不能覆盖 contract block 中的 final answer shape。

验收：

- [ ] 同一 task contract 在不同 provider 下生成的 action 可比较。
- [ ] provider 差异不会改变 allowed action 集。
- [ ] prompt 瘦身后 NL hardmatch scan 仍为 0。

### P7：Trace、Dashboard 与用户可解释性

状态：部分完成（contract rejection step trace 已落地；UI 和完整 evidence trace 未完成）

目标：

对开发者可追踪，对普通用户不暴露复杂度。

任务：

- [x] trace 对 contract action rejection 记录 contract/action/final shape/失败归因。
- [ ] trace 对所有成功/失败 step 记录 contract、action、evidence、final shape、失败归因。
- [ ] trace 记录 runtime snapshot：registry hash、matrix hash、prompt layer hash。
- [ ] UI 默认显示简洁状态：已完成、需要确认、失败并给下一步。
- [ ] 原始 JSON、trace、provider 输出放到二级详情。
- [ ] 失败时把技术原因转成人能理解的下一步。
- [ ] `TaskJournal::to_trace_json()` 增加完整 task-level contract snapshot：
  - `contract_matrix_version`
  - `contract_matrix_hash`
  - `semantic_contract`
  - `policy_decisions`（contract rejection step 已有局部字段）
  - `required_evidence`
  - `observed_evidence`
  - `final_answer_shape`（contract rejection step 已有局部字段）
  - `failure_category`（contract rejection step 已有 `failure_attribution`）
- [ ] 普通用户 UI 不展示 `model_error/code_gap/contract_gap` 术语，只展示下一步；开发者详情里展示分类。
- [ ] `observed_evidence` 默认只展示 field/source/kind/短 excerpt/hash，完整原始输出仍留在受控日志或不落盘。
- [ ] summary 与 trace 分层：summary 小而稳定，trace 可延迟展开。
- [ ] UI / API 对新增字段和缺失字段保持兼容，老任务结果仍可打开。
- [ ] `tasks.result_json` 中 contract/evidence trace 有大小上限，超过时保留 truncated/hash/count。

验收：

- [ ] 普通用户能看懂任务是否完成。
- [ ] 开发者能从 trace 判断是模型问题、代码问题还是 contract 缺口。

### P8：清理旧补丁路径

状态：待开始

目标：

等 matrix、policy、verifier、finalizer 稳定后，再删除可替代的 legacy rewrite 和分散 hardcode。

任务：

- [ ] 列出已被 matrix 覆盖的旧分支。
- [ ] 每删除一类旧分支，补一组 matrix eval。
- [ ] 保留 path token、schema enum、error kind、action metadata 等确定性逻辑。
- [ ] 不删除仍承担兼容职责的路径，除非有测试覆盖。
- [ ] 清理前确认 direct-answer、delivery fallback、多轮 followup、external skill 注册这些旁路都有 matrix/eval 覆盖。
- [ ] 清理前确认 `run_skill` 直接路径、scheduled task、admin/maintenance 路径未被误套 planner matrix。

验收：

- [ ] 不新增自然语言硬匹配。
- [ ] 旧 case 和新生成 case 都通过。
- [ ] MiniMax / OpenAI 切换后的失败归因保持一致。

## 6. 完成后能实现什么

计划完成后，RustClaw 会获得一层稳定的结构化任务控制面。

### 6.1 对用户可见的能力

- 文件、目录、进程、端口、包管理器、数据库、Docker、HTTP/health、archive、config、JSON/CSV/table、memory 等结构化任务会有更稳定的工具选择。
- 用户问“是否存在”“只输出字段值”“列出文件名”“统计数量”“检查服务状态”这类任务时，回答会更短、更确定，不再容易混入解释或内部兜底文本。
- 多轮上下文引用会更可靠，因为 contract 会要求目标、证据和输出形状一致。
- 失败时能更明确告诉用户下一步，而不是泛泛地说“无法确认”。
- 切换 MiniMax / OpenAI / 其他模型后，行为差异会明显变小。

### 6.2 对开发者可见的能力

- 每个结构化任务都能看到：semantic kind、allowed action、required evidence、observed evidence、final answer shape。
- 每个任务能看到本次使用的 runtime snapshot，避免 registry / matrix / prompt 版本混用。
- 每轮 100 条测试能从 matrix 自动生成，不再主要靠人工追加 case。
- 测试不只统计数量，还能显示 matrix cell 覆盖率。
- 失败能分类为：
  - `model_error`
  - `schema_error`
  - `code_gap`
  - `contract_gap`
  - `tool_gap`
  - `permission_denied`
  - `budget_exhausted`
  - `prompt_budget_error`
  - `delivery_error`
  - `provider_error`
- 即使最终 repair 成功，也能记录模型第一次选错了哪个 action。
- 新增 tool / skill 时，可以通过 registry + matrix 判断是否已经可被 planner 正确使用。

### 6.3 对系统架构的能力

原来的链路更像：

```text
LLM 判断语义
  -> LLM/代码混合选择工具
  -> 各处 rewrite / fallback 修补
  -> 成功或失败后再补 case
```

完成后变成：

```text
LLM 判断语义并提出候选动作
  -> contract matrix 做 action policy
  -> tool 执行
  -> evidence verifier 检查证据
  -> finalizer 按 shape 输出
  -> eval/trace 归因
```

核心变化：

| 维度 | 原来 | 完成后 |
| --- | --- | --- |
| 工具选择 | 模型 + planner rewrite 分散控制 | matrix 统一定义 allowed/preferred/forbidden actions |
| 证据判断 | 分散在 `task_contract.rs`、`observed_output.rs`、`answer_verifier.rs` | required evidence 由 matrix 统一声明 |
| 最终回答 | finalizer 和 observed fallback 多处判断 | final answer shape 统一约束 |
| 新 case 失败 | 容易补 `planning.rs` / fallback 分支 | 先判断是 contract、tool、code 还是 model 问题 |
| 测试 | 手写 100 条 case 为主 | matrix 自动生成覆盖 case |
| 模型切换 | 模型差异容易变成行为差异 | 模型只是 planner provider，runtime 收敛行为 |
| trace | 看日志才能猜问题 | trace 直接给 contract/action/evidence/failure category |

这不是让 RustClaw 变成“只靠规则的系统”。模型仍然负责理解自然语言和提出计划；区别是 runtime 不再盲信模型，而是像主流成熟 agent 一样，用 schema、policy、evidence、eval 把行为收敛住。

## 7. 与主流 Agent 架构的对应关系

| 主流概念 | RustClaw 对应实现 |
| --- | --- |
| Tool Registry | `configs/skills_registry.toml` + virtual tools |
| Function Schema | action input schema / output evidence schema |
| Tool Policy | `configs/task_contract_matrix.toml` |
| Task Contract | `IntentOutputContract` + `TaskContract` |
| Planner | `agent_engine::planning` + provider adapter |
| Tool Call | planner action JSON |
| Capability Resolution | `capability_resolver.rs` |
| Legacy Canonicalization | `virtual_tools.rs` |
| Observation | skill/tool structured output + `observed_output.rs` |
| Guardrail | policy gate + `verifier.rs` + `answer_verifier.rs` |
| Hard Rules | `configs/hard_rules/` + agent dynamic rules |
| Retry Loop | attempt ledger + planner repair |
| Loop Budget | `agent_guard.toml` + loop guard |
| Evals | matrix-driven 100-case batches |
| Final Shape | `finalize/loop_reply.rs` |
| Delivery Adapter | `delivery_utils` + `channel_send.rs` |
| Direct Skill Protocol | `run_skill` + base skill response contract |
| Runtime Snapshot | registry hash + matrix hash + prompt layer hash |
| Trace | contract/action/evidence/final-shape log |

## 8. 优先级

建议按这个顺序推进：

1. P0：从现有代码导出 inventory，先不要凭空设计。
2. P1：把 `task_contract.rs` 里的 evidence match 外部化成 matrix。
3. P2：测试生成器先读 matrix，每轮 100 条覆盖所有链路。
4. P3：policy gate 接在 planner normalization / capability resolution 之后。
5. P4：`observed_output.rs` 输出 evidence map，`answer_verifier.rs` 校验证据完整。
6. P5：`loop_reply.rs` 接 final shape handler。
7. P6-P8：最后主流化 planner 输出、UI trace，并清理旧补丁。

这个顺序的好处是：先有可观测的契约和测试，再改 runtime 主路径。每一步都能判断失败来源，避免继续变成“遇到一个 case 修一个 case”。

## 9. 完成标准

- [x] 所有 semantic kind 都在 matrix 中有 contract。
- [ ] 所有 contract 引用的 action 都有 schema 和 evidence 定义。
- [ ] 每批可生成 100 条未重复结构化测试。
- [ ] 测试覆盖内置 tool、skill、memory、多轮上下文和结构化转换。
- [x] Policy gate 能拦截 forbidden / not-allowed action。
- [ ] Evidence verifier 能明确缺少哪些证据。
- [ ] Evidence verifier 支持 all_of / one_of / negative evidence，不把 confirmed absence 误判为失败。
- [ ] Finalizer 能按 shape 稳定输出。
- [ ] direct answer gate / post_route_policy 后升级出的执行任务同样受 matrix 约束。
- [x] matrix 不能绕过 ToolsPolicy、技能开关、用户角色、确认策略。
- [ ] matrix repair / evidence retry 不能绕过 agent guard 预算，预算失败归因为 `budget_exhausted`。
- [ ] compact contract block 不会被 prompt truncation 静默裁掉。
- [ ] trace / replay 不落完整敏感 evidence，默认脱敏和截断。
- [ ] 发给 LLM verifier 的 evidence 使用 provider-safe redacted view。
- [ ] prompt schema drift test 覆盖 planner、direct answer gate、answer verifier、finalizer。
- [ ] `configs/task_contract_matrix.toml` 被回归脚本、release/install/sync 路径覆盖。
- [ ] CI 所需 case seed / fixture 放在 tracked 测试目录，不依赖 ignored 的 `plan/`。
- [ ] 外部技能只有在 INTERFACE/registry/matrix admission 都满足时，才可作为结构化 evidence source。
- [ ] `run_skill` 直接路径继续通过 runner protocol/base response contract 验证，并能写入 trace，但不误判为 planner matrix 失败。
- [ ] hard_rules / dynamic_rules / self-extension policy 的拒绝优先级高于 matrix allow。
- [ ] language_policy 不破坏 strict scalar/list/json/file token 输出。
- [ ] UI/API 能打开老任务和新任务，trace 体积可控。
- [ ] 失败可归因到 `model_error | schema_error | code_gap | contract_gap | tool_gap | permission_denied | budget_exhausted | prompt_budget_error | delivery_error | provider_error`。
- [ ] MiniMax / OpenAI provider 切换后，contract 层行为保持一致。
- [ ] `cargo check`、相关 unit test、MiniMax live replay 通过。
