# RustClaw Code Cleanup Follow-up Plan / 代码清理后续计划

状态：进行中
创建日期：2026-06-21

## 背景

本计划承接当前 Codex/Claude 风格 agent loop 收敛后的代码清理工作。最新扫描基线：

- `python3 scripts/check_long_files.py`：通过，当前没有超长文件债务。
- `python3 scripts/check_legacy_route_boundary.py`：`findings=0`。
- `python3 scripts/check_no_nl_hardmatch.py`：`unknown=0 known_legacy=0`。
- `python3 scripts/check_no_runtime_hard_reply.py`：`candidates=0`。
- `plan/` 根目录当前只放未完成计划；完成后归档到 `plan/archived_completed_20260620/` 或后续对应归档目录。

清理目标不是扩大重构面，而是减少旧迁移残留、兼容命名、dead-code allow 和 planner 后处理补丁。所有改动必须继续满足多语言 agent 约束：不新增自然语言硬匹配，不新增硬编码用户可见回复模板。

## 总原则

- [ ] 不为单个中文/英文/日文/韩文自然语言样例增加 `contains`、短语数组或语言分支。
- [ ] 生产代码只消费机器字段、enum、schema、capability、locator、field path、status code、message key。
- [ ] 用户可见自然语言由 finalizer / LLM / i18n 生成；runtime 只输出结构化事实和 evidence。
- [ ] 生产代码和测试代码保持独立；新增测试放 sibling `*_tests.rs` 或专属测试模块，不把大段测试塞回生产模块。
- [ ] 拆分文件按功能命名，禁止使用 `split_1`、`part2`、编号式临时命名。
- [ ] 单文件超过约 1,500 行优先拆分，硬上限不超过 2,000 行；已有大文件只允许小修或净减少。
- [ ] 每个小批次修改后运行对应 focused tests 和门禁，再 git add / commit / push。

## Track A: `dead_code` allow 清理

目标：删除真正无用的 `#[allow(dead_code)]`，把仍需保留的 trace / journal / reload 占位改成明确用途或缩小可见面。

- [ ] 审核 `crates/clawd/src/task_context_builder.rs`
  - `PlannerContextView`
  - `TaskContextBundle`
  - 判断是否可以通过实际调用消除 `allow(dead_code)`，或拆出只供 journal summary 使用的轻量结构。
- [ ] 审核 `crates/clawd/src/task_journal.rs`
  - `TaskJournalFinalizerStage`
  - `TaskJournalFinalizerFallback`
  - `TaskJournalVerifyIssue`
  - `TaskJournalVerifySummary`
  - `TaskJournalRoundTrace`
  - `TaskJournalStepTrace`
  - `TaskJournalFinalizerSummary`
  - `TaskJournalAnswerVerifierSummary`
  - `TaskJournalTaskMetrics`
  - `TaskJournal`
  - 对确实只用于 JSON trace 的结构，保留但补充机器用途说明；对不再写入的字段删除。
- [ ] 审核 `crates/clawd/src/runtime/state.rs`
  - `ReloadContext` 中只为历史 reload 保留的字段。
  - `note_task_llm_call` / `note_task_llm_elapsed` 旧兼容入口。
  - 若没有调用方，优先删除旧入口；若测试或历史日志需要，改名为 trace/backcompat 明确边界。
- [ ] 审核其他生产 `#[allow(dead_code)]`
  - `output_contract_verifier.rs`
  - `verifier.rs`
  - `post_route_policy.rs`
  - `bootstrap/prompts.rs`
  - `runtime/types.rs`

验收：

- [ ] `rg -n "#\\[allow\\(dead_code\\)\\]" crates/clawd/src crates/claw-core/src crates/skills` 数量减少，剩余项有明确边界说明。
- [ ] `RUSTFLAGS="-D warnings" cargo check -p clawd --all-targets` 通过。

## Track B: planner 旧兼容 rewrite 收敛

目标：减少 `agent_engine/legacy_file_config_capabilities.rs` 中的历史补丁，把仍有价值的逻辑迁到 registry / capability resolver / schema repair / 专属功能模块。

- [ ] 盘点 `normalize_legacy_compatibility_actions()` 内每个 rewrite：
  - registry metadata 已覆盖的，删除 rewrite。
  - schema repair 应负责的，迁到 normalizer schema repair 边界。
  - safety / evidence guard 应负责的，迁到 verifier 或 output contract。
  - 仍需兼容旧 planner 输出的，保留但改名标明 machine-compat，不作为普通语义分类。
- [ ] 优先处理可独立验证的小块：
  - service status -> `service_control`
  - sqlite list/schema/count -> `db_basic`
  - docker readonly -> `docker_basic`
  - archive pack/unpack -> `archive_basic`
  - config guard -> `config_basic`
- [ ] 拆分时按功能命名，例如：
  - `service_status_capability_repair.rs`
  - `sqlite_capability_repair.rs`
  - `archive_capability_repair.rs`
  - `config_guard_capability_repair.rs`
  - 不使用 `split_1.rs`、`legacy_part2.rs` 等编号式命名。
- [ ] 所有保留 rewrite 必须只读机器字段：
  - `semantic_kind`
  - `delivery_intent`
  - `locator_kind`
  - action/tool/capability 名
  - schema 字段
  - path / extension / status code
  - 不读用户自然语言短语。

验收：

