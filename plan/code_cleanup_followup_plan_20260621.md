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

2026-06-21 代码核对更新：

- `plan/` 根目录当前仅剩本计划，归档目录下计划不计入当前未完成项。
- 新增 `video_generate` / `music_generate` 后已完成真实 dry-run 验证：直接 skill 进程、`skill-runner`、`clawd /v1/tasks kind=run_skill` 均通过；registry reload 曾暴露 `output_kind = "video" / "audio"` 不属于当前 `OutputKind` 枚举，已修为 `file` 并提交 `058314a0`。
- 当前 `#[allow(dead_code)]` 清理已完成：skill 协议字段、trace / journal、runtime reload 兼容快照、contract 旧占位、fallback 旧占位均已改为实际读取、字段收窄、测试专用或物理删除。
- 当前 `rg -n "#\\[allow\\(dead_code\\)\\]" crates/clawd/src crates/claw-core/src crates/skills` 返回空结果；`RUSTFLAGS="-D warnings" cargo check -p clawd --all-targets` 通过。

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

- [x] A0：清理 skill 协议字段上的低风险 `allow(dead_code)`。
  - 2026-06-21：`browser_web`、`extension_manager`、`photo_organize` 的未读协议字段改为 `_field` + `serde(rename=...)`，保持 JSON 协议不变；`photo_organize.context` 保留原名，因为语言解析仍读取它。
  - 2026-06-21：`extension_manager` 生成外部 skill 模板同步改为 `_context/_user_id/_chat_id`，避免新生成代码继续带 dead-code allow。
  - 验证：`cargo fmt --check`、`cargo test -p browser-web-skill`、`cargo check -p extension-manager-skill -p photo-organize-skill`、`python3 scripts/check_long_files.py` 通过。
- [x] A1：清理 clawd 生产代码剩余 `#[allow(dead_code)]`。
  - 2026-06-21：`task_context_builder.rs`、`pipeline_types.rs`、`post_route_policy.rs`、`verifier.rs` 的整块 dead-code 放行已移除；`PlanStep.why` 改为进入 journal plan trace，执行仍只消费机器字段。
  - 2026-06-21：`task_journal.rs` 的 trace / summary 结构整块放行已移除；未构造的旧 finalizer stage / fallback 枚举值和旧 helper 方法已删除；fallback 维持真实 `null` 边界。
  - 2026-06-21：`runtime/state.rs` 中未读 reload 快照字段改为 `_field`，旧 `note_task_llm_call` / `note_task_llm_elapsed` 无调用方，已删除。
  - 2026-06-21：`rss_fetch` 测试 helper 改为 `#[cfg(test)]`；`crypto.require_explicit_send` 配置兼容字段改为 `_require_explicit_send + serde(rename=...)`；`photo_organize` 平台枚举改为目标平台 `cfg`；`finalize/helpers.rs` 测试 schema 未读字段改为 `_field + serde(rename=...)`；`output_contract_verifier.reason_code()` 限定为测试；未使用的 `OutputContractVerdict::label()`、`RepairSignalSource` 占位、`UserResponseKind::LlmUnavailable`、TaskContract 无构造占位已删除。
  - 2026-06-21：`bootstrap/prompts.rs` 的 `PromptReloadReport` 改为运行期 SIGHUP summary 日志实际读取，不再需要 dead-code 放行。
  - 验证：`cargo fmt --check`、`RUSTFLAGS="-D warnings" cargo check -p clawd --all-targets`、`RUSTFLAGS="-D warnings" cargo check -p rss-fetch-skill -p crypto-skill -p photo-organize-skill --all-targets` 通过。
- [x] 审核 `crates/clawd/src/task_context_builder.rs`
  - `PlannerContextView`
  - `TaskContextBundle`
  - 判断是否可以通过实际调用消除 `allow(dead_code)`，或拆出只供 journal summary 使用的轻量结构。
- [x] 审核 `crates/clawd/src/task_journal.rs`
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
- [x] 审核 `crates/clawd/src/runtime/state.rs`
  - `ReloadContext` 中只为历史 reload 保留的字段。
  - `note_task_llm_call` / `note_task_llm_elapsed` 旧兼容入口。
  - 若没有调用方，优先删除旧入口；若测试或历史日志需要，改名为 trace/backcompat 明确边界。
