# P2.1 + P2.2 大颗粒拆分方案（AppState 拆分 + DB pool）

> 状态：**proposal / await review** — 不动手实施，等用户评审后单独立项。
> 编辑日期：2026-04-17

---

## 背景

`/home/guagua/.cursor/plans/llm_请求层优化完整计划_5b939b41.plan.md` 列出的 P2 子项里，
有两条是真正的大颗粒改造：

* **P2.1 AppState 拆分** — 当前 `AppState` 单一 struct 已经长到 60+ 字段、
  `runtime/state.rs` 649 行；任意一个新字段都要在 12 处构造点同步加。
* **P2.2 DB pool** — 当前 `db: Arc<Mutex<Connection>>` 是全局单连接，
  35 处 `state.db.lock()`；高 QPS 下会成为序列化瓶颈，且单写单读阻塞已经
  是经验上的痛点（worker 写 + UI 读会互斥）。

这两项都是动一发牵全身的重构，必须先把"做什么 / 怎么切"讲清楚，避免上手
后才发现某个隐藏依赖把作用域吹炸。

---

## P2.1 — AppState 拆分

### 现状审计

* `crates/clawd/src/runtime/state.rs`: 649 行，`pub(crate)` 字段 60 个左右
  （`pub(crate) ` 出现 124 次，含方法签名）。
* 添加单个字段 → 必须改 **12 处构造点**：
  - `crates/clawd/src/main.rs`（生产真实构造）
  - `crates/clawd/src/agent_engine/{planning, loop_finalize, dispatch_support, skill_execution}.rs`
  - `crates/clawd/src/{skills, skills/builtin, execution_recipe, verifier, memory, memory/retrieval}.rs`
  - `crates/clawd/src/delivery_utils/tests.rs`
  - `crates/clawd/src/worker/ask_finalize.rs`

  P1.5 加 `llm_by_prompt_per_task` 一个字段就要在所有 12 处加上同样的初始化行，
  这正是 P2.1 想根治的痛点。

* 字段使用频次（`rg "state\.<x>"` 计数大致分布）：
  - 极高频：`state.db`（35 处）、`state.llm_providers`、`state.skill_views_snapshot`、
    `state.routing`、`state.maintenance`。
  - 中频：`state.tools_policy`、`state.workspace_root`、`state.skill_semaphore`、
    `state.memory`。
  - 低频但跨模块共享：`state.telegram_*` / `state.whatsapp_*` / `state.wechat_send_config` /
    `state.feishu_send_config`（频道适配器配置，仅 channel_send 用）。

### 字段分类（拆成 5 组的草案）

按"调用域 + 修改频率"自然分簇：

1. **`CoreServices`**（核心运行时句柄，所有模块都拿）
   - `db: Arc<Mutex<Connection>>` — 转 P2.2 后改成 pool
   - `llm_providers: Vec<Arc<LlmProviderRuntime>>`
   - `agents_by_id: Arc<HashMap<String, AgentRuntimeConfig>>`
   - `http_client: Client`
   - `skill_views_snapshot: Arc<RwLock<Arc<SkillViewsSnapshot>>>`

2. **`SkillRuntime`**（技能链路）
   - `skill_timeout_seconds`
   - `skill_runner_path`
   - `skill_semaphore`
   - `tools_policy`
   - `cmd_timeout_seconds` / `max_cmd_length`
   - `workspace_root` / `default_locator_search_dir` / `locator_scan_max_depth` /
     `locator_scan_max_files`

3. **`PolicyConfig`**（运维 / 安全 / 限速）
   - `maintenance: MaintenanceConfig`
   - `memory: MemoryConfig`
   - `routing: RoutingConfig`
   - `self_extension: SelfExtensionConfig`
   - `rate_limiter: Arc<Mutex<RateLimiter>>`
   - `allow_path_outside_workspace` / `allow_sudo`

4. **`WorkerConfig`**（worker / 调度行为参数）
   - `worker_task_timeout_seconds` / `worker_task_heartbeat_seconds`
   - `worker_running_no_progress_timeout_seconds`
   - `worker_running_recovery_check_interval_seconds`
   - `last_running_recovery_check_ts`
   - `queue_limit`
   - `started_at`
   - `database_busy_timeout_ms` / `database_sqlite_path`

