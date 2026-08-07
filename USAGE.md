# Agent Runtime 使用手册

本文档是 Agent Runtime 安装、配置、启动、更新和排障的唯一操作手册。

- 项目定位、Agent Loop 和能力架构：见 `README.zh-CN.md`
- 英文项目总览：见 `README.md`
- UI 开发说明：见 `UI/README.md`
- 技能开发规范：源码仓库中的 `AGENTS.md`

## 1. 先选择部署方式

| 场景 | 推荐方式 | 是否需要 nginx | 服务管理 |
| --- | --- | --- | --- |
| 本地 Linux/macOS | Release 包或源码，直接运行 | 否 | `agentctl` 命令 |
| 树莓派 | Pi aarch64 Release 包 | 否；公网访问时可选 | `agentctl` 或 systemd |
| 云服务器 | Ubuntu x86_64 Release 包 | 公网域名推荐 | systemd |
| 开发环境 | Git 源码 | 否 | 前台命令 |

普通用户优先使用 GitHub Releases 中与机器架构匹配的预编译包。只有需要开发、
修改源码或当前平台没有预编译包时才从源码构建。

## 2. 系统要求

基础依赖：

- `bash`
- Python 3.11 或更高版本（需要标准库 `tomllib`）
- `curl`
- `tar`

源码构建还需要：

- Rust/Cargo 1.97 或更高版本
- Clang 和 libclang
- Protocol Buffers 编译器 `protoc`
- Node.js 22 与 npm

查看工具链状态，不修改系统：

```bash
bash scripts/build_toolchain_manager.sh check
```

显式更新受支持的工具链后再检查：

```bash
bash scripts/build_toolchain_manager.sh update
```

登录 UI 后，首页“系统依赖检查”也会检查运行环境、源码/UI 构建工具以及内置工具和
技能的本机依赖，并显示已安装版本。管理员可以对支持自动处理的缺失项点击“安装”；
后端只接受固定依赖编号并使用当前 Linux 包管理器或 macOS Homebrew，不执行浏览器
传入的任意命令，也不会要求浏览器提交系统密码。Linux 服务需以 root 运行或已具备
无交互 sudo 权限才会启用自动安装；否则页面只显示缺失状态和手动配置提示。安装在
后台运行，刷新页面不会丢失进行中状态。

构建脚本会按 CPU 和当前可用内存调整并发：ARM/低内存主机使用一个 Cargo job；
内存余量充足的 12-16 GiB x86 主机使用两个；更大主机保留 Cargo 默认并发。
树莓派等低内存设备不应手工提高 Cargo 或 Node.js 并发。

`install-agent-cmd.sh` 会校验这项 Python 运行时依赖；macOS 缺失时通过
Homebrew 安装当前 `python` 公式，并让 Agent Runtime 精确使用它，不受系统自带旧版
`/usr/bin/python3` 的 PATH 顺序影响。

## 3. 获取 Agent Runtime

### 3.1 使用 Release 包

下载与平台匹配的最新包：

- Ubuntu/通用 Linux x86_64：`<artifact-id>-ubuntu-x86_64-*.tar.gz`
- 树莓派 64 位：`<artifact-id>-pi-aarch64-*.tar.gz`

`<artifact-id>` 与发布仓库由发行方在 `configs/product_identity.toml` 中定义。

同时下载 `.sha256` 文件并校验：

```bash
sha256sum -c <archive>.sha256
tar -xzf <archive>
cd <extracted-directory>
```

Release 包包含预编译二进制和 UI 静态资源，不需要在目标机器重新编译。

### 3.2 使用 Git 源码

```bash
git clone <repository-url>
cd <source-directory>
```

更新现有工作区：

```bash
git pull --ff-only
```

有本地修改时先查看 `git status`，不要用强制覆盖命令丢弃配置或数据。

## 4. 首次配置

主要配置入口：

- `configs/config.toml`：模型、工具权限、技能开关、运行时设置
- `configs/skills_registry.toml`：技能和 capability 注册信息
- `configs/channels/*.toml`：Telegram、微信、飞书、WhatsApp、webd
- `configs/image.toml`、`audio.toml`、`memory.toml` 等：专项能力配置

密钥优先放入仓库外的环境脚本，例如：

```bash
export MINIMAX_API_KEY="..."
```

启动前加载：

```bash
export APP_RUNTIME_ENV_SCRIPT=/absolute/path/runtime_env_filled.sh
```

不要把真实密钥、登录密码、Token 或设备私钥提交到 Git。

无交互启动前至少确认：

- `llm.selected_vendor`
- `llm.selected_model`
- 所选 provider 的 API key
- 需要启用的通道配置

## 5. 安装命令入口

本地用户安装，不配置 nginx：

```bash
bash install-agent-cmd.sh --user --no-deploy-ui
```

从源码构建后安装：

```bash
bash install-agent-cmd.sh --build --user --no-deploy-ui
```

检查结果：

```bash
command -v agentctl
agentctl -h
agentctl -status
command -v clawcli
```