- [x] 审核其他生产 `#[allow(dead_code)]`
  - `crates/skills/rss_fetch/src/main.rs`
  - `crates/skills/photo_organize/src/main.rs` 剩余一处历史辅助函数。
  - `crates/skills/crypto/src/main.rs` 配置兼容字段。
  - `crates/skills/browser_web/src/main.rs`、`crates/skills/extension_manager/src/main.rs` 的协议字段已在 A0 清理。
  - `output_contract_verifier.rs`
  - `verifier.rs`
  - `post_route_policy.rs`
  - `bootstrap/prompts.rs`
  - `runtime/types.rs`
  - `fallback.rs`
  - `repair_signal.rs`
  - `pipeline_types.rs`
  - `worker/ask_pipeline.rs`

验收：

- [x] `rg -n "#\\[allow\\(dead_code\\)\\]" crates/clawd/src crates/claw-core/src crates/skills` 返回空结果。
- [x] `RUSTFLAGS="-D warnings" cargo check -p clawd --all-targets` 通过。

## Track B: planner 旧兼容 rewrite 收敛

目标：减少 `agent_engine/legacy_file_config_capabilities.rs` 中的历史补丁，把仍有价值的逻辑迁到 registry / capability resolver / schema repair / 专属功能模块。

优先级调整：

1. 先只做 inventory 和测试命名拆分，不先删除 rewrite。
2. 再处理能由 registry/schema 明确覆盖的单块 rewrite。
3. 最后才拆/删跨技能兼容路径；每块都必须有 focused planning tests。

- [x] B0：盘点 `normalize_legacy_compatibility_actions()` 内每个 rewrite。
  - 2026-06-21 代码核对：`service status -> service_control` 已在 `scalar_count_explicit_path.rs`，并由 `planning_tests/scalar_count_and_hidden_entries.rs` 覆盖。
  - 2026-06-21 代码核对：`sqlite list/schema/count` 已在 `sqlite_table_listing_rewrite.rs`，并由 `planning_tests` 下 sqlite / config structured field 相关 focused tests 覆盖。
  - 2026-06-21 代码核对：`docker readonly -> docker_basic` 已在 `shell_sequence_part.rs`，不在 legacy 文件主体内。
  - 2026-06-21 代码核对：`archive unpack` 已在 `sqlite_table_listing_rewrite.rs`，`archive pack` 已在 `shell_sequence_part.rs`，`archive schema alias / short archive target` 已在 `runtime_status_scalar_plan.rs`，并由 `planning_tests/delivery_archive_config_edit.rs`、`planning_tests/config_structured_field_reads.rs` 覆盖。
  - 2026-06-21 代码核对：`legacy_file_config_capabilities.rs` 原先主要剩两类职责：normalization 编排入口 `normalize_legacy_compatibility_actions()` / `canonicalize_legacy_file_config_capabilities()`，以及 RustClaw config guard / config validation / config risk 兼容 repair。
- [x] B1：按功能命名拆出 config guard repair。
  - 2026-06-21 已迁到 `crates/clawd/src/agent_engine/config_guard_capability_repair.rs`，覆盖 config validation / config risk assessment / config excerpt / invalid locator repair / guard path helpers。
  - `legacy_file_config_capabilities.rs` 从约 1,206 行降到约 430 行，职责收窄为 legacy canonicalization + compatibility normalization 编排及少量通用 schema alias rewrite。
- [ ] 处理盘点后的剩余 rewrite：
  - registry metadata 已覆盖的，删除 rewrite。
  - schema repair 应负责的，迁到 normalizer schema repair 边界。
  - safety / evidence guard 应负责的，迁到 verifier 或 output contract。
  - 仍需兼容旧 planner 输出的，保留但改名标明 machine-compat，不作为普通语义分类。
- [x] 已拆出并有 focused tests 的小块：
  - service status -> `service_control`
  - sqlite list/schema/count -> `db_basic`
  - docker readonly -> `docker_basic`
  - archive pack/unpack -> `archive_basic`
- [x] 已拆出的 config 小块：
  - config guard / validation / risk -> `config_basic`
- [x] 拆分时按功能命名，例如：
  - `service_status_capability_repair.rs`
  - `sqlite_capability_repair.rs`
  - `archive_capability_repair.rs`
  - `config_guard_capability_repair.rs`
  - 不使用 `split_1.rs`、`legacy_part2.rs` 等编号式命名。
- [x] 所有保留 rewrite 必须只读机器字段：
  - `semantic_kind`
  - `delivery_intent`
  - `locator_kind`
  - action/tool/capability 名
  - schema 字段
  - path / extension / status code
  - 不读用户自然语言短语。

