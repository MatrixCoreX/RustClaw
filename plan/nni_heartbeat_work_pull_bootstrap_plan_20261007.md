# NNI 心跳拉取任务与受控命令执行启动计划

状态：complete

日期：2026-10-07

涉及仓库：

- Agent Runtime：`/home/guagua/rustclaw`
- NNI 服务端：`/home/guagua/NNI`

## 1. 目标

把现有“设备定期签名心跳”扩展为“心跳成功后主动领取一个服务端任务”，先用 Linux/macOS
受控命令验证完整链路，并为后续 `inference_v1`、`training_v1` 任务保留稳定协议：

1. 管理员在 NNI 服务端创建任务。
2. 在线设备完成原有签名心跳后，按自身平台、架构和能力领取一个匹配任务。
3. 任务在独立本地 worker 中执行，不延长或阻塞心跳 HTTP 请求。
4. 执行结果先持久化，到下一次正常心跳时随心跳使用设备签名上报服务端。
5. 服务端保存任务、租约、attempt、结果与审计事件，可查询成功或失败原因。
6. 原有心跳奖励、资产、Bancor、白名单和结算语义保持不变。

本轮 `command_v1` 是调度通道的验收 adapter，不把任意 shell 当成未来推理/训练协议。
后续计算任务使用新的类型化 adapter，不通过拼接 shell 文本实现。

固定调度决策：设备不启动独立的空闲长轮询，也不为了找任务缩短心跳间隔。只有原定心跳
成功时检查一次任务；有任务随该次心跳领取，没有任务就继续等待下一次正常心跳。任务完成后
只在本机持久化结果，到下一次正常心跳再上报；不立即发起额外网络请求。

## 2. 核心边界

### 2.1 保持不变

- `/heartbeat/request` 与 `/heartbeat/verify` 的硬件签名、白名单和资产绑定要求。
- 合法心跳的记录、奖励资格、奖励窗口和结算事务。
- H 只标识设备并签名，A 继续作为资产账户。
- 设备只发起出站 HTTPS，不开放公网入站端口。
- 工作任务失败不影响心跳成功，也不影响奖励。

### 2.2 新增但隔离

- 独立 `distributed_work_tasks` 与 `distributed_work_attempts` 表，不写奖励/资产/Bancor 表。
- 心跳成功响应可携带一个 `work_assignment`；无任务时明确返回 `null`，不额外轮询。
- 下一次 heartbeat request 携带待上报结果，结果内容摘要绑定到该次 heartbeat challenge。
- Agent Runtime 使用独立任务目录和 worker；心跳只领取、唤醒和回报状态。

### 2.3 命令安全边界

- `command_v1` 只能由已有 Bearer 管理员 API 创建。
- 首版命令任务必须指定精确设备 H 公钥；不能广播任意命令到所有设备。
- payload 使用 `program + args[]`，不接受 shell 字符串、重定向、管道或命令拼接。
- 设备端必须显式开启工作领取，并配置允许执行的程序基名；服务端声明不能扩大设备授权。
- 子进程清空继承环境，只加入最小系统 PATH；stdin 关闭，不继承密钥和主程序环境。
- 不允许服务端指定工作目录、环境变量、用户、sudo、提权或后台脱离进程。
- 每个命令有明确的执行上限和输出上限；超限返回结构化失败，不阻塞后续任务。
- Linux 与 macOS 使用同一协议，但分别匹配 `platform` 和 `arch`。

## 3. 服务端合同

### 3.1 管理员创建任务

新增：

`POST /v1/nni/server/admin/work/tasks`

最小请求：

```json
{
  "task_type": "command_v1",
  "target_device_pubkey": "<完整 H 公钥>",
  "target_platform": "linux",
  "target_arch": "x86_64",
  "payload": {
    "program": "uname",
    "args": ["-a"],
    "timeout_seconds": 30
  },
  "max_attempts": 1
}
```

校验要求：