删除安装器创建的 `agentctl` / `clawcli` 链接不会删除工作区、配置或数据；systemd
部署应先按第 8 节卸载 unit。当前不提供会删除工作区或数据的一键卸载脚本。

## 6. 从源码构建

完整 release 构建：

```bash
./build-all.sh
```

跳过 UI：

```bash
./build-all.sh no-ui
```

只验证 Rust 代码：

```bash
cargo check --workspace
```

旧的四套 cross 编译入口已归档到 `scripts/archive/cross-build/`，当前部署流程
不再主动使用它们。需要恢复旧流程时，先阅读该目录的 `README.md` 并重新做
工具链与目标设备验证；日常构建继续使用 `build-all.sh` 或发布包部署。

按需安装的 Skill Store 技能不会被普通全量构建主动编译；它们只在 UI 安装或
开发者明确指定单个 package 时编译。

## 7. 启动与停止

最简快速启动：

```bash
agentctl start -q
```

带 UI 启动：

```bash
agentctl -start release all --with-ui
```

指定模型：

```bash
agentctl -start --vendor minimax --model MODEL_NAME --profile release --channels all
```

常用运维：

```bash
agentctl -status
agentctl -health
agentctl -logs clawd 200 --follow
agentctl -restart release all --quick --skip-setup
agentctl -stop
```

不安装命令入口时也可直接运行：

```bash
./start-all-bin.sh release
./stop-agent.sh
```

### 7.1 在终端中连续使用 Agent

启动或恢复最近一次 CLI 会话：

```bash
clawcli chat
```

新建会话或恢复指定会话：

```bash
clawcli chat --new
clawcli chat --conversation-id CONVERSATION_ID
```

启动时附加文件或图片：

```bash
clawcli chat --file README.md --image path/to/image.png
```

会话内可用 `/model`、`/permissions`、`/compact`、`/diff`、`/file`、
`/image`、`/attachments`、`/goal`、`/resume` 和 `/resume-task` 等显式
命令；运行 `/help` 查看当前完整列表。`@path` 也是显式文件引用语法，不会按
自然语言短语猜测路径。附件限制为最多 10 个、单文件 20 MiB、总计 60 MiB，
且只能引用当前 workspace 内通过符号链接与敏感文件检查的文件。

脚本消费事件时使用严格 JSONL：

```bash
printf 'inspect the workspace\n/exit\n' | clawcli chat --jsonl
```

JSONL 模式的 stdout 每行都是独立、带版本的 JSON 对象，不包含 ANSI 或动画。
模型与权限请求仍由服务端校验；只有管理员密钥显式使用全局 `--yolo` 时，
`clawcli` 才请求无确认、`danger_full` 的执行策略。

## 8. Linux systemd 服务

仓库不保存写死用户或安装路径的 `agent-runtime.service`。Linux/systemd 主机应使用
安装器按当前环境生成 unit：

```bash
bash scripts/install-systemd-service.sh --print
```

仅渲染到文件供审查：

```bash
bash scripts/install-systemd-service.sh \
  --workspace "$PWD" \
  --user "$(id -un)" \
  --runtime-env /absolute/path/runtime_env_filled.sh \
  --output /tmp/agent-runtime.service
```

安装、启用并启动：

```bash
bash scripts/install-systemd-service.sh \
  --workspace "$PWD" \
  --user "$(id -un)" \
  --runtime-env /absolute/path/runtime_env_filled.sh \
  --enable \
  --start
```

查看状态和日志：

```bash
sudo systemctl status agent-runtime.service
sudo journalctl -u agent-runtime.service -n 200
sudo journalctl -u agent-runtime.service -f
```

卸载：

```bash
bash scripts/install-systemd-service.sh --uninstall
```

macOS、本地无 systemd 环境和容器内应使用直接进程启动；安装器会返回明确的
unsupported 错误，不会尝试 Linux 服务命令。

## 9. UI 与云服务器

本地部署由 `webd` 提供 UI、登录会话和到 loopback `clawd` 的 API 代理，
不需要 nginx。`clawd` 固定监听 `127.0.0.1:8787`，不得对外暴露。

首页的“webd 对外端口”开关控制浏览器是否能通过设备 IP 和 webd 端口直接访问：

- 开启：监听 `0.0.0.0:<当前端口>`，适合可信局域网直连，但必须结合防火墙控制访问范围。
- 关闭：监听 `127.0.0.1:<当前端口>`；同机 nginx 仍可代理 UI/API，直接访问 `IP:<端口>` 会断开。
- 切换时只修改 `configs/channels/webd.toml` 的监听地址并保留端口，随后短暂重启入口服务。

UI 开发服务器：

```bash
cd UI
npm ci
npm run dev
```

UI 生产检查：

```bash
cd UI
npm run lint
npm run build
```

云服务器使用域名和 TLS 时，可显式部署静态 UI 到 nginx：

```bash
bash install-agent-cmd.sh \
  --deploy-ui-nginx /var/www/html/agent-runtime
```

