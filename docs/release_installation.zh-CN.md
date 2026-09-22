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

推荐下载发行仓库里的 `install-latest-release.sh`，运行下面的命令；把
`OWNER/REPO` 换成发行仓库路径（见 `configs/product_identity.toml`）：

```bash
bash install-latest-release.sh --repo OWNER/REPO
```

默认安装到 `~/agent-runtime`，自动选择本机平台的最新正式 Release，验证签名后
安装并启动。无需 Git 或 Rust，不编译源码，不主动配置 nginx。需要自选路径时加
`--root /path/to/agent-runtime`；只检查版本用 `--check-only`，只安装不启动用
`--no-start`。首次引导文件来自指定 GitHub 仓库的 HTTPS 地址，并固定到同一提交；
请先核对仓库和脚本来源。已有安装继续使用原有可信签名公钥。

也可以手动验证安装：

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

安装器只校验并使用已有二进制和 UI，不编译缺失文件。

在“大模型”设置中选择厂商并填写 API Key，保存后写入本机
`.agent-runtime/credentials/models.env`。Linux/macOS 上目录权限为 `0700`、
文件权限为 `0600`。密钥不会回传到浏览器或写入 `config.toml`；输入框留空时保留
已有密钥。“测试连接”只使用当前草稿，不保存。

每次启动时，启动器先加载外部环境脚本（`APP_RUNTIME_ENV_SCRIPT`，默认
`$HOME/runtime_env_filled.sh`），再加载这份模型环境文件。UI 保存的 Key 优先于
同一厂商的外部 Key；没有保存覆盖值的厂商继续使用原有凭据来源。文件采用按字面
读取的 `NAME=value` 格式，不执行 shell 命令，请勿自行 `source`。这是由文件权限
保护的明文文件，只能纳入私密备份，不能提交 Git 或放入 Release；运行时升级会保留。
托管中转仍由设备自动管理密钥，不需手动填写。远程填写密钥请使用 HTTPS。
使用 webd 配置的端口访问 UI，通常是 `http://127.0.0.1:8788`；本地运行无需 nginx。
通过 UI 配置通信端和按需技能。技能安装成功不代表已获得权限或自动启用。

首次启动并初始化空数据库时，网页登录用户名为 `admin`，初始密码为 `654321`。
登录后请立即在账号管理中改成至少 12 字节的强密码，再开放外网访问。
升级或重启不会覆盖已有账号、密码或禁用状态；已有管理员 Key 补建登录账号以及
恢复出厂设置仍使用随机密码。初始化凭据记录在安装目录的
`data/bootstrap-credentials.txt`，妥善保存后删除该凭据文件。

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

各平台 Release 携带所有兼容的“其他技能”。UI 一键安装时，原生技能直接校验并安装
预编译包，Node 禁止运行安装构建脚本，Python 只安装 wheel。上游只有源码包的
依赖由匹配架构的发行机器预先构建 wheel 并附上固定摘要。缺少、损坏或架构不匹配的
原生技能包必须报错，不得退回 Cargo/Go 或依赖源码编译。发行包内置 Python 3.13
和 3.14 wheel；其他解释器需要兼容的依赖包，仍不允许编译。
系统依赖、浏览器及模型文件仍可能需要下载。包内携带不代表已经安装、授权或启用。

新包发布并验证后再删除旧包，每个受支持的平台各保留一个最新正式版本。
不要只保留全局最新的单个平台，也不要删除其他平台或构建中的标签。
