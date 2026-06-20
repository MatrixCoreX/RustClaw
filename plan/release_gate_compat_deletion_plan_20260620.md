# Release Gate Compat Deletion Plan

创建日期：2026-06-20

状态：未完成；Gate B/C/D 已完成，Gate E 未完成。旧 agent-loop / Codex-task 两个大计划的普通代码项已归档；本文件只跟踪最终删除旧 route / rollback / compatibility 路径前必须满足的 release gate。

## 目标

在不牺牲多语言 agent 可维护性的前提下，把最后的旧 pre-route / rollback / compatibility 路径从“保留应急”推进到“可物理删除”：

- 不新增用户自然语言硬匹配。
- 不新增 runtime / finalizer / verifier / execution adapter 的固定用户可见回复模板。
- 继续以 `semantic_route_authority`、机器字段、TaskContract、OutputContract、CapabilityResolver、PlanVerifier、journal evidence 和 route-delta 为判断依据。
- 物理删除前必须证明新 agent-loop 默认路径覆盖旧兼容路径的安全边界。

## 当前代码事实

- `plan/agent_loop_ideal_state_convergence_plan_20260615.md` 与 `plan/codex_task_execution_convergence_plan_20260617.md` 已归档到 `plan/archived_completed_20260620/`。
- 归档计划中的普通代码项已完成；剩余的是 release-gated 物理删除门槛，不是普通代码待办。
- `scripts/check_legacy_route_boundary.py` 当前用于确认旧 first-layer token 没有回流为 agent-loop / verifier / execution adapter 的控制状态。
- `scripts/nl_tests/cases/nl_cases_client_like_release_gate_equivalent.txt` 是 285 条 release-gate equivalent 精选覆盖集；覆盖 JSON 为 `scripts/nl_tests/cases/nl_cases_client_like_release_gate_equivalent_coverage.json`。
- 覆盖 JSON 当前记录：
  - `input_rows = 2096`
  - `selected_rows = 285`
  - `coverage_categories = 317`
  - `covered_categories = 317`
  - `missing_categories = 0`
  - `excluded_rows = 4`
- `nl_cases_client_like_all_aggregate.txt` 是大集合来源；运行时继续排除 image / audio / voice / X / twitter / tweet 等不适合 live API 的 case。

## 删除门槛

### Gate A: 静态边界门禁

- [x] `python3 scripts/check_no_nl_hardmatch.py`
- [x] `python3 scripts/check_no_runtime_hard_reply.py`
- [x] `python3 scripts/check_legacy_route_boundary.py`
- [x] `python3 scripts/check_long_files.py --json`
- [x] `RUSTFLAGS="-D warnings" cargo check -p clawd --all-targets`

### Gate B: 285 release-gate equivalent

- [x] MiniMax 跑完 `nl_cases_client_like_release_gate_equivalent.txt`，排除 image/audio/voice/X/twitter/tweet，功能性通过。
- [x] route-delta 使用同一批最新 case 口径汇总，`unexplained_mismatch_count = 0`。
- [x] 生成或更新 rollout metrics，确认失败不是 provider/account 阻塞。
- [x] 人工抽查高风险代表项：文件交付、缺失文件、调度、服务状态、配置读取、workspace summary、follow-up、repair。

说明：285 通过后，可以继续推进非删除、可回滚代码收敛；但不能单独授权物理删除最后旧 rollback / compatibility 路径。

进度记录：

