# Small-Host Runtime Memory / 小内存运行时

## Current Policy / 当前策略

### Dynamic admission / 动态准入

`HostResourceSnapshot` is the runtime's single source for effective memory and
CPU capacity. It uses the host or cgroup memory limit, current available memory,
swap use, cgroup memory events, and Linux PSI when the platform exposes them.
macOS and other platforms keep conservative fallbacks instead of emulating
Linux `/proc` files. A hysteretic state machine projects `normal`, `compact`,
`constrained`, or `critical`, so one noisy sample does not repeatedly start and
stop work.

`HostResourceSnapshot` 是运行时判断有效内存和 CPU 容量的唯一来源。它综合宿主或
cgroup 内存上限、当前可用内存、swap、cgroup 内存事件，以及平台支持时的 Linux PSI。
macOS 和其他平台使用保守回退，不模拟 Linux `/proc`。带滞回的状态机输出
`normal`、`compact`、`constrained` 或 `critical`，避免单次噪声让任务反复启停。

Every tool, process skill, browser action, local-model action, durable background
job, and provider/model call reserves one atomic `ResourceBroker` lease before
execution. The broker takes the maximum of the declared request, the host-owned
class floor, and the versioned observed process-tree peak. It also reserves CPU,
network, provider, and browser slots. Durable jobs persist non-secret lease
metadata and restore or release it after a daemon restart.

所有工具、进程技能、浏览器、本地模型、持久后台任务和 provider/model 调用，都必须
在执行前原子申请同一个 `ResourceBroker` 租约。内存申请取 manifest 声明、宿主资源类
下限和按 registry generation 记录的进程树观测峰值三者最大值，同时预留 CPU、网络、
provider 和浏览器 slot。持久任务只保存无敏感信息的租约元数据，守护进程重启后恢复或释放。

Resource pressure is not a task failure. The agent may make one verifier-bounded
alternative plan; repeated refusal writes a machine-readable `resource_waiting`
checkpoint and the normal resume worker retries it after capacity returns.
Interactive waiting work is preferred over aged background work, while
cancellation and terminal delivery keep their existing idempotency contracts.

资源压力不等于任务失败。Agent 可以进行一次受 verifier 约束的替代规划；连续拒绝后写入
机器可读的 `resource_waiting` checkpoint，由统一恢复器在容量恢复后继续。交互等待任务优先于
后台任务，同时取消、终态交付和幂等合同保持不变。

The optional `[runtime_resources]` table in `configs/config.toml` is the only
administrator override for pressure thresholds and safety reserve. Leave it
empty for automatic host/cgroup-derived policy. Supported fields are
`safety_reserve_mib`, `critical_available_floor_mib`, the three
`*_available_ratio` and `*_psi_avg10` thresholds, plus
`escalation_samples`/`recovery_samples`. Invalid or non-monotonic values fail
startup explicitly; there is no second compatibility configuration.

`configs/config.toml` 的可选 `[runtime_resources]` 是压力阈值和安全储备的唯一管理员覆盖
入口。保持空表即可使用宿主/cgroup 自动策略。支持 `safety_reserve_mib`、
`critical_available_floor_mib`、三组 `*_available_ratio` 与 `*_psi_avg10`，以及
`escalation_samples`/`recovery_samples`。非法或非单调配置会明确阻止启动，不存在第二套兼容配置。

The dashboard exposes bounded capacity, reservations, active lease and waiting
task counts, and the latest machine reason. Its dependency view separates missing,
disabled, and memory-constrained states. The read-only diagnostic export is
redacted and capped at 256 KiB; raw PIDs, paths, credentials, task content, and
log bodies are not included.

首页展示有界容量、已预留资源、运行/等待租约数和最近机器原因；依赖检查区分缺失、已关闭
和内存不足暂不可运行。只读诊断导出默认脱敏并限制为 256 KiB，不包含原始 PID、路径、凭据、
任务内容或日志正文。

### Small-host process and storage profile / 小内存进程与存储策略

The core detects host RAM at startup. On hosts with at most 2 GiB RAM,
main, audit, and runtime-owned skill SQLite pools open connections on demand
and release connections idle for 60 seconds during the pool's periodic reap.
The configured maximum connection count, WAL, foreign keys, busy timeouts,
schemas, and persisted data are unchanged. Larger or unrecognized hosts retain
the default pool policy. In-memory test databases retain their dedicated pool.

核心对不超过 2 GiB 内存的主机采用按需数据库连接。主库、审计库和运行时管理的
技能私有库连接空闲 60 秒后，由连接池定期回收；不是到第 60 秒立即回收。
连接数上限、WAL、外键、锁等待、数据库结构和现有数据都不变。
其他主机保留默认策略，内存数据库测试不使用此回收策略。

