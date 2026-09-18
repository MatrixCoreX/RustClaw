# 0.4.1 平台验收：2026-09-12

本轮源码修改均在 `desktop/`。仓库原有根 HEAD/index 保持原位，桌面端之外的 4031 个
源文件与开始时一致；原有缓存和历史安装包保留。安装包统一位于 [0.4.1 目录](../installers/0.4.1/)。

## 桌面安全升级

| 平台 | 构建来源 | 已完成验证 |
| --- | --- | --- |
| Ubuntu x64 | `1043b57a0b665aecccffa7b07b68d9210e1b4b9b` | 本机 DEB 安装/启动、核心测试、钱包 UI 与安装后二进制原生钱包测试 |
| Mac ARM64 / Intel | `1401721f6a6310f3b57a3cc8a716c6b039ceb6d7` | 两种原生 runner 构建，DMG 安装与哈希、签名完整性、窗口启动、各 4 项安装后原生钱包测试 |
| Windows x64 | `f3de34a1ac11bd2c281081a9adb75599a73d16a2` | EXE/MSI 构建、实际安装、主程序/独立签名组件哈希、4 项安装后原生钱包测试、20 项原生 UI 回归 |

Mac 构建：当前发行仓库的 Actions run `34668450733`。
Windows 最终通过构建：同一仓库的 Actions run `34671872080`。
发行仓库地址以 `configs/product_identity.toml` 为准。
失败的中间构建没有作为通过的发行包归档；不同平台的实际源码提交分别保存在包目录记录中。

三个桌面系统均验证了系统凭据库、独立账户密文、跨进程内存读限制、v2 备份写入/读回、
新密钥库恢复、Linux v1/v2 备份恢复、转账/Bancor K1 签名、篡改拒绝、重新认证及签名后锁定。
IPC 测试覆盖额外字段、重放、过大帧和不存在的私钥导出指令；父进程强制退出会结束
处于解锁状态的签名进程。

Windows 采用不链接 User32/GDI32 的独立 `agent-vault.exe`，强制设置并读回系统进程
缓解策略。目录穿越仅保留标准 `SeChangeNotifyPrivilege`，其权限含义见
[Microsoft 说明](https://learn.microsoft.com/en-us/previous-versions/windows/it-pro/windows-10/security/threat-protection/security-policy-settings/bypass-traverse-checking)。
同账户内存访问测试排除管理员 SeDebugPrivilege；不把管理员调试能力纳入普通进程隔离承诺。

## Android

实际 APK：`agent-mobile_0.4.1_universal.apk`，SHA-256：
`70d5c41f28a41d1250d9ea0dd5818bd839b74781180f279731af1bdb0528b495`。

- Android API 26 / 36 的 x86_64 模拟器对同一 APK 各通过 3 项原生测试；从 Android 导出的
  备份在新密钥库中恢复，再生成转账和 Bancor 签名；主机独立验证两台模拟器共 4 个签名。
- Android 16 / WebView 133 完整 UI 验收通过：TLS/拒绝未信任证书、保存登录、原生窗口
  权限限制、真实文件选择器、生成/恢复账户、复制、键盘、150% 字体、后台/锁屏锁定、
  资产/Bancor 完整表单、两个经过验证的合成交易、设备控制台登录与导航。
- 资产、Bancor、账户管理和首页共 24 组尺寸/主题/语言检查；设备控制台另检查 4 种尺寸。
  另验证 1080×2400 / 440 dpi 手机和 1600×2560 / 320 dpi 平板的实际 WebView 布局。
- APK v2 签名、三种 ABI、64 位 ELF 与 ZIP 的 16 KB 对齐、非调试发布配置、无测试桥、
  禁用备份、未导出钱包组件等检查通过。签名私钥不在 APK 内。
- 产品名耦合自测/清单、双品牌 UI 构建、跨平台合同与存储所有权检查通过。

详细测试数据及源文件哈希见 [Android manifest](../installers/0.4.1/android/manifest.json)。
UI 测试只使用临时账户和合成交易；测试 APK 与发行 APK 分离。

## 尚未覆盖的范围

Windows 包未做发布者签名，Mac 包未做 Developer ID 公证；Mac 的完整 WebDriver UI 回归
仍未覆盖。Android 的实体 ARM 手机、厂商差异、折叠屏铰链、相机/麦克风、实际局域网发现、
16 KB 页 OS 运行尚未实测，不能用静态对齐和模拟器结果代替。未进行第三方安全审计；
root/管理员、系统组件或应用主进程已被攻破仍在既定保护边界之外。
