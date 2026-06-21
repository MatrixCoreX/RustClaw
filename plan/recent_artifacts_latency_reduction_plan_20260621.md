# Recent Artifacts Latency Reduction Plan

创建日期：2026-06-21

状态：进行中。

## 背景

Gate E final smoke 已通过，但 recent-artifacts judgment 出现明显性能长尾：

- `run_20260621_075959` case 10：`llm_elapsed_ms=442300`、`llm_calls=7`。
- 同一 smoke 中 case 6/7 也偏重，但 case 10 的问题最集中：已具备 `recent_artifacts_judgment` contract、locator=`logs`、list selector=`mtime_desc + limit=2 + file`，却让 planner 生成自由 shell 管道读取日志头部，再经过多轮 synthesis/verifier。

## 目标

- 让 `recent_artifacts_judgment` 的普通路径优先使用结构化 `fs_basic.list_dir` 和必要的 bounded `fs_basic.read_text_range`，避免自由 `system.run_command` 计划。
- 不新增用户自然语言硬匹配，不新增固定用户可见回复模板。
- 保留最终自然语言判断由 LLM/finalizer/i18n 生成；Rust 只消费机器 contract、selector、path、evidence。
- 用最小精选 NL 复测覆盖 logs recent judgment，确认调用数/耗时下降或至少不再使用自由 shell 管道。

## 推进项

- [ ] 复盘 case 10 trace，确认高耗时调用点与 plan normalization 入口。
- [ ] 添加 focused unit test：`RecentArtifactsJudgment + list_selector + locator` 下，planner 若输出 `system.run_command` 读取目录，应改写为 `fs_basic.list_dir`。
- [ ] 实现改写，只基于 `semantic_kind=recent_artifacts_judgment`、locator/list selector/allowed action 机器字段，不解析用户自然语言。
- [ ] 运行静态门禁：`cargo fmt --check`、focused tests、`check_no_nl_hardmatch.py`、`check_no_runtime_hard_reply.py`、`check_legacy_route_boundary.py`、`check_long_files.py`、`RUSTFLAGS="-D warnings" cargo check -p clawd --all-targets`、`git diff --check`。
- [ ] release build + 重启 `clawd`。
- [ ] 跑最小 NL：`nl_cases_agent_loop_recent_artifacts_judgment_focus_20260612.txt` 中 logs recent 中文/英文各一条，排除 image/audio/voice/X/Twitter。
- [ ] 根据结果更新本计划，完成后归档到 `plan/archived_completed_20260620/`。
