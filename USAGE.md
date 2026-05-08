# RustClaw 使用说明

本文件面向仓库使用者和开发者，说明当前推荐的安装、构建、启动、排障方式。

## 1. 环境准备

基础依赖：

- `git`
- `bash`
- `python3`
- `rustup` / `cargo`

按需依赖：

- `npm`：构建 `UI/` 时需要
- `sqlite3`：本地查库排障时有用
- `nginx`：需要静态部署 UI 时需要

建议系统：

- Linux
- macOS

## 2. 先看哪些文件

初次使用建议先看这些配置：

- `configs/config.toml`
- `configs/skills_registry.toml`
- `configs/channels/*.toml`

常见拆分配置：

- `configs/image.toml`
- `configs/audio.toml`
- `configs/crypto.toml`
- `configs/memory.toml`

## 3. 安装与构建

### 3.1 安装 `rustclaw` 启动器

如果你只是想在本机快速运行，推荐：

```bash
bash install-rustclaw-cmd.sh --user --no-deploy-ui
```

如果你要从源码构建后再安装：

```bash
bash install-rustclaw-cmd.sh --build --user --no-deploy-ui
```

安装后可检查：

```bash
command -v rustclaw
rustclaw -h
rustclaw -status
```

卸载启动器：

```bash
bash uninstall-rustclaw-cmd.sh --user
```

### 3.2 从源码构建

推荐入口：

```bash
./build-all.sh
```

跳过 UI 构建：

```bash
./build-all.sh no-ui
```

树莓派交叉编译（默认 64 位系统）：

```bash
./cross-build-pi.sh

# 32 位 Raspberry Pi OS
./cross-build-pi.sh --target pi32

# 只编译单个包，便于快速验证
./cross-build-pi.sh --package clawd
```

或直接使用 Cargo：

```bash
cargo build --workspace --release
```

说明：

- `build-all.sh` 会先同步技能文档
- 默认构建 release
- 如果 `UI/dist` 已存在且可复用，会尽量跳过 UI 重建
- `cross-build-pi.sh` 会设置 Raspberry Pi 目标的 linker、`cc`、bindgen 头文件参数，并调用现有构建流程；64 位产物在 `target/aarch64-unknown-linux-gnu/release/`，32 位产物在 `target/armv7-unknown-linux-gnueabihf/release/`

## 4. 启动方式

### 4.1 推荐：使用 `rustclaw`

```bash
# 启动时在终端里配置通信端，然后直接启动
rustclaw -start release all

# 带 UI 启动
rustclaw -start release all --with-ui

# 查看状态、日志、健康检查、停止
rustclaw -status
rustclaw -logs clawd 200 --follow
rustclaw -health
rustclaw -stop
```

### 4.2 使用脚本启动

```bash
./start-all.sh
./stop-rustclaw.sh
```

按服务单独启动：

```bash
./component_start/start-clawd.sh
./component_start/start-telegramd.sh
./component_start/start-wechatd.sh
./component_start/start-feishud.sh
./component_start/start-larkd.sh
./component_start/start-whatsappd.sh
./component_start/start-whatsapp-webd.sh
./component_start/start-clawd-ui.sh
```

### 4.3 首次启动的注意事项

`component_start/start-clawd.sh` 当前有一套首启保护逻辑：

- 如果 `configs/config.toml` 里的 `llm.selected_vendor` 或 `llm.selected_model` 为空，首次启动会要求交互选择
- 如果当前选中的厂商 `api_key` 为空，也会要求交互输入

因此在无交互环境里启动前，最好先把以下内容填好：

- `llm.selected_vendor`
- `llm.selected_model`
- 对应 `llm.<vendor>.api_key`

## 5. 当前通道配置

仓库里当前存在这些通道配置文件：

- `configs/channels/telegram.toml`
- `configs/channels/wechat.toml`
- `configs/channels/feishu.toml`
- `configs/channels/lark.toml`
- `configs/channels/whatsapp.toml`
- `configs/channels/whatsapp-web.toml`
- `configs/channels/whatsapp-cloud.toml`
- `configs/channels/webd.toml`

这并不意味着所有通道都会被 `rustclaw -start` 一次性管理；具体是否启用，还要看你的配置和启动方式。

## 6. UI 使用方式

浏览器控制台项目位于 `UI/`。

本地开发：

```bash
cd UI
npm install
npm run dev
```

构建：

```bash
cd UI
npm run build
npm run lint
```

相关入口：

```bash
./build-ui-nginx.sh
bash install-rustclaw-cmd.sh
```

如果不想部署 nginx，请显式使用：

```bash
bash install-rustclaw-cmd.sh --user --no-deploy-ui
```

## 7. Key 管理

常用命令：

```bash
rustclaw -key list
rustclaw -key generate user
rustclaw -key generate admin
rustclaw -key add rk-xxxx admin
rustclaw -key disable rk-xxxx
rustclaw -key enable rk-xxxx
```

RustClaw 使用 `user_key` 作为跨 UI 和消息通道的主身份标识。

## 8. 常用开发命令

```bash
# 构建
cargo build --workspace --release

# 测试
cargo test

# Clippy
cargo clippy --workspace --all-targets --all-features
```

前端检查：

```bash
cd UI
npm run lint
npm run build
```

## 9. 回归与辅助脚本

仓库里有不少实用脚本，常见的有：

```bash
./simulate-telegramd.sh
./check-logs.sh
./system_report.sh
./package-release.sh
```

`scripts/skill_calls/` 下也提供了很多技能调用示例脚本。

## 10. 故障排查

建议排查顺序：

1. 看启动脚本输出
2. 看 `logs/` 下对应服务日志
3. 用 `rustclaw -status` 看进程状态
4. 用 `rustclaw -health` 或 `curl /v1/health` 看后端是否存活
5. 再检查配置、端口占用、上游 API 可达性

常见问题：

- 启动时报模型或 key 缺失：先检查 `configs/config.toml`
- UI 打不开：先确认 `UI/dist` 是否存在，或开发服务器是否启动
- 通道无响应：先检查对应 `configs/channels/*.toml` 和对应守护进程日志
- 小屏程序无界面：先确认图形会话、`DISPLAY`、`python3-tk` 和 `clawd` 已启动

## 11. Pi App 小屏程序

小屏桌面程序位于 `pi_app/`。
`install-rustclaw-cmd.sh --pi-app` 只会在检测到树莓派时配置桌面快捷方式和登录自启动；普通电脑会自动跳过。

```bash
cd pi_app
./run-small-screen.sh
./install-desktop.sh
./enable-autostart.sh
./disable-autostart.sh
./open-small-screen.sh
```

它依赖 `clawd` 的健康接口，因此后端要先启动。

## 12. 配置与安全

- 不要提交真实密钥、Token、密码
- 提交前检查配置 diff，避免把本地环境信息一起提交
- 生产环境建议重新审视 `configs/config.toml` 中的监听地址、工具权限和超时配置
- 如果使用 nginx 暴露 UI，请同时检查防火墙和公网入站规则
