# RustClaw 多语言技能 SDK 合同

新的 RustClaw 技能包使用版本为 2 的 `skill.toml`。v1 只作为集中式只读兼容
输入，新源码安装在激活前会规范化成 v2。manifest 除包、构建和运行信息外，
还声明类型化 capability/permission request，包括输入/输出 schema、effect、
execution mode、artifact、evidence、timeout、配置入口和运行资源申请。request
绝不等于 grant；risk、自动调用、确认、真实凭据和最终权限只能由宿主 policy 决定。

支持 Cargo、Python、Node、Go、预编译原生程序、通用进程和类型化
`http_json` 适配器。清单只有类型化字段，不允许写任意 Shell 命令。构建
网络默认关闭，依赖必须位于技能自己的安装根目录；缺少沙箱时必须失败关闭。

所有本地技能进程使用 `rustclaw-jsonl-v1`：stdin 一条 JSON 请求，stdout
严格一条 JSON 响应，诊断信息写 stderr。响应必须回显 `request_id`；错误必须
同时提供可读 `error_text`、稳定的 `extra.error_code` 和
`extra.message_key`。单条记录最多 1 MiB。

安装成功后生成不可变 v2 回执，记录 manifest、语义合同、源码、锁文件和产物摘要、
精确平台、适配器版本、协议冒烟结果及可信启动信息。宿主准入回执再把它与 policy
grant 摘要和 registry generation 绑定。回执只能保存凭据引用，不能保存 secret
值。激活时原子更新 `current.json`，并保留 `previous.json` 用于有限回滚。技能业务
数据不得写入安装回执目录。

执行前，`SkillRuntimeResolver` 会验证当前指针、回执、manifest 和每个产物，
再生成 `SkillLaunchSpec`。planner 参数不能覆盖程序、入口、工作目录、环境、
沙箱或回执身份。

机器 schema 位于 `docs/schemas/`。`rustclaw-skill` 命令以单个 JSON 结果提供
CI 友好的清单、协议和回执检查。

## 包目录与唯一权威

每个进程技能包都包含 `skill.toml` 和 `INTERFACE.md`。manifest 负责版本、
实现 adapter、源码/锁文件路径、支持平台、类型化启动信息、构建网络策略、沙箱、
存储和生命周期文件，并提出 planner 能力与运行资源申请；宿主 registry/policy
负责校验和收窄申请，并唯一拥有 risk、确认、自动调用、凭据解析、别名与 prompt
准入。registry 使用 `package_manifest` 引用清单。

```text
my_skill/
├── skill.toml
├── INTERFACE.md
├── README.md
├── adapter 选择的源码与锁文件
└── tests/（或该语言独立的测试目录）
```

固定/核心包放在 `crates/skills/`，bundled 按需包放在 `optional_skills/`，
外部提交放在 `external_skills/`。只有仓库维护的 core/bundled Cargo 包加入根
Cargo workspace；external Cargo 始终保持 standalone workspace。

## CLI 快速开始

先构建一次 SDK CLI，再显式选择语言：

```bash
CARGO_BUILD_JOBS=1 cargo build -p rustclaw-skill-sdk --bin rustclaw-skill
target/debug/rustclaw-skill init python demo_skill external_skills/demo_skill --human
target/debug/rustclaw-skill validate external_skills/demo_skill/skill.toml --human
target/debug/rustclaw-skill build external_skills/demo_skill/skill.toml . data/skill-packages --human
target/debug/rustclaw-skill receipt-verify data/skill-packages demo_skill --human
```

`build`、`protocol-test` 与 `install-local` 都走同一条验证安装链路。只有
manifest 声明 `network = "approval_required"` 且人工明确审查后才增加
`--network`。打包任务可以增加 `--target <triple>`；跨目标协议冒烟必须有受支持
的模拟器，否则失败关闭。默认输出机器 JSON；`--human` 只提供简短开发者摘要。

## 各语言快速说明

- Rust：使用 `init rust`；保留 `Cargo.lock`。只有仓内 bundled Cargo 包加入
  workspace，并明确声明 Cargo package/binary 身份。