- [ ] focused planning tests 覆盖每个迁移小块。
- [ ] `python3 scripts/check_no_nl_hardmatch.py` 通过。
- [ ] `python3 scripts/check_legacy_route_boundary.py` 通过。
- [ ] `cargo test -p clawd <focused_test_name> -- --nocapture` 通过。

## Track C: 旧路由命名和 trace 边界收窄

目标：继续把 `FirstLayerDecision` / `legacy_normalizer_decision` / `legacy_first_layer_decision_for_trace` 限制在 normalizer hint、journal trace 和历史日志读取边界。

- [ ] 审核 `intent_router_route_output.rs` 的 `ask_mode_from_legacy_normalizer_decision()`，确认它是否还能改名为 hint-based 转换，避免暗示旧决策仍是语义权威。
- [ ] 审核 `runtime/ask_mode.rs` 中的 `legacy_route_label_for_trace()` / `legacy_first_layer_decision_for_trace()`，确认调用点只用于 trace / journal。
- [ ] 审核 `task_journal.rs` 中 `old_first_layer_decision` 与 `legacy_first_layer_decision` 输出字段，判断是否可以新增新字段名并保留旧字段只做历史兼容。
- [ ] 保持 `semantic_route_authority` 为当前机器 token；不恢复 `agent_decides_semantic_route` / `agent_decides_migration_class` 运行时配置解析。

验收：

- [ ] `python3 scripts/check_legacy_route_boundary.py` 通过。
- [ ] 旧字段不回流为 agent-loop 控制状态。

## Track D: 文档和配置残留说明清理

目标：让 README / docs / config 的描述与当前代码一致，减少用户误解“旧开关仍可作为新架构配置”。

- [ ] 更新 README 中 release gate 描述：
  - 说明 2100+ 可以由等价覆盖集替代。
  - 当前推荐使用压缩覆盖集做代码推进门槛，完整大集合作为定期回归。
- [ ] 审核 `configs/agent_guard.toml`
  - 保留 `semantic_route_authority` 当前配置说明。
  - 旧 bool 只作为历史说明，不作为推荐配置。
- [ ] 审核 docs：
  - `docs/agent_guard_config_wiring_audit.md`
  - `docs/agent_loop_pre_agent_decision_inventory.md`
  - `docs/agent_upgrade_rollout_guardrails.md`
  - 将已完成项标记为历史状态，未完成项转入本计划或后续专项。
- [ ] 确认 README 三个流程图仍反映当前主路径：
  - API / worker / normalizer / agent loop / finalizer。
  - boundary guard 只做安全、绑定、预算、contract，不做普通语义权威。
  - legacy / compatibility 路径只作为非 eligible、高风险、schedule、delivery、回滚边界。

验收：

- [ ] README 与当前代码主流程一致。
- [ ] 文档不推荐旧 bool 开关作为新架构入口。
- [ ] `git diff --check` 通过。

## Track E: 测试资产和生成物清理

目标：删除不应长期保留的测试生成物，保留可复用 fixtures、case 集合和 release-gate 结果摘要。

- [ ] 扫描 `scripts/nl_suite_logs/`、`logs/agent_rollout_metrics/`、`document/` 下历史测试产物。
- [ ] 区分：
  - release gate 证据：保留。
  - 可复用 NL case / fixture：保留。
  - 临时调试输出、图片、音频、手工试验文件：删除或移入明确 ignored 目录。
- [ ] 不删除用户资料、密钥、运行数据库和当前服务需要的日志。
- [ ] 不提交 secrets、token、私钥。

验收：

- [ ] `git status --short` 中没有无意义测试生成物。
- [ ] `.gitignore` 覆盖新的临时输出位置。

## Track F: 验证策略

每个代码小批次至少运行：

- [ ] `cargo fmt --check`
- [ ] `python3 scripts/check_long_files.py`
- [ ] `RUSTFLAGS="-D warnings" cargo check -p clawd --all-targets`
- [ ] `git diff --check`

涉及 route / normalizer / agent-loop 边界时追加：

- [ ] `python3 scripts/check_no_nl_hardmatch.py`
- [ ] `python3 scripts/check_legacy_route_boundary.py`
- [ ] focused NL：最小精选集，不测 image / audio / voice / X / Twitter live API。

涉及 finalizer / fallback / 用户可见回复路径时追加：

- [ ] `python3 scripts/check_no_runtime_hard_reply.py`
- [ ] 人工检查新增生产字符串是否为用户可见自然语言模板。

涉及 planner rewrite / capability repair 时追加：

- [ ] focused planning unit tests。
- [ ] 1-5 条最小 NL 实测，覆盖对应功能即可；完整 NL 回归放在全部代码清理完成后。

## 完成定义

- [ ] `#[allow(dead_code)]` 明显减少，剩余项都有 trace / schema / compatibility 边界理由。
- [ ] planner 旧兼容 rewrite 被拆分或迁移，`legacy_file_config_capabilities.rs` 职责明显变窄。
- [ ] 旧路由字段只作为 trace / journal / historical fallback，不作为控制状态。
- [ ] README / docs / config 描述和当前代码一致。
- [ ] 没有新增自然语言硬匹配和硬编码用户回复。
- [ ] focused tests、门禁检查和必要 NL 实测通过。
- [ ] 完成后将本计划移入归档目录，并在文件内记录完成 commit、测试命令和 NL 结果。
