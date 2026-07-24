# Raspberry Pi aarch64 Release 包

RustClaw 可以通过 GitHub Actions 发布预编译树莓派包，树莓派无需本地运行 `cargo build`。

## 构建与发布

推荐命令：

```bash
./release-latest.sh --platform pi
```

脚本创建并推送下一个 `pi-aarch64-YYYYMMDD[-N]` tag，触发 `Build Pi aarch64 Release` 并发布正式 GitHub Release。也可以手动运行 workflow；只有测试包不应成为普通更新源时才设置 `prerelease=true`。

Workflow 构建：

- `aarch64-unknown-linux-gnu` Rust workspace 二进制；
- `UI/dist`；
- `RustClaw-pi-aarch64-<tag>.tar.gz`（tag 已以 `pi-aarch64-` 开头时为 `RustClaw-<tag>.tar.gz`）；
- 对应 `.sha256`。

Archive 同时上传为 workflow artifact 和 GitHub Release asset。

发布成功后，workflow 自动只保留最新一个 `pi-aarch64-*` Release，并删除旧 Pi Release 及关联 tag。`ubuntu-x86_64-*` 使用独立前缀，不会被删除。

## 树莓派更新说明

已有安装必须保留：

- `configs/`
- `data/`
- `logs/`
- `.pids/`

`data/` 同时包含主运行时数据库和 `data/skills/` 下的技能私有库；更新时必须
保留整个目录，不能只保留 `rustclaw.db`。

更新包提供二进制、脚本、prompt、migration 和 `UI/dist`，不得用包内默认值覆盖线上 secret 或 channel 设置。

替换文件后重启所需服务。仅后端更新重启 `clawd`；完整 runtime 更新还应重启已选择 channel adapter。

Admin Release 更新路径验证 checksum、保留 runtime 目录，并通过原子替换逐个更新预编译二进制，避免覆盖正在运行的 `clawd` 时出现 `Text file busy`，随后重启 `clawd`。systemd 托管的 Linux 安装会用独立 transient unit 调度重启，避免旧 service 停止时同时杀死自己的重启进程。已有 RustClaw nginx site 时直接复制包内 `UI/dist`，不在树莓派本地编译 UI；没有 nginx 的本地安装不会被配置 nginx。