5. **`ChannelConfig`**（外部通道适配器，仅 channel_send 系列消费）
   - `telegram_bot_token` / `telegram_configured_bot_names`
   - `whatsapp_cloud_enabled` / `whatsapp_api_base` / `whatsapp_access_token` /
     `whatsapp_phone_number_id` / `whatsapp_web_enabled` / `whatsapp_web_bridge_base_url`
   - `wechat_send_config` / `feishu_send_config` / `lark_send_config`
   - `future_adapters_enabled`

6. **`TaskMetricsRegistry`**（per-task LLM 计数 / by_prompt / schedule cache）
   - `llm_calls_per_task`
   - `llm_elapsed_per_task`
   - `llm_by_prompt_per_task`
   - `task_schedule_intent_cache`

7. **`ReloadContext`**（用于 `reload_skill_views` 的旁路入口；可考虑 lazy 拆出）
   - `config_path_for_reload`
   - `registry_path_for_reload`
   - `skill_switches_for_reload`
   - `initial_skills_list_for_reload`
   - `command_intent: CommandIntentRuntime`
   - `schedule: ScheduleRuntime`
   - `persona_prompt`
   - `active_provider_type`

### 实施策略（按风险递增）

**阶段 1：保留 AppState 大壳 + 内嵌子 struct（零迁移成本）**
- `AppState` 内部把字段重新组织：
  ```rust
  pub(crate) struct AppState {
      pub(crate) core: CoreServices,
      pub(crate) skills: SkillRuntime,
      pub(crate) policy: PolicyConfig,
      pub(crate) worker: WorkerConfig,
      pub(crate) channels: ChannelConfig,
      pub(crate) metrics: TaskMetricsRegistry,
      pub(crate) reload: ReloadContext,
  }
  ```
- 加 `Deref`-like accessor 方法（`state.db()`, `state.skill_timeout()`等）保持
  调用面只改一次 → 但**不改任何调用方代码**：先在新结构上加同名的 const 方法。
- 改 12 个构造点为构造 7 个子 struct，相比加单字段，构造代码集中、可读。
- **预期改动面**：1 个 state.rs 重写 + 12 处构造点 + 0 处使用点。
  改动行数 ~600 行，但 diff 全部集中在 state.rs。
- **风险**：低。所有字段访问表达式仍然 `state.xxx` —— 因为子 struct 是
  `pub(crate)` 字段，可继续直接访问 `state.core.db` 等。但这意味着调用面
  会被批量变更（`rg state.db | wc -l` ~35 处）。

**阶段 2：调用面分批迁移（高频字段先做）**
- 把"`state.db` → `state.core.db`"这种最频繁的字段访问，每次一个高频字段
  改完 + 验证，分若干 PR 推进。
- 每次 PR 影响面有限（一个字段名空间），易于 review。
- **风险**：低-中。改动量大（~400 行 grep+replace），但语义完全等价。

**阶段 3（可选）：把子 struct 抽成独立模块的"业务封装"**
- 例如 `CoreServices` 加 `acquire_db_conn()`, `note_llm_call(...)` 等业务方法。
- 一些 self-contained 子模块可只持有它需要的 sub-struct，不再需要整个
  AppState（参数瘦身、易测试）。
- **风险**：中。这是真正的"拆分"，可以等 1-2 月运行稳定后再做。

### 收益

* **加新字段不用动 12 处**：只动子 struct 的构造，diff 缩到 1 个文件。
* **测试 fixture 简化**：构造小 struct 比拼整个 AppState 容易。
* **后续做 P2.2（DB pool）时，db 类型变化的影响只在 CoreServices 内部**，
  不会污染所有 12 个构造点。
* **可读性**：状态类目集中可见，新人 onboarding 时不必扫 60 个字段才能理解
  哪些属于"channel 适配器"vs 哪些属于"worker 行为参数"。

### 估时

