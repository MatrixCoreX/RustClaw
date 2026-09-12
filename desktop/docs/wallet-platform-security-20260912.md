# 0.4.1：Windows 与 macOS 私钥保护

本轮只修改 `desktop/`。Windows x64、Mac Apple 芯片、Mac Intel 与 Ubuntu 使用同一份账户、备份和签名合同；各系统保护初始化失败时拒绝钱包请求，不影响设备控制台读取已有设备配置。

## 共同行为

- 每个账户独立随机数据密钥；主密钥只用于解包选定账户。全部账户目录和密文元数据经过认证。
- 系统凭据库仅保存经用户密码加密的主密钥包装。Windows 使用 Credential Manager，Mac 使用 Keychain，Linux 使用 Secret Service；密码和明文私钥不落盘。
- v2 备份使用 Argon2id 256 MiB / 3 次 / 4 路、XChaCha20-Poly1305、独立盐与 nonce；密码强度检测、与钱包密码分离、导出前重新认证、写入后读回验证。旧 v1 备份仍可用原密码恢复。
- 私钥处理进入独立原生子进程，不初始化 WebView。IPC 是有上限和超时的二进制帧，拒绝额外字段、重复字段、重放和私钥导出指令；签名进程重新校验交易条款。
- 长期密钥及解密记录使用锁页缓冲区及两侧不可访问保护页，释放前清零。锁页失败不回退普通堆。
- 签名与导出都要求重新输入钱包密码并在结束后锁定；失焦、关闭、5 分钟期限、休眠时间跳变和系统会话锁定也撤销解锁状态。

## 原生保护差异

| 系统 | 内存与进程保护 | 生命周期与存储 |
| --- | --- | --- |
| Ubuntu | `mlock`、`MADV_DONTDUMP`、禁 core dump、禁止调试附加；seccomp 禁止 IP socket、exec、fork 等 | 父进程退出内核自动杀死；0700/0600、文件描述符检查、logind 锁定 |
| macOS | `mmap/mlock`、禁 core dump、`PT_DENY_ATTACH`；强制 Seatbelt 禁网络和新程序执行；包启用 Hardened Runtime | kqueue 监视父进程死亡，即使正在计算密码派生也退出；0700/0600；WindowServer 会话与失焦检测 |
| Windows | `VirtualAlloc/VirtualLock`；Job 限制仅一个进程并随父进程关闭终止；禁止动态代码、新子进程、Win32k 调用、远程映像和扩展注入；移除 token 特权 | 仅当前用户/SYSTEM 的保护 DACL；新进程句柄禁止读写内存与注入；本地专用管道、显式超时和背压；WTS 会话锁定 |

Windows 使用进程缓解措施和权限收紧，**没有宣称 AppContainer 或内核网络沙箱**。Windows worker 保留当前用户可访问的文件和网络能力；Mac/Linux 也保留凭据服务所需的本地 IPC 和文件访问。三个系统不能称为完全等价的系统级沙箱。

Windows 管道在父进程中建立双方端点后再启动 worker：随机名称、仅一个实例、拒绝远程客户端、用户 ACL，子进程只收到继承的已连接句柄。同步非阻塞读写在帧截止时间内轮询，不使用不受限后台线程或命名 TCP 端口。Job 安装完成并验证后才允许读取钱包凭据。

Mac Seatbelt 的 `sandbox_init` 已弃用，未来系统移除或拒绝时会明确关闭钱包；不能静默跳过保护。正式发行仍需要 Developer ID、公证和 Windows 发布者签名；内部包的 ad-hoc 签名不能证明发布者身份。

## 验证与边界

`scripts/verify-native-package.py` 在安装包实际安装后，使用安装目录里的程序执行原生钱包测试，覆盖 OS 凭据库、双账户、v1/v2 Linux 备份恢复、真实 K1 签名验证、篡改拒绝、IPC 异常和父进程强制终止。测试只使用临时账户，不发送真实交易。加密测试夹具来源在 `tests/fixtures/wallet-backup-provenance.json`。

`test-support/platform-check` 直接编译生产平台模块，便于在 Ubuntu 上先检查 Windows/macOS API 类型；它不能替代真实系统执行测试。最终平台与安装包状态以本轮验证记录和各包 manifest 为准，不沿用 0.2.1 的通过记录。

锁页不涵盖所有临时副本：Argon2 工作区会清零但没有整体锁页，JS/JSON 密码、编译器临时栈/寄存器和系统 IPC 缓冲区也无法完全证明消除。系统管理员、内核、已攻破的当前账户或 GUI 进程仍超出保护边界；Windows 对进程 owner 的隐式改 ACL 权限尤其不能当作恶意同账户隔离。WER 排除普通报告的堆不保证管理员无法主动生成转储。没有进行第三方安全审计。

参考：[Microsoft VirtualLock](https://learn.microsoft.com/en-us/windows/win32/api/memoryapi/nf-memoryapi-virtuallock)、[进程缓解策略](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-setprocessmitigationpolicy)、[命名管道等待模式](https://learn.microsoft.com/en-us/windows/win32/ipc/named-pipe-type-read-and-wait-modes)、[Windows 会话状态](https://learn.microsoft.com/en-us/windows/win32/api/wtsapi32/ns-wtsapi32-wtsinfoex_level1_w)、[Apple 会话查询](https://developer.apple.com/documentation/coregraphics/cgsessioncopycurrentdictionary)。