On Linux/glibc small hosts, the core applies allocator settings before starting
threads: at most two arenas, and 1 MiB mmap/trim thresholds. This trades some
allocation concurrency for lower retained anonymous memory. Explicit
`MALLOC_*` tuning or `glibc.malloc.*` entries in `GLIBC_TUNABLES` take precedence;
the automatic profile does not override them. macOS and non-glibc targets do
not call glibc APIs. Startup logs expose `runtime_allocator_tuning` with
`attempted` and `applied`. See the [glibc allocator reference](https://sourceware.org/glibc/manual/latest/html_node/Malloc-Tunable-Parameters.html).

Linux/glibc 小内存主机在线程创建前设置最多两个内存分配区，以及 1 MiB 的大块
分配与空闲回收阈值，以部分分配并发度换取更低的内存滞留。用户已有分配器调优
配置优先，自动策略不覆盖。macOS 和非 glibc 平台不调用此接口。
启动日志可查看是否尝试设置以及是否成功。

When the automatic glibc profile is active, a serialized background operation
also attempts `malloc_trim(0)` once per 60 seconds on a blocking worker. It only
returns allocator-free pages, including whole-page holes between live objects;
it does not clear application caches, task history, live buffers, or databases.
It does not block the async reactor or run on macOS/non-glibc hosts. Explicit
allocator overrides disable this automatic maintenance too. See
[malloc_trim semantics and thread safety](https://man7.org/linux/man-pages/man3/malloc_trim.3.html).

自动 glibc 策略生效时，每 60 秒通过阻塞工作线程串行尝试归还空闲堆页，包括仍然存活的
对象之间已经释放的整页。这不是删除应用缓存、任务历史或数据库，也不改变仍被使用的内存。
该操作不在异步执行线程上运行；macOS、非 glibc 平台或手动覆盖分配器配置时不启用。

Skill receipt JSON is read incrementally and its canonical digest is computed
with a bounded 64 KiB serialization buffer. Receipt schemas, digest bytes,
manifest matching, pinned versions, and artifact integrity checks are unchanged.
This avoids retaining full serialized copies of large Python package receipts.

技能收据改为流式读取，摘要计算使用 64 KiB 序列化缓冲区，不再额外复制整份 JSON。
收据格式、摘要结果、manifest 校验、版本固定和文件完整性检查保持不变。

## Verification / 验证

`scripts/runtime_memory_probe.py` issues only authenticated GET requests to the
local core health, AiAPP catalog, and skill-store endpoints. It prints request
latency, response size, RSS/PSS, swapped memory, database descriptors, and peak
samples without credentials or API response bodies. On Linux, run it as the
deployment user with an existing admin web session:

```sh
python3 scripts/runtime_memory_probe.py --root . --label candidate
```

Compare old and new binaries after equivalent restarts and identical workloads.
Include `Pss + SwapPss`, not just RSS: swapping memory out is not a reduction in
the runtime's memory footprint. This is a diagnostic measurement, not a strict
memory cap or a guarantee that a browser/media workload fits into 1 GiB.
Do not clear user data, disable authorization, or lower skill resource grants
to improve benchmark numbers.

比较新旧版本时，两边都要在相同重启条件下运行同一组请求，并比较 PSS 与换出量之和。
仅看 RSS 可能把交换分区占用误认为优化。此策略不改变任务预算和技能授权，也不保证
1 GiB 设备能够容纳任意浏览器或媒体任务；不得通过删除用户数据或绕过资源准入提高成绩。

### Measured JSON and SQLite hot paths / 已测 JSON 与 SQLite 热点

The 2026-10-01 read-only large-database probe used a 9.0 GiB runtime database
with 3.30 million archived task events and 2,367 event artifacts. SQLite
`quick_check` passed. A 50-row task-history page used about 189 KiB of Python
heap, a 100-row artifact metadata page used about 33 KiB, a bounded 20-row
large-result projection used about 2.7 MiB, and the worst 1,024-event replay
page used about 7.0 MiB. The report contains no payload content or credentials:
`docs/performance/runtime_large_db_probe_20261001.json`.

2026-10-01 的只读大库探针使用约 9.0 GiB 主库，其中包含 330 万归档事件和
2,367 个事件 artifact，`quick_check` 通过。50 条任务历史页约使用 189 KiB
Python 堆，100 条 artifact 元数据页约 33 KiB，20 条大结果受限投影约 2.7 MiB，
最重的 1,024 条事件回放页约 7.0 MiB。报告不含载荷正文和凭据。

One historical long task exposed the dominant amplification: 17 model records
carried about 8.0 MiB of request JSON, while per-token event persistence grew
the same task to 29,406 archived records and 27.5 MiB. Runtime event projection
now archives the first token delta and every 64th delta, while keeping all
semantic lifecycle, tool, usage, completion, and interruption events. Two
post-change model tasks produced 44 and 64 archived events. Their complete raw
model request/response records remain in the bounded, task-scoped teaching
trace; token deltas are also cleared before the planner state is reused.

一个历史长任务暴露了主要放大源：17 条模型记录的请求 JSON 合计约 8.0 MiB，
逐 token 事件持久化却把同一任务放大到 29,406 条归档记录、约 27.5 MiB。当前投影
只归档首个 token delta 和之后每第 64 个 delta，同时完整保留生命周期、工具、用量、
完成与中断事件；修改后两个真实模型任务分别只有 44 和 64 条归档事件。完整模型原始
请求/响应仍由按任务分页的教学 trace 保存，planner 复用状态前会清空 token delta 列表。

These measurements do not justify a repository-wide `Arc<str>` or `Bytes`
conversion: bounded database pages are below the process-tree browser and local
model peaks, and such a rewrite would increase ownership complexity. The
measured event amplification was fixed at its producer boundary instead.

这些数据不支持全仓改写为 `Arc<str>` 或 `Bytes`：受限数据库页远低于浏览器和本地模型
进程树峰值，全面改写反而增加所有权复杂度。因此本轮只在测得的事件生产边界消除放大。

SQLite resilience is covered by `scripts/tests/test_sqlite_resilience.py`,
which uses disposable databases to verify committed/uncommitted process-crash
recovery, concurrent readers with a committing writer, online backup restore,
and integrity checks. Existing Rust tests separately verify migration digests,
busy retry, and skill-private storage isolation.
