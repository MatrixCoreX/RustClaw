# Background Task Resume Convergence Plan (2026-06-21)

目标：把 RustClaw 的长任务执行收敛到 Codex/Claude 风格的“前台快速返回 task_id、后台 worker 持续执行、可 checkpoint、可恢复、可轮询、可取消”的架构。不是取消所有 timeout，而是把 timeout 分层：HTTP/channel 等待不等于任务失败，worker lease/heartbeat 负责存活判断，tool/skill 有自己的 timeout，agent loop 软预算耗尽时写 checkpoint 并进入 waiting/background。

## Current Code Baseline

- [x] `worker_once()` 已是后台轮询 worker，API 提交任务后返回 `task_id`，执行发生在 worker。
- [x] `WorkerConfig` 已包含 `task_timeout_seconds`、`llm_total_timeout_seconds`、`task_heartbeat_seconds`、`running_no_progress_timeout_seconds`、`running_recovery_check_interval_seconds`。
- [x] `task_lifecycle.rs` 已定义 `TimeoutLayer`、`TaskLifecycleState::{Waiting,Background,NeedsUser}`、`TaskCheckpoint`、`ResumeEntrypoint`、`ResumeTrigger`、`CheckpointResumeDirective`。
- [x] `repo/tasks.rs` 已有 due paused checkpoint claim、resume work item、resume executor claim、execution plan record 等仓储函数。
- [x] `repo/task_resume_execution.rs` 已有 planned -> handoff -> dispatch -> result projection 的机器状态流。
- [x] `worker/runtime_support.rs` 已在每轮 worker tick 先执行 `maybe_recover_stale_running_tasks_runtime()`，串联 stale running recovery、due checkpoint、resume executor、handoff、dispatch、result projection。
- [x] `resume_replay_executor` 与 `async_poll_executor` 已作为 runtime dispatch executor 接入。
- [x] 当前实现没有依赖用户自然语言硬匹配来驱动恢复链路，恢复链路主要消费机器字段和状态 token。
- [x] 直接 `kind=run_skill` 路径已用 `health_check` 跑通一次真实 E2E：任务成功，查询响应通过顶层 `data.lifecycle` 暴露状态机器字段；该路径不要求模型处理 auth/key。

## Remaining Gaps

- [x] 计划/文档层已明确“长任务不会靠单次 HTTP 请求硬撑到底”，并把当前代码里的 timeout 分层语义和恢复链路写进 README 流程图。
- [x] `get_task` / active task 查询对 paused checkpoint 的可见性已复核：`TaskQueryResponse.lifecycle` 与 `ActiveTaskItem.lifecycle` 都来自 `task_query_lifecycle_projection()`，已暴露 `state`、`resume_due`、`resume_wait_seconds`、`checkpoint_id`、`can_poll`、`can_cancel` 等机器字段。
- [x] stale running recovery 已补 worker/runtime 单测：同一轮 runtime tick 覆盖“stale 普通 running -> timeout；paused waiting/background -> 保持 running 并走 resume executor”。
- [x] async job resume 已有 poll executor，已补测试确认 `PollAsyncJob` 的成功、等待、过期、失败不会变成自然语言硬回复；文档说明仍归入 README Track。
- [x] seeded agent loop resume 已接入 dispatch，已补直接单测确认 checkpoint 带 completed side effects 时会恢复 idempotency guard 状态，避免从头重做副作用。
- [x] API/channel 层已在 README 中明确：调用方超时只应继续 poll task_id，不能把后台任务标记失败。
- [x] UI 后续约束已写入 README：runtime 只输出机器字段，文案交给 UI/i18n；具体 UI 渲染实现不在本轮后端计划范围内。
- [x] 压缩 NL 覆盖集要固定为 release-gate equivalent：不必每次跑 2100 条；285 条或更小精选集可以作为推进代码的门槛，最终发布前再按需跑大集合。
- [x] 精选 NL 初跑已暴露并修复媒体产物链路问题：媒体输出路径不再被当成 UTF-8 文本读取/写入，已存在的生成产物路径也不会触发 scalar content auto-locator 直接短路为“读文件名”结果。
- [ ] 精选 NL 仍有非媒体缺口待收敛：配置/系统矩阵只返回单值、临时文件副作用超时、归档/SQLite 证据不足、package/docker dry-run 证据不足、checkpoint/resume 机器字段 case 被澄清回复截断。

## Implementation Tracks

### Track A - Lifecycle Query And Observability

- [x] 核对 `TaskQueryResponse` 与 `ActiveTaskItem.lifecycle` 是否已经包含所有 UI 需要的机器字段。
- [x] 若缺字段，只在 `task_lifecycle::task_query_lifecycle_projection()` 追加机器字段，不新增用户可见固定自然语言；本轮复核无需新增生产字段。
- [x] 增加单测：queued/running/waiting/background/needs_user/terminal 的 query projection 字段稳定。

### Track B - Runtime Recovery Gate

- [x] 增加 worker/runtime 单测覆盖普通 stale running 和 paused checkpoint stale running 的分流。
- [x] 增加 due checkpoint resume tick 测试：从 due waiting/background 到 work item/executor state 的最短闭环。
- [x] 增加 lease 保护测试：已有 active resume lease 时不重复 claim（现有 `repo/tasks_tests.rs` 覆盖 due checkpoint 与 executor lease suppression）。

