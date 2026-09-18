# 桌面密钥保护 0.4.0 验收

方案 1（账号独立加密）、2（内存和密钥进程保护）、5（强密码新版备份）已在本机 Ubuntu 完成开发、测试、安装和启动。本轮源代码修改仅在 `desktop/`。实现与各系统后续要求见 [密钥保护说明](wallet-protection-20260911.md)。

| 检查 | 结果 |
| --- | --- |
| Rust 最终完整测试 | 49 项通过，0 失败 |
| TypeScript 类型检查 / 前端测试 | 通过 / 11 项通过 |
| 安装包独立密钥进程 | 7 组通过：受限系统调用、同用户进程内存读取被拒绝、强制锁页、签名验证、异常帧、父进程死亡等 |
| 原生资产与 Bancor | 16 组通过：资产、转账、买卖、窗口权限、失焦锁定、节点切换、交易结果核实、多语言和复制 |
| 原生保存登录 | 10 组通过 |
| 真实旧版程序升级 | 3 组通过：由 0.3.11 生成账号与旧备份，再由 0.4.0 原生升级；账号、公钥和备份状态保留 |
| 产品身份 self-test / inventory / 双品牌 UI | 通过 |
| 跨平台静态门禁 | 通过 |
| Apple target 编译 | 当前 Ubuntu 无 Apple 工具链/SDK，ring 的 Apple C 编译参数不被宿主编译器支持；不算通过 |

测试环境为 Ubuntu 26.04.1 x86_64、Linux 7.0.0-31-generic。新版备份的密码验证、加密和读回验证，在隔离测试中耗时约 1.655 秒；不同电脑速度会变化。Windows/macOS 的新版密钥进程 adapter 尚未完成，当前结构化拒绝资产密钥操作，本轮不发布其新安装包；ARM 未做原生验收。

已安装并打开 0.4.0，独立密钥进程运行、seccomp 生效，启动时密钥保持锁定。安装前后真实账号文件、连接资料和节点配置的 SHA-256 一致。安装不会触发迁移；首次正确解锁时才升级。

测试过程中发现并修复了标准输入/输出缓冲导致管道超时、serde 无字段操作未拒绝附加字段、状态轮询可能吞掉锁定事件的问题。中途一次测试子进程因测试二进制被并发重新编译替换而启动失败；最终固定二进制完整重跑的 49 项全部通过。

本轮所有原生账号/交易测试使用隔离数据目录、隔离运行目录、隔离 Secret Service 和测试账户，未使用真实账户密码或提交真实资金交易。仓库中 `optional_skills/media_download/tests/test_protocol.py` 出现外部并发变更，本轮没有写入或回滚它。

这些检查不是独立安全审计。进程保留当前用户文件权限和本地 IPC；Argon2 工作区清零但不锁页，语言运行时、栈和寄存器临时副本不具备完整清除保证。已有旧备份不会因新备份生成而失效。更完整的边界见实现说明。

- [安装包](../installers/0.4.0/linux/agent-desktop_0.4.0_amd64.deb)
- [验收数据](../test-results/wallet-protection-0.4.0/validation.json)
- [安装与真实文件校验](../test-results/wallet-protection-0.4.0/installation.json)
- [Rust 最终日志](../test-results/wallet-protection-0.4.0/rust-final-acceptance.log)
- [源文件校验](../test-results/wallet-protection-0.4.0/final-verification.json)
