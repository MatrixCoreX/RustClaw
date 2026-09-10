# 0.2.0 跨平台验收

日期：2026-09-11。

本次只在 `desktop/` 增加跨平台适配、构建和测试；没有修改服务端、Pi 配置或共享 UI 源码。

## 变更

- Windows 使用 IP Helper 的接口类型及硬件标志选择物理 LAN 网卡；macOS 使用 SystemConfiguration 类型识别有线和 Wi-Fi，排除隧道、网桥等接口。
- AiAPP 使用 Tauri 转换后的平台协议 origin，支持 WebView2 的 `http://device.localhost`；导航与 CSP 仍限制精确 origin、会话、技能路径和授权合同。
- Windows 安装器提供当前用户安装和 WebView2 引导程序；macOS 提供 app / DMG、最低系统版本和具体的局域网 / 麦克风用途说明。没有添加宽泛 ATS 例外。
- 构建阶段与原生程序共用 `ProductIdentity` Rust 验证器，不依赖 Bash；共享 UI 适配器统一 Windows 路径格式。
- 凭据库、HTTPS、SSH、会话、数据库及服务器合同继续使用中性身份。

## 本地验证

记录位于 `desktop/test-results/cross-platform/`。

- 桌面及共享 UI lint、8 项前端测试和 18 项 Rust 安全 / 协议测试通过（1 项 Linux 凭据库不可用测试按显式运行约定未在本轮重复执行）。
- 产品身份自检、inventory 和双品牌 UI 构建通过。
- 跨平台静态检查、MCP 合同、存储归属检查通过。
- 长文件守卫仍报告 12 个既有服务端文件超限，均在桌面目录以外；本次没有改动这些文件。
- 本地 Apple target 检查已执行，因 Linux C 编译器不支持 Apple `-arch` / SDK 而在 `ring` 失败；需要真实 Mac 原生构建，不能据此声称 Mac 验收完成。

- Ubuntu 0.2.0 release 构建和 `.deb` 打包成功，17 项真实 WebKitGTK 窗口回归全部通过，包括可见密码登录、原生 IPC、布局 / 下拉框、流式传输、MP4 seek 和 AiAPP 隔离。启动至设备页约 1.93 秒；本轮仅使用 loopback 协议夹具，不接触 Pi。
- Windows、macOS Tauri 配置通过 schema 校验，生成的 Mac Info.plist 通过解析与隐私字段检查。

## 原生构建状态

Windows x64、Mac Apple 芯片与 Intel 的代码及 CI 流程已准备；原生安装包和对应系统验收尚未完成，等待桌面专用 GitHub workflow 入口的目录例外确认。

现有 Ubuntu 0.1.4 安装与 Pi UI 部署保持不变。不会用尚未完成原生验收的产物覆盖它们。
