# RustClaw Systemd Service 使用说明

## 服务配置

已创建 systemd service 文件：`/etc/systemd/system/rustclaw.service`

该服务配置了：
- ✅ 自启动（已启用）
- ✅ 使用 singbox 代理（端口 2080）
- ✅ 依赖 sing-box.service（确保代理先启动）
- ✅ 自动重启（失败时）
- ✅ 免配置直接启动（使用 `start-all-bin.sh`，默认 release profile）

## 常用命令

### 查看服务状态
```bash
sudo systemctl status rustclaw
```

### 启动服务
```bash
sudo systemctl start rustclaw
```

### 停止服务
```bash
sudo systemctl stop rustclaw
```

### 重启服务
```bash
sudo systemctl restart rustclaw
```

### 查看日志
```bash
# 查看实时日志
sudo journalctl -u rustclaw -f

# 查看最近日志
sudo journalctl -u rustclaw -n 50

# 查看今天的日志
sudo journalctl -u rustclaw --since today
```

### 禁用自启动
```bash
sudo systemctl disable rustclaw
```

### 启用自启动
```bash
sudo systemctl enable rustclaw
```

## 代理配置

服务已配置使用 singbox 代理，环境变量：
- `HTTP_PROXY=http://127.0.0.1:2080`
- `HTTPS_PROXY=http://127.0.0.1:2080`
- `ALL_PROXY=socks5://127.0.0.1:2080`

如果您的 singbox 端口不是 2080，请修改 `/etc/systemd/system/rustclaw.service` 文件中的端口号，然后执行：
```bash
sudo systemctl daemon-reload
sudo systemctl restart rustclaw
```

## 注意事项

1. 服务会在系统启动时自动启动（已启用）
2. 服务依赖 sing-box.service，确保 singbox 已正确配置并运行
3. 服务使用 `guagua` 用户运行，确保该用户有执行脚本的权限
4. 日志可以通过 `journalctl` 查看，也可以查看 `/home/guagua/git_upload/logs/` 目录下的日志文件
