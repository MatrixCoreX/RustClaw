# 跨平台运行时合同

RustClaw 共享生产代码以 Linux 和 macOS 为目标。平台专属能力必须显式处理：使用平台 adapter、返回结构化 unsupported 结果，或通过 `cfg` 排除。依赖缺失绝不允许降级到限制更少的后备实现。

## 进程沙箱

`tools.sandbox_backend = "auto"` 在 Linux 解析为 Bubblewrap，在 macOS 解析为 Seatbelt（`/usr/bin/sandbox-exec`）。本地后端可执行文件缺失时，两者都必须 fail closed。`danger_full` 是唯一直接进程模式，必须显式选择。`remote_container` 目前只是合同占位；远端 executor 未配置时返回 `sandbox_remote_backend_not_configured`，不得把它当作隐式后备。

沙箱诊断提供请求和解析后的 backend、可用性、fail-closed 状态、reason code、平台，以及文件系统、网络、进程、凭据、资源和环境控制字段。这些是机器合同，不是本地化用户回复。

## 平台服务与工具

- Linux 服务发现和生命周期操作只能通过 Linux 平台 adapter 调用 systemd 或 SysV。
- macOS 服务发现使用 Homebrew services、launchd 或进程观测。在 macOS 请求 Linux manager 时返回 `unsupported_platform`，不得启动 Linux 命令。
- 包、系统健康、进程和文件系统技能应提供原生 macOS 实现；不支持的测量必须返回结构化 unavailable 数据。
- 长时间命令任务使用 GNU `timeout`、Homebrew `gtimeout` 或 Python 进程组 watchdog。如果都不可用，执行必须 fail closed，不得在没有期限的情况下运行。

## 开发脚本

脚本通过 `scripts/shell_compat.sh` 处理超时、文件元数据、host/target 检测和低内存构建设置。Release 与 NL 脚本不得依赖 GNU 专属 `stat -c`、`date -d`、`find -printf`，以及 Bash 4 专属数组或大小写转换，确保 macOS 默认 shell 可用。

修改平台敏感代码后运行永久门禁：

```bash
python3 scripts/check_cross_platform_contracts.py --self-test
python3 scripts/check_cross_platform_contracts.py
```

在 macOS 主机上运行原生 workspace 测试。在其他主机上，只有同时安装 Apple target 和 Darwin C toolchain/SDK 时才尝试 Apple target check。仅有 Rust target 不足以编译包含原生 C 或汇编依赖的 crate（例如 `ring`）；Release 证据必须区分源码失败与 cross compiler/SDK 不可用。
