# Ask/Act 主链：progress / delivery / trace 三层交付语义

本文档描述 clawd 中 ask/act 主链上 **progress**、**delivery**、**trace** 的职责与写入点（已收口后的状态）。

## 1. 三层职责（收口后）

### 1.1 progress

- **用途**: 阶段提示 / 进度提示，供通信端“处理中”展示。
- **不允许**: 承载最终正文、完整原始工具/技能输出。
- **写入**: `append_progress_hint(state, task, &mut loop_state.progress_messages, hint)`；仅短句，如 "Skill X completed"、"Reply generated"。
- **发布**: `publish_progress(state, task, &progress_messages)` → `update_task_progress_result`，payload 为 `{ "progress_messages": progress_messages }`。
- **日志**: `debug!("progress published task_id=... count=... last=...")`。

### 1.2 delivery

- **用途**: 最终交付给用户的内容；通信端唯一正式消费的最终结果来源。
- **写入**: 仅两处：
  1. **respond**：当 `should_publish_respond_message` 为 true 时，`append_delivery_message(&task.task_id, &mut loop_state.delivery_messages, text)`。
  2. **fallback finalizer**：当 loop 结束且 delivery 为空但 subtask_results 非空时，用 `fallback_finalize_from_raw(subtask_results)` 生成一条，再 `append_delivery_message`；若仍为空则 `synthesize_final_response`（chat）生成一条并 `append_delivery_message`。
- **不写入**: tool/skill 原始输出不再进入 delivery；仅进入 subtask_results 与 trace。
- **日志**: `info!("delivery appended task_id=... len=... content=...")`；fallback 时 `info!("delivery fallback_from_raw ...")` / `info!("delivery fallback_from_synthesize ...")`。

### 1.3 trace

- **用途**: 日志、调试、排障；可包含原始执行输出。
- **不进入**: 不直接进入用户可见的 progress 或 delivery。
- **写入**: `append_subtask_result`（subtask_results）、`register_step_output`（last_output / output_vars）、`history_compact`。
- **日志**: `info!("... trace_only=raw_not_delivery")` 于 skill 输出；`debug!("trace_only step_output ...")` 于 register_step_output。

## 2. 当前写入点汇总

| 内容类型           | 写入 progress | 写入 delivery | 写入 trace (subtask_results / step_output) |
|--------------------|---------------|---------------|---------------------------------------------|
| tool/skill 原始输出 | 仅短 hint     | 否            | 是                                          |
| respond            | 短 hint       | 是（若 publish）| 是（subtask_result + register）             |
| fallback from raw  | 否            | 是            | 否                                          |
| synthesize final   | 否            | 是            | 是（subtask_result + history）             |

## 3. 主流程最终结果

- **位置**: `main.rs` ask 分支 `match result { Ok(mut answer) => ... }`。
- **行为**: `answer.text` = `delivery_messages.last().or(subtask_results.last())`；`answer.messages` = `delivery_messages`。经 `intercept_response_payload_for_delivery` 后写入 `update_task_success`。
- **结论**: 用户侧最终交付仅来自 **delivery**（及无 delivery 时的 subtask_results 最后一条兜底）；progress 仅用于“处理中”展示，不混入最终 result。

## 4. respond 职责与去重

- respond 是**最终交付动作**；不与 raw tool/skill output 重复进入 delivery。
- `should_publish_respond_message` 为 false 时（不进入 delivery）：
  - 与 `delivery_messages.last()` 相同；
  - 与 `last_output`（上一 step 的原始输出）相同；
  - 或为 summary 类且用户未显式要求总结。
- 允许：文件 token、路径、用户明确要求的总结/解释/转述。不鼓励：无必要的重复自然语言包装。

## 5. 日志排查

- **某条内容为什么进 progress**：来自 `append_progress_hint`，仅短提示；查 `progress published` 日志。
- **某条内容为什么进 delivery**：来自 `append_delivery_message` 或 fallback；查 `delivery appended` / `delivery fallback_from_raw` / `delivery fallback_from_synthesize`。
- **某条内容仅 trace**：来自 `register_step_output` / `append_subtask_result`；查 `trace_only step_output` / `trace_only=raw_not_delivery`。

---

以上为收口后的 progress / delivery / trace 三层语义与写入点说明。