* 阶段 1：**4-6 小时**（含 cargo check 反复修字段引用 + 12 个 fixture）
* 阶段 2：**6-10 小时**（按字段分 4-6 个 PR，每 PR 1-2 小时含验证）
* 阶段 3：可选，按业务推进。

### 风险与缓解

* **风险 A**：阶段 1 一次性改 12 个 fixture 时漏改，运行时才崩。
  缓解：cargo check 必须 0 error 才进入下一步；每个 fixture 编译完跑一次
  对应 module 的 unit test。
* **风险 B**：阶段 2 grep+replace 时漏掉非常规调用形式（如 `(&state).db`）。
  缓解：每个字段改完先 `cargo check` 找到所有未替换处再继续。
* **风险 C**：和正在进行的功能 PR 频繁冲突。
  缓解：尽量在功能 PR 间隙短窗推进；阶段 1 可以一次干完，避免长期 dirty。

---

## P2.2 — DB pool

### 现状审计

* `db: Arc<Mutex<Connection>>` — `rusqlite::Connection` + `tokio::sync::Mutex` 包裹。
* 全仓共 **35 处 `state.db.lock()`**，分布最重的：
  - `crates/clawd/src/memory.rs` — 19 处
  - `crates/clawd/src/http/ui_routes.rs` — 7 处
  - `crates/clawd/src/worker/runtime_support.rs` — 4 处
  - 其他 11 个文件零散 1-3 处。
* 数据库 = sqlite，文件路径在 `database_sqlite_path`，busy_timeout 配置在
  `database_busy_timeout_ms`。
* 大部分 `db.lock()` 持有时间 < 10ms（rusqlite 是同步 API，但都是命中索引
  的简单 SELECT/INSERT）。

### 痛点

1. 所有 worker 任务 + 所有 UI 请求 + 所有 memory/audit 写入都串行化在这一把
   `Mutex` 上。短查询基本不感知，但当某次写入触发 sqlite 的"忙锁"
   （busy_timeout 内重试）时会拖累整条调用链。
2. 拓宽性差：未来如果加并行 worker / 多并发 channel adapter，这个全局锁
   会先被打到。
3. `tokio::sync::Mutex` 在 sync code 里 lock + 调用 rusqlite 同步 API 的组合
   要求每次都 `.lock().await`，调用面 await 污染。

### 候选方案对比

| 方案 | 实现 | 改动面 | 优点 | 缺点 |
|---|---|---|---|---|
| **A. r2d2-sqlite + tokio::task::spawn_blocking** | `r2d2::Pool<r2d2_sqlite::SqliteConnectionManager>` | 中（35 处 + spawn_blocking 包装） | 最稳熟方案；多连接并行 | 每次 db 调用走 blocking pool，需要重写所有 lock 调用 |
| **B. deadpool-sqlite (async wrapper)** | `deadpool_sqlite::Pool` 内部仍 spawn_blocking | 中-高 | 原生 async API；与 tokio 自然 | 仍是 sync 之上 wrap，没真正 async；新 crate 依赖 |
| **C. sqlx::SqlitePool** | `sqlx::SqlitePool` 替换 rusqlite | **高** | 真原生 async；编译期 SQL 检查 | rusqlite → sqlx 是类型系统迁移，35 处 SQL 都要改写；migration 路径不平凡 |
| **D. 不拆 pool，只把 read 与 write 分开两把锁** | `RwLock<Connection>` 不行（rusqlite 不支持并发只读 & sqlite 写也只能单线程）；改成 `db_read: Arc<Mutex<Connection>>` + `db_write: Arc<Mutex<Connection>>` | 低 | 改动最小 | 收益有限；只缓解了 read-heavy 场景 |

### 推荐：A（r2d2-sqlite + spawn_blocking）

**理由**：
* rusqlite 已有的 35 处调用全部是同步 API 形态，A 方案只要把
  "lock 一把全局 mutex" 换成 "从 pool 取一个 conn"，调用面变化最小。
* sqlite 的写其实是**单线程内核**（serialized mode），多连接并发也不会
  让写吞吐翻倍；但读是并发的，pool 化之后读密集场景显著受益（memory.rs 有
  大量 SELECT 类查询）。
