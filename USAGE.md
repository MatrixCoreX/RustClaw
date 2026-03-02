# RustClaw 使用说明 / Usage Guide

本文件帮助开发者快速上手 `RustClaw`，并提供适合 Git 协作的标准流程。  
This document helps developers quickly start using `RustClaw` and follow a Git-friendly collaboration workflow.

## 1. 环境准备 / Environment

- 操作系统 / OS: Linux or macOS (recommended)
- 依赖工具 / Required tools:
  - `git`
  - `rustup`, `cargo`
  - `bash`
- 可选工具 / Optional:
  - `sqlite3` (for local DB inspection and troubleshooting)

## 2. 克隆与初始化 / Clone and Initialize

```bash
git clone <your-repo-url>
cd RustClaw
./setup-config.sh
```

如使用自定义配置，请先检查 `configs/` 下配置再启动。  
If you use custom settings, review files under `configs/` before starting services.

## 3. 本地运行 / Local Run

### 一键启动与停止 / Start and Stop

```bash
./start-all.sh
./stop-rustclaw.sh
```

### 按服务单独启动（按需） / Start Individual Services (Optional)

```bash
./start-clawd.sh
./start-telegramd.sh
./start-whatsappd.sh
./start-whatsapp-webd.sh
```

### 回归与模拟脚本（按需） / Regression and Simulation (Optional)

```bash
./simulate-telegramd.sh
```

## 4. 常用开发命令 / Common Development Commands

```bash
# Build
cargo build

# Test
cargo test

# Lint (if enabled in your workflow)
cargo clippy --all-targets --all-features
```

## 5. 配置与敏感信息 / Config and Secrets

- 不要提交真实密钥、Token、密码。  
  Do not commit real secrets, tokens, or passwords.
- 使用 `.env.example` 作为模板，复制为本地私有文件再填写。  
  Use `.env.example` as a template and keep real values in local private files.
- 提交前检查配置差异，避免泄露本地环境信息。  
  Review config diffs before commit to avoid leaking local environment data.

## 6. Git 协作流程（推荐） / Recommended Git Workflow

### 6.1 新建分支 / Create a Branch

```bash
git checkout -b feat/<short-name>
# or
git checkout -b fix/<short-name>
```

### 6.2 开发与自检 / Develop and Verify

1. 完成功能或修复。/ Implement the feature or fix.
2. 运行 `cargo test`，必要时补充脚本验证。/ Run `cargo test` and add script-level checks if needed.
3. 确认没有误跟踪敏感文件。/ Ensure no sensitive files are tracked:

```bash
git status
```

### 6.3 提交 / Commit

```bash
git add .
git commit -m "feat: add xxx support"
```

提交类型建议 / Recommended commit prefixes:

- `feat:` 新功能 / New feature
- `fix:` 缺陷修复 / Bug fix
- `refactor:` 重构 / Refactor
- `docs:` 文档更新 / Documentation
- `chore:` 构建、脚本、杂项 / Build, scripts, maintenance

### 6.4 推送与合并 / Push and Merge

```bash
git push -u origin <branch-name>
```

PR 建议包含 / PR should include:

- 变更目的 / Why this change
- 主要改动点 / Key changes
- 测试方式与结果 / Test plan and results
- 回滚方案（如有） / Rollback plan (if applicable)

## 7. 发布（按需） / Release (Optional)

仓库已包含发布脚本：  
The repository includes a release script:

```bash
./package-release.sh
```

发布前建议 / Before release:

- 确认分支与 Tag 策略 / Confirm branch and tag strategy
- 确认配置已脱敏 / Ensure configs are sanitized
- 记录版本变更说明 / Prepare release notes

## 8. 故障排查 / Troubleshooting

- 优先查看启动脚本输出与服务日志。  
  Check startup output and service logs first.
- 使用 `check-logs.sh` 汇总关键日志。  
  Use `check-logs.sh` to inspect critical logs quickly.
- 网络问题先查配置与端口，再查外部依赖可达性。  
  For network issues, verify config and ports before external dependency reachability.

---

建议扩展顺序：环境准备 -> 启动方式 -> Git 流程 -> 排障。  
Recommended extension order: Environment -> Run Flow -> Git Workflow -> Troubleshooting.