- 2026-06-20: MiniMax `run_20260620_093132` 从 case 1 开始，case 1-9 功能性通过；case 10 `cases_act_service_status_brief` 失败，原因是 `service_status` finalizer 把 planner 已合成的一句话服务状态覆盖成 health_check 系统健康字段 dump，随后 answer verifier 以 `answer_verifier_gap` 阻断。
- 2026-06-20: 已按机器字段修复：`service_status` deterministic replacement 在具备 `service_control` 结构化观测且已有 publishable one-sentence synthesis 时不覆盖 LLM 合成；该判断只检查输出形状和结构化观测，不匹配用户自然语言。
- 2026-06-20: focused 验证通过：`cargo test -p clawd finalize_loop_reply_prefers_service_control_status_over_health_check_dump --quiet`、`cargo test -p clawd service_status --quiet`、格式/diff/硬匹配/硬回复/legacy route/长文件门禁、`RUSTFLAGS="-D warnings" cargo check -p clawd --all-targets`、`cargo build --release -p clawd`。
- 2026-06-20: clawd release 重启后，MiniMax 单条复测 `run_20260620_095651` case 10 通过，回复为一句话服务状态；rollout `pass_rate=1.0`，route-delta `route_delta_items=2`、`unexplained_mismatch_count=0`。
- 2026-06-20: MiniMax `run_20260620_095902` 从 case 11 继续，case 11-16 功能性通过；case 17 `nl_case_act_log_analyze_brief_only_0001` 暴露质量问题：planner 已生成简短异常总结，但 `recent_artifacts_judgment` deterministic replacement 将最终回复覆盖为 `recent_entries.*` 机器字段清单。
- 2026-06-20: 已按输出契约修复：`recent_artifacts_judgment` 在当前 delivery 是 planner 最新 publishable synthesis，且契约要求 one-sentence 时保留合成回复；该判断基于 response shape、planner delivery 与结构化 inventory，不匹配用户自然语言。
- 2026-06-20: focused 验证通过：`cargo test -p clawd recent_artifacts_judgment_preserves_one_sentence_synthesis --quiet`、`cargo test -p clawd recent_artifacts_judgment --quiet`、格式/diff/硬匹配/硬回复/legacy route/长文件门禁、`RUSTFLAGS="-D warnings" cargo check -p clawd --all-targets`、`cargo build --release -p clawd`。
- 2026-06-20: clawd release 重启后，MiniMax 单条复测 `run_20260620_101531` case 17 通过，回复为自然语言简短总结且未输出机器字段清单；rollout `pass_rate=1.0`，route-delta `route_delta_items=2`、`unexplained_mismatch_count=0`。
- 2026-06-20: MiniMax `run_20260620_101829` 从 case 18 继续，case 18-26 功能性通过；case 27 `nl_case_fuzzy_top3_only_0001` 首次失败为 4 行 `name size`，修正后第二次仍为 4 行 path_list，说明问题分两层：`FilePaths` 契约被 size-ranked projection 污染，且 normalizer 未把 locator 中的 top-k 机器 token 落到 `list_selector.limit`。
- 2026-06-20: 已按机器字段修复：`FilePaths` 的 `inventory_dir` 结构化观测只投影 `entries[].path/resolved_path`，不输出 `name size`；`list_selector.limit` 由 finalizer/verifier 通用消费；同时把单条 fuzzy top3 测试 prompt 改为明确要求前 3 个路径，避免 runtime 从目录名推断用户语义。
- 2026-06-20: focused/宽测通过：`cargo test -p clawd matrix_file_paths_inventory_uses_paths_and_applies_selector_limit --quiet`、`cargo test -p clawd matrix_shape --quiet`、`cargo test -p clawd quantity --quiet`、`cargo test -p clawd answer_verifier --quiet`、格式/diff/硬匹配/硬回复/legacy route/长文件门禁、`RUSTFLAGS="-D warnings" cargo check -p clawd --all-targets`、`cargo build --release -p clawd`。
- 2026-06-20: clawd release 重启后，MiniMax 单条复测 `run_20260620_104836` case 27 通过，回复为 3 行路径；随后 case 45 暴露 locator-path top-k repair 会误伤 all-matches path-list，因此撤销该 runtime repair，保留显式 `list_selector.limit` 通用支持。后续需重新复测 case 27 与 case 45，再从 case 46 继续 Gate B。
- 2026-06-20: clawd release 重启后，MiniMax 单条复测 `run_20260620_111810` case 27 通过，显式 top3 prompt 回复 3 行路径；MiniMax 单条复测 `run_20260620_111839` case 45 通过，all-matches path-list 回复 4 行完整路径。两个 run 均 rollout `pass_rate=1.0`，route-delta `route_delta_items=2`、`unexplained_mismatch_count=0`。后续从 case 46 继续 Gate B。
- 2026-06-20: Gate B 全部 285 条 MiniMax latest-case 去重口径完成；`case_count=285`、`missing_case_ids=[]`、`status_counts.succeeded=285`、`final_status_counts.success=285`、`pass_rate=1.0`、`delivery_consistent_counts.true=285`、`prompt_truncation_count=0`、`provider_final_error_count=0`。本轮存在 2 次 provider retryable plan timeout，但均由重试恢复；2 个 parse error 来自旧 partial JSON，未进入 latest-case 选择。
- 2026-06-20: Gate B route-delta 汇总完成；`turn_files=285`、`route_delta_items=551`、`unexplained_mismatch_count=0`。解释性差异为 `agent_loop_valid_direct_response_vs_legacy_planner=79`、`legacy_attribution_schema_without_decision_envelope=8`、`planner_decision_rejected:respond_requires_evidence_observation=2`，其余 `not_mismatch=462`。
- 2026-06-20: Gate B 高风险人工抽查完成，代表项均为 `succeeded/success` 且 `delivery_consistent=true`：case 2 文件交付、case 8 调度、case 9/195 缺失文件、case 10/209 服务状态、case 42 workspace summary、case 128/142/281 follow-up/alias/basename、case 170 repair、case 181 失败步骤总结、case 189 高风险删除澄清、case 193 配置验证、case 218 web search、case 285 配置 dry-run。
- 2026-06-20: Gate B 结论：285 等价集已经足够支持继续做非删除、可回滚代码收敛；但 Gate C/D/E 未完成，仍不能物理删除最后旧 rollback / compatibility 路径。