* 不引入大型 ORM 迁移成本（C 方案）。
* 新依赖最小：`r2d2 = "0.8"` + `r2d2_sqlite = "0.24"`，都是轻量稳定 crate。

**实施草案**：

1. CoreServices 字段从 `db: Arc<Mutex<Connection>>` 改成 `db_pool: Arc<r2d2::Pool<...>>`。
2. 加 helper：
   ```rust
   impl CoreServices {
       /// 在 spawn_blocking 里跑一段使用 conn 的 sync 闭包。
       /// 大部分 db 调用都用这个 helper。
       pub(crate) async fn db<F, T>(&self, f: F) -> Result<T, anyhow::Error>
       where
           F: FnOnce(&mut rusqlite::Connection) -> Result<T, anyhow::Error> + Send + 'static,
           T: Send + 'static,
       {
           let pool = self.db_pool.clone();
           tokio::task::spawn_blocking(move || {
               let mut conn = pool.get()?;
               f(&mut conn)
           }).await?
       }
   }
   ```
3. 35 处调用从 `let conn = state.db.lock().await; ... conn.execute(...)?;` 改成
   `state.core.db(|conn| { ... conn.execute(...) }).await?;`。
4. busy_timeout 在 pool 的 `connection_customizer` 里设。
5. 写入路径里 INSERT 后要 `last_insert_rowid()` 的几处，把它包在同一个
   闭包里返回。

**估时**：
* 设计 + helper：1 小时。
* 35 处调用迁移：每处 5-10 分钟，**4-6 小时**。
* 测试 + 跑 _golden / _b1_regression / _minimax_patch_v2：1-2 小时。
* 合计：**1-1.5 天**。

**风险**：
* 风险 A：spawn_blocking 包装后，原 `?` error type 链路要走 anyhow。
  缓解：先在 helper 里统一 anyhow，调用面 `.await?` 即可。
* 风险 B：原代码里嵌套调用（lock 后再 lock 同一个 mutex）会死锁。pool 化
  之后嵌套 `pool.get()` 不会死锁，但**也不会重用同一个事务**——如果有
  原子事务横跨多次 `db.lock()`，必须改成单个闭包。
  缓解：grep 检查每个长 db.lock() 闭包内是否再调 db.lock()。
* 风险 C：sqlite 多 conn 并发写时仍然会串行化，但会 eat 一部分 retry quota。
  缓解：busy_timeout 调到合理值（已是 ms 级）。

---

## P2.7 — drop legacy router（备注）

* 计划文档里 P2.7 是"删除老的 intent_router 调用面"。
* **先决条件**：normalizer-direct-reply（P1.1）已上线一段时间，**at-least 2 周
  生产数据 + LLM 调用次数指标**确认旧路径调用归零。
* **当前状态**：P1.1 已 2026-04 落地（见 `crates/clawd/src/intent_router.rs:113`
  与 `crates/clawd/src/ask_flow.rs:89,426`），但生产指标尚未跑满 2 周窗口。
* **建议**：本 proposal 不实施 P2.7，记录为"等指标 ≥ 2 周稳定 + by_prompt
  指标显示 router_legacy 桶为 0 后再启动"。

---

## 推进顺序建议

按收益 / 风险比：

1. **P2.1 阶段 1**（state.rs 内嵌子 struct 重组）— 单 PR，**当周可做**，
   立刻消除"加字段动 12 处"的痛点。
2. **P2.1 阶段 2**（调用面 grep+replace 迁移）— 分 4-6 个 PR，按字段分批，
   每周 1-2 个 PR 推进。
3. **P2.2 DB pool**（r2d2-sqlite）— 等 P2.1 阶段 1 完成（CoreServices 已
   存在），1-1.5 天集中实施。
4. **P2.7** — 等 by_prompt 指标 ≥ 2 周显示 router_legacy 桶为 0。

总耗时估算：**约 2-3 周（按周 5h 投入）** 把 P2.1 + P2.2 落地，P2.7 看
观测窗口。