脚本会保留已有 TLS 站点配置，不会用默认 HTTP 模板覆盖已管理的证书配置。
nginx 只负责静态资源以及 `/v1/`、`/webd/` 反向代理；Agent Runtime 进程仍由
systemd 或直接启动方式管理。

首页的“Nginx Web 入口配置”提供两个管理动作：

- “启用/修复 nginx”会在 Linux/macOS 上检查并按需安装或更新 nginx，修复站点、
  启动服务并部署当前 `UI/dist`。
- “关闭 nginx”会停止并禁用服务，删除 Agent Runtime 站点和专用 UI 部署，但不卸载
  nginx。云服务器通过该入口访问时会立即断开，执行前必须确认仍有服务器终端或直连
  `webd` 的恢复通道。

## 10. 身份与 Key

```bash
agentctl -key list
agentctl -key generate user
agentctl -key generate admin
agentctl -key add rk-xxxx admin
agentctl -key disable rk-xxxx
agentctl -key enable rk-xxxx
```

- `user`：普通对话和允许的技能调用
- `admin`：管理操作；UI/webd 管理员会按策略获得自动批准执行能力

同一 IP 和用户名连续登录失败达到限制后会暂时锁定。不要在日志、截图或工单中
公开 key。

## 11. 树莓派小屏

仅在树莓派上安装桌面入口与登录自启：

```bash
bash install-agent-cmd.sh --user --pi-app
```

手工运行：

```bash
cd pi_app
./run-small-screen.sh
```

没有 MatrixAI 签名硬件时，Agent Runtime 主功能仍可使用，但 NNI 设备签名和网络原生
智能参与能力不可用。

## 12. 更新

UI 首页会检查与当前平台匹配的最新 Release。Release 更新会保留本地配置、数据、
日志和 `.pids` 目录，再部署新二进制与 UI。

也可以在服务器或本地运行目录中直接部署与当前 Linux 平台匹配的最新 Release：

```bash
./deploy-github-release.sh
```

脚本会选择 Ubuntu x86_64 或树莓派 aarch64 资产，强制校验配套 SHA256 文件，
保留本地配置和运行数据，为被替换的程序文件建立回滚备份，并在原服务处于运行状态时
自动重启。仅检查可用版本或由其他进程安排重启时：

```bash
./deploy-github-release.sh --check-only
./deploy-github-release.sh --no-restart
```

源码更新和 Release 更新是两条不同路径：

- 普通用户：优先 Release 更新
- 开发者：使用 Git 拉取并重新构建

Release 包安装没有 `.git` 和完整构建源码，因此 UI 默认只显示 Release 更新，不会
执行 Git 状态检查，也不会显示源码编译按钮。需要在同一设备上继续开发时，管理员可在
Release 更新卡片中选择“切换到源码模式”。该操作会先把完整仓库克隆到临时目录并验证，
再保留 `configs`、`data`、`logs`、`.pids`、外部技能和现有运行二进制，原子替换运行
目录并重启。迁移成功后 UI 才显示 Git 拉取和本机编译功能；原 Release 目录会保留为一个
回滚备份。不要在 Release 运行目录中手工执行 `git init`。

命令行执行同一迁移：

```bash
./scripts/switch-to-source-checkout.sh
./deploy-github-release.sh --package-mode
```

更新后检查：

```bash
agentctl -status
agentctl -health
```

## 13. 开发与测试

Rust：

```bash
cargo fmt --all -- --check
cargo test --workspace
python3 scripts/check_long_files.py
python3 scripts/check_cross_platform_contracts.py
```

技能 prompt 或 registry 变更：

```bash
python3 scripts/sync_skill_docs.py
python3 scripts/check_skill_prompts.py
python3 scripts/check_skill_registry_parity.py --mode all --strict
```

UI：

```bash
cd UI
npm run lint
npm run build
```

NL 测试必须使用隔离数据库/端口，打印每个 case 及编号后的原始 LLM 请求和响应。
具体入口见 `scripts/nl_tests/README.md`。

## 14. 故障排查

按以下顺序检查：

1. `agentctl -status`
2. `agentctl -health`
3. `agentctl -logs clawd 200`
4. 对应 `configs/channels/*.toml`
5. provider API key、额度和网络
6. 端口占用、磁盘和内存

常见问题：

- UI 无法访问：确认后端健康；云部署再检查 nginx/TLS/防火墙。
- 通道无响应：检查通道配置、绑定状态和对应 daemon 日志。
- Release 更新后仍显示旧版本：刷新版本状态并确认下载资产与系统架构匹配。
- 树莓派编译 OOM：使用 Release 包；必须构建时保持低并发并启用足够 swap。
- systemd 启动失败：先用 `--print` 审查生成 unit，再查看 journal。
- 签名芯片不可用：确认设备是否安装 MatrixAI 硬件、I2C 和 cryptoauthlib。

报告问题时提供结构化状态、错误码、相关日志时间段和系统信息，不要附带密钥。