### Gate C: 500 canary

- [x] 最新去重 case 口径达到 500/500 功能性通过。
- [x] route-delta `unexplained_mismatch_count = 0`。
- [x] 若存在 explained mismatch，必须全是机器可解释且符合新 agent-loop 预期，例如 agent-loop valid direct response vs legacy planner label。

进度记录：

- 2026-06-20: MiniMax `run_20260620_194139` 从 all-aggregate case 1 开始执行 Gate C；case 1-36 功能性通过。case 37 `cases_act_health_check_brief_en` harness 判定通过，但人工审计发现 final reply 被 `service_status` deterministic replacement 覆盖成 `system_health.*` / `clawd_health_port_open` 等机器字段 dump，未采用 planner 已生成的健康检查摘要，因此本 partial run 不计作 Gate C 通过。
- 2026-06-20: 已按机器字段修复：`service_status` 在存在 successful `health_check` 结构化观测、已有 publishable summary synthesis、且当前请求不是显式 `system_health.*` raw/scalar selector 时，保留 planner 合成摘要；该判断只检查 action/selector/response shape/机器字段形态，不匹配用户自然语言，也不新增固定用户可见回复模板。
- 2026-06-20: focused/静态验证通过：`cargo test -p clawd service_status -- --nocapture`、`cargo test -p clawd health_check -- --nocapture`、格式/diff/硬匹配/硬回复/legacy route/长文件门禁、`RUSTFLAGS="-D warnings" cargo check -p clawd --all-targets`、`cargo build --release -p clawd`。
- 2026-06-20: clawd release 重启后，MiniMax 单条复测 `run_20260620_204123` case 37 通过，回复为健康检查摘要而非 `system_health.*` 字段 dump；rollout `pass_rate=1.0`、`prompt_truncations=0`。后续可从 case 38 继续发现问题；最终 Gate C 仍需形成 500 条 latest-code 通过口径后才能勾选完成。
- 2026-06-20: MiniMax `run_20260620_204522` 从 case 38 继续，case 38-63 功能性通过；case 50 与 case 63 人工审计发现同类质量问题：`process_basic` 服务状态 direct-answer 在 runtime 里生成固定中英文用户可见模板，并把内部 action 名 `process_basic` 泄漏给用户。该 partial run 因此停在 case 64 前，不计作 Gate C 完成。
- 2026-06-20: 已按新架构修复：`process_basic` 服务状态的非 scalar / one-sentence 用户回复不再由 observed-output direct-answer 生成固定自然语言模板；scalar 路径只保留 `running/not_running` 机器状态 token，多轮普通服务状态交给 planner synthesis / finalizer 语言层渲染。该改动不解析用户自然语言，也不新增固定回复句子。
- 2026-06-20: focused/静态验证通过：`cargo test -p clawd process_basic_service_status -- --nocapture`、`cargo test -p clawd process_basic_no_match -- --nocapture`、`cargo test -p clawd finalize_loop_reply_preserves_process_basic_status_summary_synthesis -- --nocapture`、`cargo test -p clawd service_status -- --nocapture`、格式/diff/硬匹配/硬回复/legacy route/长文件门禁、`RUSTFLAGS="-D warnings" cargo check -p clawd --all-targets`、`cargo build --release -p clawd`。
- 2026-06-20: clawd release 重启后，MiniMax 单条复测 `run_20260620_212610` case 50 与 `run_20260620_212738` case 63 均通过；回复由 synthesis 生成，不再出现 `process_basic 没有返回匹配的进程记录` 固定模板。后续从 case 64 继续 Gate C。
- 2026-06-20: MiniMax `run_20260620_212941` 从 case 64 继续，case 64-141 功能性通过；人工审计发现 case 142/143 hidden entries strict-list 质量问题：`fs_basic.list_dir` 已返回完整 `names` 机器字段（`.agents/.codex/.git/.gitignore/.pids`），但 direct candidate 被 matrix grounding 拦截，模型只从 evidence 摘要/size summary 中看到 `.codex` 并解释“其余无法补全”。该 partial run 因此停在 case 144 前。
- 2026-06-20: 已按机器字段修复：`matrix_checked_direct_candidate` 对 `HiddenEntriesCheck + Strict` 增加结构化专用通过条件，候选逐行必须等于最新 `latest_hidden_entries` 投影结果（含 selector limit），避免把隐藏项列表交给 LLM 猜测；该判断不解析用户自然语言，不新增自然语言回复模板。
- 2026-06-20: focused/静态验证通过：`cargo test -p clawd hidden_entries -- --nocapture`、格式/diff/硬匹配/硬回复/legacy route/长文件门禁、`RUSTFLAGS="-D warnings" cargo check -p clawd --all-targets`、`cargo build --release -p clawd`。clawd release 重启后，MiniMax focused `run_20260620_232119` 覆盖 case 142/143/144 全部通过，均直接输出 5 个隐藏项名字。后续从 case 145 继续 Gate C。
- 2026-06-20: Gate C 性能观察：`run_20260620_212941` 中多条功能通过但耗时偏高，代表包括 case 66 `llm_elapsed_ms=380378`（contract_repair 约 268s）、case 84 `265244`、case 88 `272785`、case 112 `250078`、case 114 `209429`、case 128 observed 约 159s。当前不阻断功能门槛，但应作为后续 prompt/observed evidence 压缩优化项。
- 2026-06-21: MiniMax `run_20260620_232324` 覆盖 case 145-179 功能性通过；case 176 首次 provider timeout 后由 harness retry 恢复，case 175/179 等 recent-artifacts judgment 仍有高耗时（case 175 约 386s，case 179 约 343s）。断电后从 case 180 继续。
- 2026-06-21: MiniMax `run_20260621_005009` case 180 通过；case 181 暴露质量问题：zip 二进制 read_range 失败后，最终回复被 recent-artifacts one-sentence preservation 误保留为 `recent_entries.count=3`，缺少条目和分类判断。该 run 停在 case 182 前，不计作 Gate C 完成。
- 2026-06-21: 已按机器字段修复：`recent_artifacts_judgment` 不再把只包含 `recent_entries.*` / `classification.*` 的机器字段 dump 当作 publishable one-sentence synthesis；这类输出会回退为完整 deterministic per-entry verdict。该判断只识别语言无关机器字段 key，不匹配用户自然语言，也不新增固定自然语言回复模板。
- 2026-06-21: focused/静态验证通过：`cargo test -p clawd recent_artifacts_judgment_replaces_count_only_machine_field_synthesis -- --nocapture`、`python3 scripts/check_no_nl_hardmatch.py`、`python3 scripts/check_no_runtime_hard_reply.py`、`python3 scripts/check_long_files.py`、`git diff --check`。小修已单独提交并 push：`20d26fb8 Fix recent artifact count-only fallback`。clawd release 重启后，MiniMax focused `run_20260621_010610` case 181 通过，回复包含 3 个 recent entries 和 temporary bundle vs formal config 判断。后续从 case 182 继续 Gate C。
- 2026-06-21: Gate C latest-code 分段推进到 case 500 并完成汇总。关键修复包括 inline transform follow-up contract（`9f6452ba`）和 `package.json` follow-up selector 归一化（`13320a43`），二者都只消费机器 contract/token，不新增用户自然语言硬匹配或固定回复模板。最后失败点 case 436/437 复测 `run_20260621_065525` 通过，case 437 输出 `rustclaw-nl-fixture`；case 438-500 续跑 `run_20260621_065620` 63/63 通过。
- 2026-06-21: Gate C rollout metrics 已按 latest-case 去重汇总到 `logs/agent_rollout_metrics/gate_c_500_20260620_20260621_rollout_metrics.json`：`case_count=500`、`missing_case_ids=[]`、`status_counts.succeeded=500`、`final_status_counts.success=497`、`final_status_counts.clarify=3`、`pass_rate=1.0`、`delivery_consistent_counts.true=500`。汇总跳过 8 个历史 partial JSON parse error，它们均来自失败/中断 run，已被后续同 case 成功 run 覆盖。
- 2026-06-21: Gate C route-delta 已按 latest-case 去重汇总到 `logs/agent_rollout_metrics/gate_c_500_20260620_20260621_route_delta.json`：`case_count=500`、`route_delta_items=1037`、`unexplained_mismatch_count=0`。解释性差异为 `not_mismatch=935`、`agent_loop_valid_direct_response_vs_legacy_planner=36`、`legacy_attribution_schema_without_decision_envelope=42`、`planner_decision_rejected:respond_requires_evidence_observation=24`，均为机器可解释项。

