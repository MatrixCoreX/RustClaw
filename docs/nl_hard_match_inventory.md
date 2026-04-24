# NL Hard-Match Inventory

目标：盘点 RustClaw 当前 NL 主链里的词面硬匹配，明确哪些可以保留、哪些只能暂留、哪些必须迁移出主链。

判定口径：

- `structural`：结构/协议解析，可保留。
- `narrow_fallback`：窄 fallback，可暂留但禁止扩张。
- `must_migrate`：已经碰到主理解链，必须迁移。
- `secondary_heuristic`：不是主链 blocker，但后续不应继续扩词表。

## 1. 结构/协议解析，可保留

这些规则不是在猜用户语义，而是在抽协议和结构。

| File | Area | Why It Can Stay | Classification |
| --- | --- | --- | --- |
| `crates/clawd/src/intent/surface_signals.rs` | path/url/file/dotted-field/range token extraction | 结构解析，不决定主路由 | `structural` |
| `crates/clawd/src/worker/ask_pipeline.rs` | `Cargo.toml` / `package.json` basename resolution, candidate narrowing | 文件系统定位规则，不是主语义理解 | `structural` |
| `crates/clawd/src/agent_engine/observed_output.rs` | `read_range` / `extract_field` / `lsof` / `git status` 输出格式解析 | 执行结果解析，不是路由理解 | `structural` |
| `crates/clawd/src/intent/deterministic_gate.rs` | path/locator/field-path based plan synthesis | 依赖显式结构信号时可保留 | `structural` |

约束：

- 这类规则以后只能继续服务“结构抽取”和“协议修复”。
- 不得顺手塞入 `contains("比较")`、`contains("目录")` 这类词面语义判断。

## 2. 窄 fallback，可暂留但禁止扩张

这些规则现在还有用，但只能作为 fallback，不能继续长成主路由器。

| File | Function / Area | Current Pattern | Classification | Target |
| --- | --- | --- | --- | --- |
| `crates/clawd/src/followup_frame.rs` | `prompt_contains_same_target_reference` | `它 / 这个 / same / this / it` | `narrow_fallback` | 收口到 persisted-frame fallback parser |
| `crates/clawd/src/followup_frame.rs` | `prompt_requests_bound_target_basename_only` | `只说文件名 / basename only` | `narrow_fallback` | 降级成 renderer-side fallback |
| `crates/clawd/src/followup_frame.rs` | `detect_ordered_entry_selection` | `第一个 / 第二个 / 最后一个 / first / second / last one` | `narrow_fallback` | 后续交给 `followup_delta_kind` 或更结构化 ordinal parser |
| `crates/clawd/src/followup_frame.rs` | `detect_relative_entry_selection` | `上一个 / 前一个 / previous one` | `narrow_fallback` | 后续交给 `followup_delta_kind` |
| `crates/clawd/src/intent/continuation_resolver.rs` | `prompt_looks_like_deictic_filename_wrapper` | `那个 / 这个 / that / this` | `narrow_fallback` | 降到 clarify/follow-up 窄层 |
| `crates/clawd/src/task_context_builder.rs` | `request_looks_like_fresh_deictic_reference` | `那个 / 这个 / 那份 / this / that` | `narrow_fallback` | 用 `FollowupFrame + ClarifyState` 替代 |

约束：

- 允许保留一段时间，但必须放在 `persisted state not enough -> fallback parse` 的位置。
- 禁止继续补更多语言词表。

## 3. 必须迁移出主理解链

这些已经直接影响 route、response shape、intent family 或 follow-up 主状态流向，是当前最危险的硬匹配债。

### 3.1 `response_shape_classifier`

文件：`crates/clawd/src/intent/response_shape_classifier.rs`

问题：

- `request_prefers_one_sentence`
  - `one sentence / single sentence / briefly / 一句话 / 简短 / 简洁`
- `request_prefers_scalar`
  - `output only / return only / just the value / 只输出 / 只返回 / 只给结果`
  - 以及组合式 `raw.contains("只") && raw.contains("值") && raw.contains("给我")`

风险：

