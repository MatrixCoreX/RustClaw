# 用户与部署 Agent 的 Release 安装说明

[English](release_installation.md)

安装和更新统一使用签名验证通过的 GitHub Release 包。不要在目标设备编译，
不要自动安装 Rust 工具链，缺少匹配包时也不要改用源码编译。源码构建仅供
技术人员明确选择，见[开发者构建说明](developer_build.md)。

## 选择正确的包

发行仓库和包名前缀由 `configs/product_identity.toml` 定义。选择对应平台最新的
正式 Release，不使用草稿、预发布版本或其他平台的全局 latest。

| 设备 | Release 标签前缀 | 目标平台 |
| --- | --- | --- |
| Linux x86_64 / 云服务器 | `ubuntu-x86_64-` | `x86_64-unknown-linux-gnu` |
| 64 位树莓派 / Linux aarch64 | `pi-aarch64-` | `aarch64-unknown-linux-gnu` |
| Intel Mac | `macos-x86_64-` | `x86_64-apple-darwin` |
| Apple Silicon Mac | `macos-aarch64-` | `aarch64-apple-darwin` |

先检查 `uname -s`、`uname -m`、可用磁盘和发行说明的系统版本要求。
32 位树莓派系统不能使用 aarch64 包。运行环境需要 Bash、Python 3.11+、curl、
tar 和 OpenSSH 的 `ssh-keygen`；技能的额外依赖按需安装。

## 首次安装

1. 从同一个 Release 下载压缩包和 `.sha256`、`.spdx.json`、`.manifest.json`、
   `.manifest.json.sig` 四个配套文件。
2. 从独立可信的发行方来源取得 `configs/release_allowed_signers` 和
   `scripts/security/release_manifest.py`。不能仅依靠未验证压缩包里附带的公钥
   建立信任。先按[英文版验证命令](release_installation.md#first-installation)
   验证签名、清单、包摘要和目标架构，再解压；验证失败立即停止。
3. 解压到新的用户可写目录，不覆盖已有运行目录，进入包目录后执行：

```bash
bash install-agent-cmd.sh --user --no-deploy-ui
export APP_RUNTIME_ENV_SCRIPT=/absolute/path/runtime_env_filled.sh
agentctl start -q
agentctl -status
agentctl -health
```

安装器只校验并使用已有二进制和 UI，不编译缺失文件。模型密钥放在安装目录外。
使用 webd 配置的端口访问 UI，通常是 `http://127.0.0.1:8788`；本地运行无需 nginx。
通过 UI 配置通信端和按需技能。技能安装成功不代表已获得权限或自动启用。

## 更新现有设备

使用 UI 的 Release 更新，或在现有可信运行目录执行：

```bash
bash deploy-github-release.sh --check-only
bash deploy-github-release.sh --restart
```

脚本自动选择平台，验证签名、checksum、manifest、SBOM 和二进制架构，保留本地
配置和运行数据，再重启服务。已经部署 nginx 的设备同步包内 UI；本地没有 nginx
时不会主动配置。需要指定版本时添加 `--tag TAG`。

安装任务不得执行 `git pull`、`cargo build`、`npm run build`，也不得调用
`install-agent-cmd.sh --build`。缺包、网络或验证失败时报告真实原因，等待对应
Release，不要切换为现场编译。

更新前确认备份和磁盘能容纳下载、暂存及回滚副本；更新后检查服务健康、UI、
已配置通信端、已安装技能和版本显示。保留配置、钱包、数据库、技能数据及最近
回滚副本，不要为安装而清理业务数据或构建缓存。

## 发布保留规则

新包发布并验证后再删除旧包，每个受支持的平台各保留一个最新正式版本。
不要只保留全局最新的单个平台，也不要删除其他平台或构建中的标签。
