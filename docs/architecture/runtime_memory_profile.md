# Small-Host Runtime Memory / 小内存运行时

## Current Policy / 当前策略

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
