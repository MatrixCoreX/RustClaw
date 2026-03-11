# Telegram 投递链路日志解读指南

本文用于排查 Telegram 侧重复消息、以及任务投递时序问题。注：交易确认按钮已移除，`requires_confirmation` 对 trade_preview 恒为 false。

## 建议日志级别

- 启动时设置 `RUST_LOG=debug`（或包含 `telegramd=debug`）。
- 本指南对应 `crates/telegramd/src/main.rs` 新增的 `phase=...` 日志字段。

## 全链路 phase 含义

- `phase=submit`
  - 提交任务前摘要日志（`kind/chat_id/user_id/payload_fp/payload_preview`）。
- `phase=submit_done`
  - clawd 返回 `task_id` 后日志（可与后续轮询关联）。
- `phase=poll`
  - `spawn_task_result_delivery` 轮询中的状态快照（含 `status/sent_progress_count/progress_len/elapsed_ms`）。
- `phase=deliver_progress`
  - 准备发送 progress 文本前日志（`msg_fp/msg_len/requires_confirmation`）。
- `phase=deliver_success`
  - 进入成功态后，准备发送最终结果集合前日志。
- `phase=deliver_success_item`
  - 逐条最终结果发送前日志（可和 progress 的 `msg_fp` 对比）。
- `phase=success_source`
  - 成功态消息来源判定：
    - `source=messages`
    - `source=text_fallback`
    - `source=text_only`
- `phase=confirm_detect`
  - 原为确认按钮判定；现已不再对 trade_preview 挂按钮，decision 恒为 false。
- `phase=deliver_text_confirm` / `phase=deliver_text`
  - 实际发送文本成功后日志，包含 Telegram `message_id`。
- `phase=deliver_media` / `phase=deliver_media_preface`
  - 文件/图片/语音类消息发送日志。
- `phase=callback` / `phase=callback_ack`
  - 原为确认按钮回调；交易确认按钮已移除，此 phase 仅在其他场景下出现。

## 关键字段说明

- `task_id`
  - 同一次任务的全链路关联键。
- `msg_fp`
  - 文本指纹（哈希），用于跨阶段识别“同一内容是否重复发送”。
- `msg_preview`
  - 截断后的文本预览，辅助人工核对。
- `requires_confirmation`
  - 当前发送是否会挂确认按钮（交易确认按钮已移除，此项现恒为 false）。
- `telegram_msg_id`
  - Telegram 实际投递消息 ID，可用于确认是否真实发送了多条。

## 排查重复消息的推荐顺序

1. 用 `task_id` 过滤日志，确认只看单任务。
2. 对比 `phase=deliver_progress` 与 `phase=deliver_success_item` 的 `msg_fp`：
   - 若相同，说明跨阶段重复。
3. 看 `phase=success_source`：
   - 若出现 `text_fallback`，常见于 `messages` 偏移后回退到 `text`。
4. 看 `phase=deliver_text_confirm`：
   - 若同 `msg_fp` 出现两次且 `telegram_msg_id` 不同，说明同一条文本重复发出。

## 实战示例（买币链路）

当用户发送“帮我买1u eth”时，关注以下顺序：

1. `submit` / `submit_done`
2. `poll(status=Running)` + `deliver_progress(trade_preview)`（不再挂确认按钮）
3. `poll(status=Succeeded)` + `success_source(...)` + `deliver_success_item(...)`
4. 若再次出现相同 `msg_fp` 的 `deliver_text_confirm`，即可确认重复来源。