验收：

- [x] focused planning tests 覆盖每个迁移小块。
  - 2026-06-21：`cargo test -p clawd config_structured_field_reads -- --nocapture`
  - 2026-06-21：`cargo test -p clawd delivery_archive_config_edit -- --nocapture`
- [x] `python3 scripts/check_no_nl_hardmatch.py` 通过。
- [x] `python3 scripts/check_legacy_route_boundary.py` 通过。
- [x] `cargo test -p clawd <focused_test_name> -- --nocapture` 通过。

## Track C: 旧路由命名和 trace 边界收窄

目标：继续把 `FirstLayerDecision` / `legacy_normalizer_decision` / `legacy_first_layer_decision_for_trace` 限制在 normalizer hint、journal trace 和历史日志读取边界。

- [x] 审核 `intent_router_route_output.rs` 的 `ask_mode_from_legacy_normalizer_decision()`，确认它是否还能改名为 hint-based 转换，避免暗示旧决策仍是语义权威。
  - 2026-06-21：已改名为 `ask_mode_from_normalizer_hint()`；输入仍是兼容 normalizer token，但 `AskMode` 才是 runtime dispatch 状态。
- [x] 审核 `runtime/ask_mode.rs` 中的 `legacy_route_label_for_trace()` / `legacy_first_layer_decision_for_trace()`，确认调用点只用于 trace / journal。
  - 2026-06-21：保留 `_for_trace` 命名；focused `ask_mode` tests 证明实际 dispatch 使用 `gate_kind()` / `AskMode`，旧 label 仅为日志/journal 兼容输出。
- [x] 审核 `task_journal.rs` 中 `old_first_layer_decision` 与 `legacy_first_layer_decision` 输出字段，判断是否可以新增新字段名并保留旧字段只做历史兼容。
  - 2026-06-21：`route_result` 和 rollout attribution 新增 `initial_gate_ref` / `initial_hint_ref`；旧 `legacy_first_layer_decision` / `old_first_layer_decision` 继续输出给历史日志兼容。
- [x] 保持 `semantic_route_authority` 为当前机器 token；不恢复 `agent_decides_semantic_route` / `agent_decides_migration_class` 运行时配置解析。
  - 2026-06-21：代码核对 `load_agent_loop_guard_policy()` 只解析 `semantic_route_authority` 和 `agent_loop_canary_bucket`；旧 bool 仅在配置注释、docs 和测试说明中出现。

验收：

- [x] `python3 scripts/check_legacy_route_boundary.py` 通过。
- [x] 旧字段不回流为 agent-loop 控制状态。
  - 2026-06-21：`cargo test -p clawd task_journal -- --nocapture`、`cargo test -p clawd ask_mode -- --nocapture` 通过。

## Track D: 文档和配置残留说明清理

目标：让 README / docs / config 的描述与当前代码一致，减少用户误解“旧开关仍可作为新架构配置”。

- [x] 媒体 skill registry 约束已补进当前代码事实：
  - 当前 `OutputKind` 只支持 `text/file/image/mixed`；视频/音乐生成属于文件产物，registry 使用 `output_kind = "file"`。
  - 若未来要引入 `video/audio` 输出枚举，必须先扩展 `claw-core::skill_registry::OutputKind`、UI health 输出、run_skill finalize、planner output contract，再改 registry。
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

涉及新增或修正 runner skill / registry 映射时追加：

- [ ] `cargo check -p skill-runner -p <skill-crate>`
- [ ] 直接 skill 进程 dry-run。
- [ ] `target/release/skill-runner` dry-run。
- [ ] `POST /v1/admin/reload-skills` 后 `POST /v1/tasks kind=run_skill` dry-run。
- [ ] 不实际调用 image/audio/video/music/X 等高额度或外部发布 API，除非用户明确要求 live test。

## 完成定义

- [ ] `#[allow(dead_code)]` 明显减少，剩余项都有 trace / schema / compatibility 边界理由。
- [ ] planner 旧兼容 rewrite 被拆分或迁移，`legacy_file_config_capabilities.rs` 职责明显变窄。
- [ ] 旧路由字段只作为 trace / journal / historical fallback，不作为控制状态。
- [ ] README / docs / config 描述和当前代码一致。
- [ ] 没有新增自然语言硬匹配和硬编码用户回复。
- [ ] focused tests、门禁检查和必要 NL 实测通过。
- [ ] 完成后将本计划移入归档目录，并在文件内记录完成 commit、测试命令和 NL 结果。