- 目标 H 必须在白名单。
- `task_type`、平台和架构使用封闭 enum/受限 token。
- program 只允许无路径基名；args 数量、单项长度和总字节数有界。
- timeout 使用 `1..300` 秒；首版命令不承担长推理生命周期。
- payload 规范化后计算 SHA-256，创建后不可修改。

新增查询：

- `GET /v1/nni/server/admin/work/tasks?limit=...&status=...`
- `GET /v1/nni/server/admin/work/tasks/<task_id>`

### 3.2 领取

心跳 request 增加有界 `worker_capabilities`：

- `protocol_version`
- `platform`
- `arch`
- `supported_task_types`
- `worker_instance_id`

服务端在签名心跳验证成功后，在同一串行状态变更边界中：

1. 正常接受心跳并完成原有记录。
2. 回收已经过期且允许重试的 work lease。
3. 查找与 H、平台、架构和 task type 匹配的 queued task。
4. CAS 更新为 leased，增加 attempt，生成不可猜测 lease token。
5. 在响应中返回一个有界 `work_assignment`。

没有任务、领取失败或任务系统内部错误不能把已经成功的心跳改为失败；响应提供独立
`work_poll_status` 和结构化错误码。

### 3.3 状态机

```text
queued -> leased -> running -> succeeded
                    |          failed
                    |          retryable_failed -> queued
                    +--------> lease_expired -> queued|failed
queued|leased -> cancelled
```

- 同一任务同一时刻只有一个有效 lease。
- 终态结果幂等；重复回报返回原终态。
- 旧 lease/attempt 的迟到结果不能覆盖新 attempt。
- lease token 只在 TLS 响应和设备私有状态中出现，数据库只保存摘要。

### 3.4 随下一次心跳签名上报结果

设备执行完成后不调用独立报告接口。下一次 `/heartbeat/request` 额外携带 H、task/attempt、
lease token 和规范化结果；服务端重新计算结果摘要并把它绑定到该次 heartbeat challenge。
`/heartbeat/verify` 验证同一次设备签名后，分别提交心跳和结果：结果提交失败不得回滚或改写
已经接受的心跳，响应通过独立 `work_report_status/error_code` 返回结果状态。

结果正文只包含：

- `status=success|failed`
- `exit_code`
- `duration_ms`
- 有界 `stdout` / `stderr`
- `stdout_truncated` / `stderr_truncated`
- 稳定 `error_code`

## 4. Agent Runtime 实现

### 4.1 配置与能力

新增中性环境配置：

- `APP_NNI_WORK_PULL_ENABLED`：默认 false，测试/目标设备显式打开。
- `APP_NNI_WORK_ALLOWED_PROGRAMS`：逗号分隔的程序基名，默认空。
- `APP_NNI_WORK_COMMAND_TIMEOUT_MAX_SECONDS`：设备端上限，不得被服务端扩大。

平台/架构由编译目标与运行主机确定，不接受服务端覆盖。

### 4.2 本地持久化

使用 `data/nni/work/` 私有目录保存：

- assignment envelope 与 payload digest。
- lease token（0600 文件，不进入日志或模型上下文）。
- `assigned/running/completed_pending_report/reported` 状态。
- 有界输出、duration、exit code 和 error code。

写入采用临时文件 + rename；进程重启后扫描未回报终态并继续回报。发现 `running` 但没有可恢复
子进程时，记为 `worker_restarted` 失败，不重复执行可能有副作用的命令。

### 4.3 执行器

- 每台设备首版并发为 1。
- assignment 先验证 schema、task type、H、平台、架构、payload digest 和 allowlist。
- 使用 `tokio::process::Command` 直接执行 program/args，不经过 shell。
- `env_clear()`，设置最小 PATH，stdin=null，stdout/stderr pipe，`kill_on_drop(true)`。
- 输出分别最多 16 KiB；继续排空管道但只保存前缀并设置 truncated，保证心跳请求有界。
- 超时杀死子进程并等待回收，记录 `command_timeout`。
- 完成后保持 `completed_pending_report`；下一次正常心跳上报，不唤醒或加速心跳。

## 5. 未来推理/训练扩展点

