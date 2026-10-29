# Ubuntu x86_64 Release 包

用户与部署 Agent 按[Release 安装说明](release_installation.zh-CN.md)安装签名预编译包，不在目标设备编译。下方构建和发布流程仅供维护者使用。

Agent Runtime 可以通过 GitHub Actions 发布预编译 Ubuntu x86_64 runtime 包，适用于普通 64 位 Ubuntu 云服务器和 PC。

## 构建与发布

推荐命令：

```bash
./release-latest.sh --platform ubuntu
```

脚本创建并推送下一个 `ubuntu-x86_64-YYYYMMDD[-N]` tag，触发 `Build Ubuntu x86_64 Release` 并发布正式 GitHub Release。也可以手动运行 workflow；只有测试包不应成为普通更新源时才设置 `prerelease=true`。

只构建签名验收候选包、不创建或更新 GitHub Release：

```bash
gh workflow run ubuntu-x86_64-release.yml -f publish_release=false
```

记录该次运行的源码 SHA，整轮成功后下载 `agent-runtime-ubuntu-x86_64` Actions artifact。
此模式保留相同的构建、测试、签名、SBOM 和 provenance 校验，不部署、不创建 tag、
不发布或清理 Release。仅设置 `prerelease=true` 仍会发布 Release，并非只构建模式。
手动运行默认仍发布；显式推送发布 tag 的行为也保持不变。

Workflow 构建：

- `x86_64-unknown-linux-gnu` Rust workspace 二进制；
- `UI/dist`；
- 依据所选 product identity 的 `release_artifact_id`、平台和 tag 命名的 runtime archive；
- 对应 `.sha256`、`.spdx.json`、`.manifest.json` 和 `.manifest.json.sig`。

Archive 上传为 workflow artifact；仅在启用发布时上传为 GitHub Release asset。

旧 Release 默认保留，只有维护者显式运行独立清理 workflow 才会删除。删除某个平台的
旧版前须验证替代版本；只构建候选包不会触发清理。

## 更新说明

已有安装必须保留：

- `configs/`
- `data/`
- `logs/`
- `.pids/`

`data/` 同时包含主运行时数据库和 `data/skills/` 下的技能私有库；更新时必须
保留整个目录，不能只保留 `agent-runtime.db`。

更新包提供二进制、脚本、prompt、migration 和 `UI/dist`，不得用包内默认值覆盖线上 secret 或 channel 设置。

Admin Release 更新路径验证 checksum、保留 runtime 目录，并通过原子替换逐个更新预编译二进制，避免覆盖正在运行的 `clawd` 时出现 `Text file busy`，随后重启 `clawd`。systemd 托管的 Linux 安装会用独立 transient unit 调度重启，避免旧 service 停止时同时杀死自己的重启进程。主机已有 Agent Runtime nginx site 时，只复制包内 `UI/dist`，不重新构建；没有 nginx site 的本地安装不会被配置 nginx。