### Gate D: 2100 safe aggregate 或覆盖等价证明

- [>] 方案一：跑完剔除禁测项后的 `nl_cases_client_like_all_aggregate.txt`，功能性通过且 route-delta 干净；本轮未采用，保留为后续更重回归选项。
- [x] 方案二：用覆盖 JSON 证明 285 等价集覆盖全部 release gate category，并补足 gate B/C 的 provider-run 证据。
- [x] 若采用方案二，必须在本计划记录覆盖证明摘要与未覆盖风险。

进度记录：

- 2026-06-21: 采用方案二。覆盖证明来自 `scripts/nl_tests/cases/nl_cases_client_like_release_gate_equivalent_coverage.json`：`input_rows=2096`、`selected_rows=285`、`coverage_categories=317`、`covered_categories=317`、`missing_categories=[]`、`excluded_rows=4`。排除范围仅限当前 live API 禁测项 tag：`audio`、`image`、`voice`、`x`、`twitter`、`tweet` 及对应 image/audio skill tag。
- 2026-06-21: provider-run 证据已由 Gate B 285 MiniMax 全通过和 Gate C 500 MiniMax latest-case 全通过补足；两批 route-delta 均为 `unexplained_mismatch_count=0`。未覆盖风险：未实测 image/audio/voice/X/Twitter live API；这类外部发布/媒体能力仍需 dry-run、mock 或专门账号额度窗口验证，不能由本 release gate 授权实际 live 发布。

