# 任务事件归档与回放合同

RustClaw 把实时事件交付与持久回放分开：

- `task_event_stream`：用于低延迟 SSE 的有界热后缀。
- `task_event_archive`：只追加的脱敏事件记录。
- `task_event_snapshots`：绑定精确来源 seq 区间和哈希的周期投影。
- `task_event_artifacts`：保存超过内联事件预算的 payload。

## 事件准入

每个准入事件包含：

- 事件和 payload schema version；
- 任务内单调递增的 `seq`；
- 当前事件哈希和前一事件哈希；
- task/thread/parent/child 机器引用；
- 脱敏元数据和 artifact 引用；
- 脱敏 payload，或持久化的大 payload artifact 引用。

只有精确 `(lease_owner, claim_attempt)` 仍有效时才追加 claim-owned 事件。事件准入会在一个 SQLite 事务中写入 hot row 和 archive row，然后才通知实时 SSE 订阅者。

## 快照

每归档 256 个事件以及发生 `task_final` 时写入快照。快照记录：

- `source_event_range.start_seq/end_seq/event_count`；
- 对有序来源事件哈希计算的 SHA-256 digest；
- snapshot hash；
- 事件类型计数和最新机器 task/execution state；
- 归档脱敏策略。

快照是 replay 索引和完整性证据，不能替代来源事件，也不得包含原始 prompt、原始 provider 响应、凭据或未脱敏的用户可见 payload。

## 回放

`GET /v1/tasks/{task_id}/events` 通过有界分页读取 archive，只把 hot broadcast channel 当作唤醒信号。因此从 cursor zero 开始的客户端可以读取超过 hot 1,024-event 后缀的数据而不阻塞。

- `archive_replay` 表示从持久 archive 恢复了旧 hot cursor。
- 只有 archive 本身存在真实前缀缺口时才发送 `cursor_expired`；payload 会报告可用区间和 replay source。
- `follow=false` 会读取当前所有 archive 页面后关闭。
- `follow=true` 会先读取 archive，再等待新通知。

浏览器 task trace 和教学模式消费这些带版本事件。`clawcli events/watch` 保留原始机器事件访问；`clawcli replay export` 在只记录、已脱敏的 bundle 中包含归档事件 seq/hash/payload。

## 保留与删除

Archive 遵循任务保留策略。只有任务行被配置的 task retention policy 删除后，cleanup 才删除 hot event、archive row、snapshot 和 event artifact。Replay export 是显式 operator artifact，不能用作 runtime 权限、路由或任务状态。
