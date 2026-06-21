# Recent Artifacts Latency Reduction Plan

创建日期：2026-06-21

状态：进行中，代码优化与静态门禁已完成，等待 release 重启和最小 NL 实测。

## 背景

Gate E final smoke 已通过，但 recent-artifacts judgment 出现明显性能长尾：

- `run_20260621_075959` case 10：`llm_elapsed_ms=442300`、`llm_calls=7`。
- 同一 smoke 中 case 6/7 也偏重，但 case 10 的问题最集中：已具备 `recent_artifacts_judgment` contract、locator=`logs`、list selector=`mtime_desc + limit=2 + file`，第一步也已经使用结构化 `fs_basic.list_dir`，但 loop 没有把可交付的 `inventory_dir` 机器字段及时交给 runtime observed finalizer，导致后续多轮 plan / selected content read / synthesis / verifier。

## 目标

- 让 `recent_artifacts_judgment` 的普通路径在已具备可交付结构化 `inventory_dir` 清单时优先由 runtime observed finalizer 收口；只有清单不足、选择器过滤后为空或确需内容证据时，才继续 bounded `fs_basic.read_text_range`。
- 不新增用户自然语言硬匹配，不新增固定用户可见回复模板。
- 保留最终自然语言判断由 LLM/finalizer/i18n 生成；Rust 只消费机器 contract、selector、path、evidence。
- 用最小精选 NL 复测覆盖 logs recent judgment，确认调用数/耗时下降或至少不再使用自由 shell 管道。

## 推进项

- [x] 复盘 case 10 trace，确认高耗时调用点与 loop observed-finalize 收口入口。
- [x] 添加 focused unit test：`RecentArtifactsJudgment + list_selector + locator` 下，已有 `inventory_dir` 机器字段时可第一轮停止；选择器过滤后为空时不能误停。
- [x] 实现收口优化，只基于 `semantic_kind=recent_artifacts_judgment`、selector、skill/action、JSON `inventory_dir` 机器字段，不解析用户自然语言。
- [x] 运行静态门禁：`cargo fmt --check`、focused tests、`check_no_nl_hardmatch.py`、`check_no_runtime_hard_reply.py`、`check_legacy_route_boundary.py`、`check_long_files.py`、`RUSTFLAGS="-D warnings" cargo check -p clawd --all-targets`、`git diff --check`。
- [ ] release build + 重启 `clawd`。
- [ ] 跑最小 NL：`nl_cases_agent_loop_recent_artifacts_judgment_focus_20260612.txt` 中 logs recent 中文/英文各一条，排除 image/audio/voice/X/Twitter。
- [ ] 根据结果更新本计划，完成后归档到 `plan/archived_completed_20260620/`。

## 已完成验证

- `cargo test -p clawd recent_artifacts_inventory -- --nocapture`
- `cargo test -p clawd recent_artifacts_judgment_classifies_logs_per_entry -- --nocapture`
- `cargo test -p clawd recent_artifacts_judgment -- --nocapture`
- `cargo fmt --check`
- `python3 scripts/check_no_nl_hardmatch.py`
- `python3 scripts/check_no_runtime_hard_reply.py`
- `python3 scripts/check_legacy_route_boundary.py`
- `python3 scripts/check_long_files.py`
- `RUSTFLAGS="-D warnings" cargo check -p clawd --all-targets`
- `git diff --check`
