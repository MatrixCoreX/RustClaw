# RustClaw 使用手册

本文档是 RustClaw 安装、配置、启动、更新和排障的唯一操作手册。

- 项目定位、Agent Loop 和能力架构：见 `README.zh-CN.md`
- 英文项目总览：见 `README.md`
- UI 开发说明：见 `UI/README.md`
- 技能开发规范：源码仓库中的 `AGENTS.md`

## 1. 先选择部署方式

| 场景 | 推荐方式 | 是否需要 nginx | 服务管理 |
| --- | --- | --- | --- |
| 本地 Linux/macOS | Release 包或源码，直接运行 | 否 | `rustclaw` 命令 |
| 树莓派 | Pi aarch64 Release 包 | 否；公网访问时可选 | `rustclaw` 或 systemd |
| 云服务器 | Ubuntu x86_64 Release 包 | 公网域名推荐 | systemd |
| 开发环境 | Git 源码 | 否 | 前台命令 |

普通用户优先使用 GitHub Releases 中与机器架构匹配的预编译包。只有需要开发、
修改源码或当前平台没有预编译包时才从源码构建。

## 2. 系统要求

基础依赖：

- `bash`
- `python3`
- `curl`
- `tar`

源码构建还需要：

- Rust/Cargo
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

构建脚本会按 CPU 和可用内存调整并发；树莓派等低内存设备不应手工提高 Cargo
或 Node.js 并发。

## 3. 获取 RustClaw

### 3.1 使用 Release 包

下载与平台匹配的最新包：

- Ubuntu/通用 Linux x86_64：`RustClaw-ubuntu-x86_64-*.tar.gz`
- 树莓派 64 位：`RustClaw-pi-aarch64-*.tar.gz`

同时下载 `.sha256` 文件并校验：

```bash
sha256sum -c RustClaw-*.tar.gz.sha256
tar -xzf RustClaw-*.tar.gz
cd RustClaw
```

Release 包包含预编译二进制和 UI 静态资源，不需要在目标机器重新编译。

### 3.2 使用 Git 源码

```bash
git clone https://github.com/MatrixCoreX/RustClaw.git
cd RustClaw
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
export RUSTCLAW_RUNTIME_ENV_SCRIPT=/absolute/path/runtime_env_filled.sh
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
bash install-rustclaw-cmd.sh --user --no-deploy-ui
```

从源码构建后安装：

```bash
bash install-rustclaw-cmd.sh --build --user --no-deploy-ui
```

检查结果：

```bash
command -v rustclaw
rustclaw -h
rustclaw -status
command -v clawcli
```

卸载命令入口不会删除配置和数据：

```bash
bash uninstall-rustclaw-cmd.sh --user
```

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
CARGO_BUILD_JOBS=1 cargo check --workspace
```

树莓派 64 位交叉构建：

```bash
./cross-build-pi.sh --target pi64 --workspace
```

树莓派 32 位交叉构建：

```bash
./cross-build-pi.sh --target pi32 --workspace
```

按需安装的 Skill Store 技能不会被普通全量构建主动编译；它们只在 UI 安装或
开发者明确指定单个 package 时编译。

## 7. 启动与停止

最简快速启动：

```bash
rustclaw start -q
```

带 UI 启动：

```bash
rustclaw -start release all --with-ui
```

指定模型：

```bash
rustclaw -start --vendor minimax --model MODEL_NAME --profile release --channels all
```

常用运维：

```bash
rustclaw -status
rustclaw -health
rustclaw -logs clawd 200 --follow
rustclaw -restart release all --quick --skip-setup
rustclaw -stop
```

不安装命令入口时也可直接运行：

```bash
./start-all-bin.sh release
./stop-rustclaw.sh
```

## 8. Linux systemd 服务

仓库不保存写死用户或安装路径的 `rustclaw.service`。Linux/systemd 主机应使用
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
  --output /tmp/rustclaw.service
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
sudo systemctl status rustclaw.service
sudo journalctl -u rustclaw.service -n 200
sudo journalctl -u rustclaw.service -f
```

卸载：

```bash
bash scripts/install-systemd-service.sh --uninstall
```

macOS、本地无 systemd 环境和容器内应使用直接进程启动；安装器会返回明确的
unsupported 错误，不会尝试 Linux 服务命令。

## 9. UI 与云服务器

本地部署由 `clawd`/`webd` 直接提供 UI，不需要 nginx。

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
bash install-rustclaw-cmd.sh \
  --deploy-ui-nginx /var/www/html/rustclaw
```

脚本会保留已有 TLS 站点配置，不会用默认 HTTP 模板覆盖已管理的证书配置。
nginx 只负责静态资源以及 `/v1/`、`/webd/` 反向代理；RustClaw 进程仍由
systemd 或直接启动方式管理。

## 10. 身份与 Key

```bash
rustclaw -key list
rustclaw -key generate user
rustclaw -key generate admin
rustclaw -key add rk-xxxx admin
rustclaw -key disable rk-xxxx
rustclaw -key enable rk-xxxx
```

- `user`：普通对话和允许的技能调用
- `admin`：管理操作；UI/webd 管理员会按策略获得自动批准执行能力

同一 IP 和用户名连续登录失败达到限制后会暂时锁定。不要在日志、截图或工单中
公开 key。

## 11. 树莓派小屏

仅在树莓派上安装桌面入口与登录自启：

```bash
bash install-rustclaw-cmd.sh --user --pi-app
```

手工运行：

```bash
cd pi_app
./run-small-screen.sh
```

没有 MatrixAI 签名硬件时，RustClaw 主功能仍可使用，但 NNI 设备签名和网络原生
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

更新后检查：

```bash
rustclaw -status
rustclaw -health
```

## 13. 开发与测试

Rust：

```bash
cargo fmt --all -- --check
CARGO_BUILD_JOBS=1 cargo test --workspace
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

1. `rustclaw -status`
2. `rustclaw -health`
3. `rustclaw -logs clawd 200`
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