- Python：使用 `init python`；保留声明的 `requirements.lock`。安装创建技能
  私有 venv，禁止用户级或全局 site-packages。
- Node：使用 `init node`；提交 `package-lock.json`。依赖安装在技能私有根，
  所有受支持 schema 都会禁用并拒绝依赖生命周期脚本。
- Go：使用 `init go`；提交 `go.mod` 与 `go.sum`。adapter 使用隔离缓存并产出
  一个目标平台可执行文件，禁止全局 `go install`。
- Prebuilt：使用 `init prebuilt`；为精确 OS/架构声明产物、SHA-256、可选大小、
  archive 类型和入口。平台必须精确匹配。
- Generic process：用于本地已经构建好的 JVM/.NET/native 产物，只允许类型化
  launcher 与参数向量，禁止任意 shell 字符串。
- `http_json`：必须是无凭据 HTTPS endpoint，声明构建网络审批和运行网络权限；
  禁止重定向，密钥只能由 registry 能力在运行时按 scope 提供。

所有实现读取同一组请求字段（`request_id`、`args`、`context`、`user_id`、
`chat_id`），并只输出一条响应。失败必须包含 `status="error"`、可读
`error_text` 与稳定的 `extra.error_code`/`extra.message_key`。

## 发布与注册

1. 在 `INTERFACE.md` 写全 action、类型化参数、错误和示例。
2. 运行 manifest 校验及对应 adapter 的 build/protocol smoke。
3. 运行 `python3 scripts/check_polyglot_skill_contracts.py --require-all`；修改
   prompt 后再运行 `python3 scripts/check_skill_prompts.py`。
4. 注册 `package_manifest`、planner 元数据、存储/配置所有权和生成 prompt；
   不要在 `clawd` 增加单技能分支。
5. 编译/协议通过只表示产物可进入准入；必须同时有已验证回执、明确宿主 grant 和
   registry generation 激活后才启用。外部/导入包不得按扩展名推断入口或自授权限。

## 生命周期与诊断

Skill Store 操作持久记录 queued、preflight、dependency、build、smoke、activate、
configure、success/failure/cancel 阶段。禁用保留安装包和私有数据；卸载只删除该
技能的版本化运行目录/回执，默认保留配置和私有数据。更新通过后原子切换新目录；
回滚前重新验证 previous 指针、回执、manifest 与每个产物摘要。
卸载技能不得删除共享 Rust/Cargo、Python、Node、Go、JVM/.NET 运行环境或可复用构建
缓存；工具链清理由独立管理员操作负责。

普通用户界面使用稳定 phase/code/message key；脱敏且有上限的诊断放在二级详情。
manifest、回执、操作记录与协议 fixture 都不得包含凭据、原始 provider 响应、
环境转储或隐藏推理。

## 平台

共享代码同时支持 Linux 与 macOS。native/prebuilt 包必须声明 OS 和架构。
Python、Node、generic-process 的跨目标构建返回结构化 unsupported；Go 使用明确
GOOS/GOARCH 且关闭 CGO；Cargo 与 prebuilt 遵循各自 target/artifact 声明。
低内存主机和树莓派默认串行执行重型构建。缺少沙箱、模拟器、runtime 或 toolchain
时必须失败关闭，不能退化为无隔离启动。

## 参考实现一致性

`crates/skill-sdk/tests/reference/` 提供等价的 Cargo、Python、Node、Go 与
prebuilt 夹具，验证同一套计算、结构化错误、产物、waiting、needs-user、超时、
非法/多条/超大 stdout 以及仅写 stderr 的诊断行为。测试还会对每个可用 adapter
执行原子更新、已验证回滚、失败更新保留、构建网络拒绝和源码目录无污染检查。
Ubuntu 与 macOS CI 设置 `RUSTCLAW_REQUIRE_REFERENCE_ADAPTERS=1`，因此五种
adapter 和必需沙箱必须全部实际执行，不能因缺少工具链而静默跳过。
