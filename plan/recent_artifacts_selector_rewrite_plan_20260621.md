# Recent Artifacts Selector Rewrite Plan

创建日期：2026-06-21

状态：进行中。

## 背景

`run_20260621_090741` 最小 NL 已证明 `RecentArtifactsJudgment` 能在第一轮 observed finalizer 收口，但仍有一个执行效率问题：

- planner 可能输出 shell 风格 `run_cmd` 计划。
- runtime 会把 contract-rejected `run_cmd` 改写为 `fs_basic.list_dir` / `inventory_dir`，但该改写发生在 recent-artifacts selector 归一化之后。
- 因此改写后的 structured list action 未携带 `list_selector` 的 `target_kind=file`、`limit=2`、`sort_by=mtime_desc`，导致工具列出整个目录，再由 finalizer 过滤。

## 目标

- 对 `RecentArtifactsJudgment`，在 ad-hoc command 被替换成 preferred structured action 后，再应用一次 selector 归一化。
- 只使用 `semantic_kind`、`list_selector`、action/tool 名等机器字段；不解析用户自然语言，不新增固定回复。
- 保持用户显式要求执行 shell 命令的路径不受影响。
- 用 focused unit test 和 1 条最小 NL 验证执行层拿到 bounded structured inventory。

## 推进项

- [ ] 添加 focused unit test：planner-introduced shell listing 被替换为 `fs_basic.list_dir` 后，仍保留 `files_only=true`、`max_entries=2`、`sort_by=mtime_desc`。
- [ ] 在 normalization pipeline 中补第二次 recent-artifacts selector pass。
- [ ] 运行 focused tests、`cargo fmt --check`、hardmatch/hardreply/legacy/long-file checks、`RUSTFLAGS="-D warnings" cargo check -p clawd --all-targets`、`git diff --check`。
- [ ] release build + 重启 `clawd`。
- [ ] 跑 1 条最小 NL logs recent case，确认 `rounds=1`，并检查 step trace 的 `fs_basic` listing 已 bounded。
- [ ] 更新计划并归档。
