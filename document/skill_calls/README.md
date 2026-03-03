# 技能单独调用脚本说明

本目录配套 `scripts/skill_calls/` 下的脚本，用于**绕过 Telegram/Agent**，直接通过 `skill-runner` 调用单个技能。

## 目录结构

- 脚本目录：`scripts/skill_calls/`
  - 通用入口：`scripts/skill_calls/_run_skill.sh`
  - 每技能脚本：`scripts/skill_calls/call_<skill>.sh`
- 说明目录：`document/skill_calls/`
  - 总说明：`document/skill_calls/README.md`
  - 每技能说明：`document/skill_calls/<skill>.md`

## 快速开始

```bash
bash scripts/skill_calls/call_crypto.sh
bash scripts/skill_calls/call_health_check.sh --profile release
bash scripts/skill_calls/call_http_basic.sh --args '{"method":"GET","url":"https://api.github.com"}'
```

## 通用参数

所有 `call_<skill>.sh` 都支持：

- `--profile debug|release`：runner profile，默认 `debug`
- `--args '<json>'`：覆盖脚本默认参数
- `--user-id N`：请求中的 `user_id`，默认 `1`
- `--chat-id N`：请求中的 `chat_id`，默认 `1`
- `--auto-build`：缺少 runner/skill 二进制时自动编译
- `--raw`：输出原始一行 JSON（不做 `jq` 美化）
- `--help`：查看帮助

## 统一请求协议

脚本通过 `skill-runner` 发送 JSON（单行）：

```json
{
  "request_id": "skill-call-xxx",
  "user_id": 1,
  "chat_id": 1,
  "skill_name": "crypto",
  "args": {"action":"quote","symbol":"BTCUSDT"},
  "context": null
}
```

返回是技能标准响应：

```json
{
  "request_id": "skill-call-xxx",
  "status": "ok|error",
  "text": "...",
  "extra": {},
  "error_text": null
}
```

## 全部技能清单

- `x`
- `system_basic`
- `http_basic`
- `git_basic`
- `install_module`
- `process_basic`
- `package_manager`
- `archive_basic`
- `db_basic`
- `docker_basic`
- `fs_search`
- `rss_fetch`
- `image_vision`
- `image_generate`
- `image_edit`
- `audio_transcribe`
- `audio_synthesize`
- `health_check`
- `log_analyze`
- `service_control`
- `config_guard`
- `crypto`

## 常见问题

- **报 `skill-runner not found`**
  - 先执行 `./build-all.sh`，或脚本加 `--auto-build`
- **报 `--args is not valid JSON`**
  - 检查 JSON 引号，建议外层单引号、内部双引号
- **报 `unknown skill`**
  - 检查脚本名是否正确，或二进制是否已编译