- `task_type` 是版本化 adapter 名，不把任务类型藏在 command 字符串中。
- `inference_v1` 将使用模型摘要、输入 artifact、资源声明、进度和可续租 lease。
- `training_v1` 将使用 shard/round/checkpoint/artifact 合同和独立长任务 worker。
- 长计算不沿用 `command_v1` 的 300 秒上限；它们通过进度与 lease 续期判断存活。
- 大模型、训练数据和结果通过 artifact broker 传递，不塞进心跳 JSON。
- 后续允许平台级候选池时，只对审核过的计算 adapter 开放；`command_v1` 始终要求精确 H。

## 6. 实施顺序

1. 新增服务端 work schema、迁移、存储方法和纯存储测试。
2. 新增管理员创建/查询 API 与校验测试。
3. 扩展 heartbeat capability 与原子领取，证明奖励路径不变。
4. 把待回报结果摘要绑定到下一次 heartbeat challenge，在 heartbeat verify 内独立提交结果，
   增加幂等与迟到 attempt 测试；不得新增独立结果回报 API。
5. 新增 Agent Runtime assignment/parser/local state/command executor。
6. 把 heartbeat 成功后的 assignment 交给独立 worker，并把结果接入下一次正常 heartbeat；禁止
   空闲任务轮询和任务完成后的即时网络请求。
7. 更新英文/中文架构文档与部署说明。
8. 运行两仓库测试、格式化、静态门禁和差异检查。
9. 本机部署 NNI 测试节点与 Agent Runtime，显式开启 `uname`/`printf`。
10. 服务端创建一个 Linux 定向任务，等待心跳领取，验证服务端终态、stdout、attempt 和本地审计。
11. 使用模拟 macOS capability 做合同测试；真实 macOS 部署不在本机验收范围时不伪报真实执行。

## 7. 验收标准

- 原心跳成功且奖励记录行为保持不变。
- 无任务时心跳返回 `work_assignment=null`，不产生额外执行。
- 两次正常心跳之间没有空闲 work poll 或 work report 请求。
- 命令执行与结果等待不得阻塞、延迟或改变其他心跳；结果失败只影响 work 状态。
- 未开启工作领取或程序不在 allowlist 时绝不执行，并结构化回报拒绝原因。
- Linux 定向 `uname -a` 从服务端创建、设备领取、执行、签名回报到服务端成功形成闭环。
- 错误平台、错误 H、过期 lease、重复结果、篡改结果摘要和迟到 attempt 均被拒绝。
- 命令失败/超时不影响下一次心跳，也不阻塞后续任务。
- 敏感 lease token、签名、完整 H 和管理员 token 不进入普通日志、模型输入或 UI 文案。
- 两个仓库的相关测试、格式化和 `git diff --check` 通过；已有用户改动不被覆盖。

## 8. 完成记录

完成于 2026-10-07：

- Agent Runtime：提交 `94545286d`；`cargo check -p clawd --all-targets`、4 项定向
  work executor 测试、格式化和差异检查通过。
- NNI 服务端：功能提交 `57f88fc`，部署容量修复 `b794934`，大账本启动等待修复
  `662aa2c`；定向测试 22 项、完整测试 317 项、`npm run check` 和差异检查通过。
- 本机部署：release `clawd 0.1.8` 已重启，显式启用 work pull，allowlist 仅为
  `uname,printf`；Core 已部署 `662aa2c`，schema 26，API 与结算 worker 均为 active。
- Linux 实际闭环：任务 `nni-work-f2c63b06a8a8b0ff6b54bdb43fe09c66` 在 20:11:16
  随正常心跳领取，`printf` 独立执行成功并保持 `completed_pending_report`；执行后没有额外
  心跳或报告请求，20:21:09 随下一次正常心跳签名回报，服务端终态为 `succeeded`、
  attempt=1、stdout=`heartbeat-work-live-ok`，本地 pending 状态随后清除，心跳无失败。