- 直接决定 `response_shape`
- 多语言扩张成本高
- 现在仍在主链生效

分类：`must_migrate`

目标：

- 拆成 `legacy_response_shape()` + `structured_response_shape()`
- 先并跑 diff
- 后续仅让小分类器处理弱语义部分

### 3.2 `intent_kind_classifier`

文件：`crates/clawd/src/intent/intent_kind_classifier.rs`

问题：

- `request_wants_file_delivery`
  - `send me / send it / attach / upload / 发给我 / 发我 / 别贴正文`
- `request_looks_like_inline_structured_transform`
  - `json / sort / markdown / table / render / convert`
- `is_bare_path_only_input_no_verb`
  - `BARE_PATH_VERB_TOKENS` 里中英大词表

风险：

- 已经在决定 `Act/ChatAct/clarify`
- 维护成本高
- 很容易继续长成多语言词表路由器

分类：`must_migrate`

目标：

- `request_wants_file_delivery` 尽量收敛到 `OutputContract + attachments + FollowupFrame`
- `is_bare_path_only_input_no_verb` 后续转成更强结构判定或小分类器

### 3.3 `fast_path`

文件：`crates/clawd/src/intent/fast_path.rs`

问题：

- `compare_targets_fast_path`
  - `比较 / 对比 / 哪个更大 / compare / bigger / smaller / which one`
- `simple_command_output_fast_path`
  - 命令自然语言触发词
- `workspace_child_bounded_listing_fast_path`
  - `目录 / folder / dir`
- `pwd_only_fast_path`
  - `pwd` 相关自然语言触发
- `short_joke_fast_path`
  - 明确是词面识别
- `explicit_local_scalar_read_fast_path`
  - `package.json / Cargo.toml / name field` 里混有词面判断

风险：

- 直接控制 pre-normalizer route
- 一旦继续扩词表，会让主链越来越不可维护

分类：`must_migrate`

目标：

- 只保留显式 path/url/file/dotted-field/range 这类结构强信号
- 词面语义部分降到小分类器或 narrow fallback

### 3.4 `followup_frame`

文件：`crates/clawd/src/followup_frame.rs`

问题：

- 当前 delta parser 还大量依赖词面识别
- 而 `FollowupFrame` 本应成为短期状态主真相

风险：

- 继续扩词表会直接伤到持续聊天稳定性

分类：`must_migrate`

目标：

- 先把 `switch_scope / pick_ordinal / reuse_target / change_slice / correction`
  收成结构化 delta
- 词面 parser 只保留为 `legacy_followup_delta_parser()`

## 4. 次级 heuristic，可保留观察但不要继续扩张

这些不是当前最高优先级 blocker，但后续不应继续加更多 marker。

| File | Area | Example | Classification | Notes |
| --- | --- | --- | --- | --- |
| `crates/clawd/src/intent/deterministic_gate.rs` | compare request detection | `比较 / compare / bigger / which one` | `secondary_heuristic` | 未来应更多依赖显式双目标结构 |
| `crates/clawd/src/agent_engine/observed_output.rs` | directory scope bias / artifact style bias | `doc/readme/script/config/log/service/docker` | `secondary_heuristic` | 输出摘要偏置，不应继续膨胀 |
| `crates/clawd/src/agent_engine/observed_output.rs` | sentence-count parsing | `one sentence / 三句话 / 一句话` | `secondary_heuristic` | 后续应统一归到 `OutputContract/response_shape` |

## 5. 迁移优先级

按风险和收益排序，建议这样清：

1. `response_shape_classifier`
2. `intent_kind_classifier`
3. `followup_frame` + `continuation_resolver` + `task_context_builder` 里的 deictic / delta 词面规则
4. `fast_path`
5. `deterministic_gate` 与 `observed_output` 的次级 heuristic

## 6. 实施边界

以后每遇到一条新词面规则，必须先判断：

1. 它是不是结构/协议解析？
2. 它是不是只能作为 narrow fallback？
3. 它是不是已经在决定 route / response_shape / follow-up 主状态？

如果答案是第 3 类，就不能再直接加进主链。