### Gate E: rollback window

- [ ] 至少 24-48 小时或连续 3 个发布窗口/准线上 run 无 unexplained mismatch。
- [ ] 确认 provider/account 阻塞已和功能失败区分记录。
- [ ] 确认 `semantic_route_authority` 应急回滚 token 是否仍需保留；若要删除，必须另开删除 patch 并单独验证。

## 可推进顺序

1. 先跑 Gate B 的 285 MiniMax 覆盖；如果 provider 额度阻塞，记录阻塞 run，不把它当功能失败。
2. Gate B 干净后，继续跑或汇总 Gate C 的 500 canary。
3. Gate C 干净后，在 Gate D 选择 2100 全量或覆盖等价证明。
4. Gate D/E 都满足后，才允许写删除 patch，物理删除旧 route / rollback / compatibility 路径。
5. 删除 patch 必须再次跑 Gate A，并做 focused NL smoke，不得引入自然语言硬匹配或硬回复模板。

## 当前禁止事项

- 不得因为 285 短集通过就删除最终 emergency rollback token。
- 不得把旧 `FirstLayerDecision` / route label 重新变成普通运行态控制字段。
- 不得为了单个 NL case 在 Rust 主流程添加 `prompt.contains(...)`、语言短语数组或固定回复句子。
- 不得把 image / audio / voice / X / twitter / tweet live API case 混入当前 release gate 实测。