- macOS：平台/架构、错误目标、租约、摘要和报告合同已由服务端/运行端自动测试覆盖；按计划
  不把本轮未进行的真实 macOS 命令执行伪报为实机验收。

## 9. 2026-10-08 能力任务扩展

在保留 `command_v1` 受限命令 adapter 的基础上，增加 `capability_v1`，使管理员可通过同一
心跳链路调用设备当前 registry generation 中已经准入、启用并获得宿主授权的能力：

- 内置工具和内置技能统一使用 `call_capability`，不得在心跳模块按工具名或技能名分支。
- 外置技能只有完成 admission、启用、policy grant，并把 capability 投影到当前 generation
  后才可调用；仅安装文件、仅构建成功或自行声明低风险均不构成运行授权。
- 设备把任务转换为现有 `run_capability` 本地 ask 任务，继续经过
  `CapabilityResolver`、`PlanVerifier`、当前管理员权限、sandbox、receipt、policy 和精确版本
  检查，不从 NNI payload 直接启动技能进程或工具。
- `args` 必须是有界 JSON object；任务结果只保留有界、清洗后的用户可见文本，不上报凭据、
  原始内部结果或 lease token。
- 长能力任务不使用 `command_v1` 的 300 秒超时。设备在正常心跳里携带 active lease 摘要续租；
  本地任务 ID 持久化，进程重启后继续观察同一任务，心跳本身不等待能力执行完成。
- 管理页面默认创建“工具或技能”任务，并保留显式确认；空设备选择表示首台匹配设备，空平台
  默认为 Linux，空架构默认为树莓派常用的 `aarch64`。

扩展验收要求：

- 内置工具、内置技能及已准入的外置技能使用同一 resolver/verifier 链路。
- 未安装、禁用、未授权、receipt/generation 不匹配或参数不符合合同的能力必须结构化失败。
- 长任务执行期间 lease 可随正常心跳续期，且不阻塞心跳、奖励或其他 NNI 数据处理。
- Core、管理网页和 Agent Runtime 的协议、迁移、权限、入队、续租、重启恢复和结果回报测试通过。

### 9.1 能力任务完成记录

完成于 2026-10-08：

- Agent Runtime 提交 `ea3651684`，架构文档提交 `d95b34b8c`；NNI 服务端提交
  `425f190`；MatrixAI 管理页提交 `26837d1`，均已推送。
- Core 已部署 `425f190`，API 与 settlement worker 为 active，结算健康检查为
  `caught_up`、`pending_periods=0`、`lag_seconds=0`；管理网页已部署 `26837d1`。
- 树莓派通过签名预编译 release `pi-aarch64-20261008` 更新，不在设备上编译；
  `agent-runtime.service` 为 active，并且只开启 `capability_v1`，未开放任意系统命令 allowlist。
- 真实内置技能闭环：Core 任务 `nni-work-120ded06c45438ba844eb52167179751`
  随心跳在树莓派领取并调用 `web.search_results`，83 秒完成；下一次正常签名心跳回报后，
  Core 终态为 `succeeded`、attempt=1，结果摘要已落库。
- 真实内置工具执行：Core 任务 `nni-work-29c13f1a4b5a2b46911ad9f13e7d100a`
  随心跳在树莓派领取并调用 `filesystem.find_entries`，成功找到
  `configs/product_identity.toml`；下一次签名心跳回报后，Core 终态为 `succeeded`、
  attempt=1，结果摘要已落库。执行使用与外置技能相同的本地管理员 task、
  `CapabilityResolver` 和 `PlanVerifier` 链路。
- 外置技能没有独立旁路或名称分支；只有 admission、enable、policy grant、receipt 与当前
  generation 全部有效并投影出 capability 后，才能通过同一 `capability_v1` 合同运行。
- 验证通过：Rust 定向测试 7 项、NNI 定向测试 12 项、NNI 完整测试 322/323（唯一一次
  settlement worker 并发套件抖动单测随后独立通过）、管理后端/服务测试 16 项、管理组件
  测试 1 项、前端 lint 与 production build、三仓库 `git diff --check`。