### Track C - Async Job Resume

- [x] 补 `PollAsyncJob` 端到端单测：pending -> wait/reschedule、succeeded with observation -> verify/finalize、expired -> terminal machine failure。
- [x] 确认 async job adapter 输出只使用 `status_code`、`message_key`、`job_id`、`cancel_ref`、`artifact_refs` 等机器字段。
- [x] README 描述 async long-tail tool：先启动 job，再 checkpoint，再由 worker poll。

### Track D - Seeded Agent Loop Resume

- [x] 补 seeded resume 单测：checkpoint 恢复后带 budget counters、observations、completed side effect refs。
- [x] 确认恢复后的 planner 继续消费 checkpoint，不从原始 NL 重跑已完成副作用。
- [x] 对恢复失败写入结构化 `TerminalFailureReason` / `error_code` / `message_key`，自然语言表达交给 finalizer/i18n。

### Track E - README And Flowcharts

- [x] 按当前代码重写 README 中和任务执行相关说明。
- [x] 重画/更新 3 个流程图：
  - `POST /v1/tasks` -> queued -> worker -> tool/skill -> result。
  - agent loop soft budget -> checkpoint -> waiting/background -> resume executor -> result projection。
  - async job -> poll/reschedule -> verify/finalize。
- [x] 明确旧语义 pre-route 不再新增普通语义分类；planner/runtime resolver 消费结构化能力字段。

### Track F - Compressed NL Gate

- [x] 建立最小精选 NL 覆盖文档：覆盖 ask、run_skill、文件、配置、系统状态、任务查询/取消、agent loop、checkpoint/resume、图片、语音；默认剔除 X 发布/获取等真实 X API 操作。
- [x] 已用精选集推进第一批代码修复。当前精选集固定为 `scripts/nl_tests/cases/nl_cases_codex_task_resume_release_smoke_20260621.txt`，初跑目录为 `scripts/nl_suite_logs/manual/20260621_143601`，首轮 24 case 结果为 14 succeeded / 8 failed / 2 timeout。
- [x] 图片/语音链路已单独复测通过：
  - `image_edit_smoke_zh`：task `60be9809-c125-43be-8249-12831829f727`，输出 `document/rust_icon_pixel_smoke.png`，wall 254s。
  - `audio_synthesize_smoke_zh`：task `50ca35c0-98d2-4241-a076-71e15211e29c`，输出 `document/skill_audio_smoke.mp3`，wall 81s。
- [x] provider 额度不足时记录为 external blocker，不把路由/代码判失败；当前已确认 `image_generate_smoke_zh` 属于图像生成额度/渠道 blocker，web search 后端缺失也属于外部能力 blocker。
- [ ] 继续修复剩余非媒体 NL 缺口；完成后再跑一轮更大 aggregate 或等价覆盖。

### Track G - Cleanup Boundary

- [x] 复核是否还有旧 rollback/compat 路径仍实际被 runtime 配置读取；旧 `agent_decides_*` 不再由运行时配置读取，`semantic_route_authority=legacy` 仍是当前可控回滚 token，不能作为无入口旧代码删除。
- [x] 不删除历史日志解析 fallback、测试防回归、迁移说明中的旧 token。
- [x] 每次 Rust 改动后运行：
  - `cargo fmt --check`
  - `python3 scripts/check_long_files.py`
  - `python3 scripts/check_no_nl_hardmatch.py`
  - `python3 scripts/check_no_runtime_hard_reply.py`
  - `python3 scripts/check_legacy_route_boundary.py`
  - 受影响 crate 的 `cargo check` 或定向测试

## Latest Verification

- [x] `cargo fmt --check`
- [x] `cargo test -p clawd media_artifact -- --nocapture`
- [x] `cargo test -p clawd scalar_content_auto_locator_does_not_read_generated_file_path_report_target -- --nocapture`
- [x] `cargo test -p clawd image_edit_prompt_alias_normalizes_to_instruction -- --nocapture`
- [x] `cargo test -p clawd normalize_planned_actions_applies_skill_arg_aliases_before_verifier -- --nocapture`
- [x] `python3 scripts/check_long_files.py`
- [x] `python3 scripts/check_no_nl_hardmatch.py`
- [x] `python3 scripts/check_no_runtime_hard_reply.py`
- [x] `python3 scripts/check_legacy_route_boundary.py`
- [x] `cargo check -p clawd --all-targets`
- [x] `cargo build -p clawd --release`

## Done Criteria

- [x] README 和流程图与当前代码一致，并解释前台请求、后台 worker、heartbeat/lease、checkpoint/resume、async poll 的关系。
- [x] runtime recovery 至少有普通 stale、paused waiting/background、active lease、async poll、seeded loop resume 的定向测试。
- [ ] 精选 NL 集通过；图片/语音能力至少覆盖一次，若 provider 额度不足则保留结构化 blocker 记录。
- [x] 没有新增自然语言硬匹配或 runtime 硬回复模板。
- [ ] `plan/` 根目录只保留未完成计划；完成后本文件移入归档目录。
