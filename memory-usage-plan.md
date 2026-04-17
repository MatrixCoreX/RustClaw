# RustClaw 内存占用排查与治理计划

更新时间：2026-03-17

## 目标

- 明确区分“正常工作集上升（分配器保留）”与“真实泄漏（持续单调增长）”。
- 优先修复已识别的可累积点，降低长期运行的内存上限。
- 建立最小可用观测，避免后续靠猜测排障。

## 已知现状（当前结论）

- `clawd` 内存增长目前更像工作集上升后平台化，并非直接由 `channel-gateway` 导致。
- `channel-gateway` 与 `clawd` 是独立进程口径，不会直接抬高 `clawd` RSS。
- 代码中存在一个明确可累积点：`RateLimiter.per_user`（`HashMap<i64, VecDeque<u64>>`）键数量可随用户规模长期增长。
- `telegramd` 侧也有按 chat 持有的运行态 `HashMap`，缺少统一 TTL 清扫策略，可能在长期运行中缓慢增长。

## 执行计划

### Phase 1：先做低风险修复（当天可完成）

1. 修复 `clawd` 的 `RateLimiter.per_user` 键清理
   - 位置：`crates/clawd/src/main.rs` 的 `RateLimiter::check_and_record`
   - 方案：
     - 清理过期时间戳后，对空 `VecDeque` 的用户执行 `retain` 删除。
     - 确保不影响现有限流语义（global/user rpm 判定保持不变）。

2. 给 `telegramd` 运行态 map 增加定时清扫
   - 位置：`crates/telegramd/src/main.rs`，针对：
     - `pending_resume_by_chat`
     - `pending_image_by_chat`
     - `bound_identity_by_chat`（可按“最近活跃 chat”策略择优清理）
   - 方案：
     - 复用现有时间戳字段（或补充）做 TTL 删除。
     - 保留功能正确性（不影响正在进行的确认/恢复流程）。

3. 编译与回归
   - `cargo check -p clawd -p telegramd`
   - 冒烟用例：
     - 发送普通 ask、多轮 ask、`/positions`、失败后“继续”流程；
     - 检查消息结果与之前一致（除计划内输出格式变更）。

### Phase 2：降低内存峰值（本周内）

1. 减少每轮大字符串拼接
   - 位置：`crates/clawd/src/agent_engine.rs`
   - 目标：
     - 对技能 playbook 文本做快照级缓存（按技能集/配置版本缓存）。
     - 避免每轮都拼接大段 `String`。

2. 限制 memory context 的拼接开销
   - 位置：`crates/clawd/src/memory/service.rs` / `memory/retrieval.rs`
   - 目标：
     - 进一步约束 `chat_memory_budget_chars` 实际上限；
     - 提前裁剪低价值段落，减少最终 prompt 体积。

3. 评估日志体积与内存抖动
   - 位置：`MODEL_IO_LOG_MAX_CHARS` 相关路径
   - 目标：
     - 保留排障价值前提下，降低超长字符串创建频率。

### Phase 3：观测与判定标准（并行执行）

1. 增加轻量指标打点（日志即可）
   - 建议周期输出：
     - `clawd rss`
     - `RateLimiter.per_user.len()`
     - 最近 N 次 ask 的 prompt 字符数统计（min/p50/p95/max）
     - 当前排队和运行任务数

2. 判定标准
   - 正常：冷启动上升 -> 平台区间波动。
   - 可疑：业务低负载下仍持续单调上升，且 30~60 分钟不回落。
   - 泄漏高疑：释放压力后仍线性上涨，且与并发/请求量弱相关。

## 验收标准

- 连续运行 24 小时后：
  - `clawd` RSS 峰值较修复前下降，或至少达到更稳定的平台值；
  - `RateLimiter.per_user.len()` 不再只增不减；
  - `telegramd` 运行态 map 大小受控（随活跃 chat 波动，而非累积）。

## 回滚与风险

- 所有改动保持增量、可回滚，不触及核心业务协议。
- 如出现异常（限流误伤、恢复流程中断），优先回滚对应清理逻辑。

## 任务拆分建议（可直接执行）

1. `clawd`：限流 map 清理（1 PR）
2. `telegramd`：pending map TTL 清理（1 PR）
3. `clawd`：playbook 缓存与 prompt 裁剪（1 PR）
4. 观测日志与判定脚本（1 PR）

