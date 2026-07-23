# Ubuntu x86_64 Release 包

RustClaw 可以通过 GitHub Actions 发布预编译 Ubuntu x86_64 runtime 包，适用于普通 64 位 Ubuntu 云服务器和 PC。

## 构建与发布

推荐命令：

```bash
./release-latest.sh --platform ubuntu
```

脚本创建并推送下一个 `ubuntu-x86_64-YYYYMMDD[-N]` tag，触发 `Build Ubuntu x86_64 Release` 并发布正式 GitHub Release。也可以手动运行 workflow；只有测试包不应成为普通更新源时才设置 `prerelease=true`。

Workflow 构建：

- `x86_64-unknown-linux-gnu` Rust workspace 二进制；
- `UI/dist`；
- `RustClaw-ubuntu-x86_64-<tag>.tar.gz`（tag 已以 `ubuntu-x86_64-` 开头时为 `RustClaw-<tag>.tar.gz`）；
- 对应 `.sha256`。

Archive 同时上传为 workflow artifact 和 GitHub Release asset。

发布成功后，workflow 自动只保留最新一个 `ubuntu-x86_64-*` Release，并删除旧 Ubuntu Release 及关联 tag。`pi-aarch64-*` 使用独立前缀，不会被删除。

## 更新说明

已有安装必须保留：

- `configs/`
- `data/`
- `logs/`
- `.pids/`

更新包提供二进制、脚本、prompt、migration 和 `UI/dist`，不得用包内默认值覆盖线上 secret 或 channel 设置。

Admin Release 更新路径验证 checksum、保留 runtime 目录，并通过原子替换逐个更新预编译二进制，避免覆盖正在运行的 `clawd` 时出现 `Text file busy`，随后重启 `clawd`。主机已有 RustClaw nginx site 时，只复制包内 `UI/dist`，不重新构建；没有 nginx site 的本地安装不会被配置 nginx。
