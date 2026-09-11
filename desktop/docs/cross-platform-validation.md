# 0.2.1 跨平台验收

日期：2026-09-11。

本次在 `desktop/` 增加跨平台适配、本机发现、HTTP 回环连接、构建和测试，并添加桌面专用 `.github/workflows/desktop-native.yml` 入口；没有修改服务端源码、Pi 配置或共享 UI 源码。另按用户要求为 162 增加独立 HTTPS 运行入口，见 [部署记录](host-162-https-validation.md)。

## 变更

- Windows 使用 IP Helper 的接口类型及硬件标志选择物理 LAN 网卡；macOS 使用 SystemConfiguration 类型识别有线和 Wi-Fi，排除隧道、网桥等接口。
- AiAPP 使用 Tauri 转换后的平台协议 origin，支持 WebView2 的 `http://device.localhost`；导航与 CSP 仍限制精确 origin、会话、技能路径和授权合同。
- Windows 安装器提供当前用户安装和 WebView2 引导程序；macOS 提供 app / DMG、最低系统版本和具体的局域网 / 麦克风用途说明。没有添加宽泛 ATS 例外。
- 构建阶段与原生程序共用 `ProductIdentity` Rust 验证器，不依赖 Bash；共享 UI 适配器统一 Windows 路径格式。
- 凭据库、HTTPS、SSH、会话、数据库及服务器合同继续使用中性身份。

## 本地验证

本轮记录位于 `desktop/test-results/native-release/` 和 `desktop/test-results/native/`；上一轮 0.2.0 记录保留在 `desktop/test-results/cross-platform/`。

- 桌面及共享 UI lint、8 项前端测试和 22 项 Rust 安全 / 协议测试通过（1 项 Linux 凭据库不可用测试按显式运行约定未在本轮重复执行）。
- 产品身份自检、inventory 和双品牌 UI 构建通过。
- 跨平台静态检查、MCP 合同、存储归属检查通过。
- 长文件守卫仍报告 12 个既有服务端文件超限，均在桌面目录以外；本次没有改动这些文件。
- 本地 Apple target 交叉检查因 Linux C 编译器不支持 Apple `-arch` / SDK 而在 `ring` 失败；两种 Mac 架构的编译与测试已改由真实 Mac runner 完成。

- Ubuntu 0.2.1 release 构建和 `.deb` 打包成功，19 项真实 WebKitGTK 窗口回归全部通过，包括可见密码登录、原生 IPC、布局 / 下拉框、流式传输、MP4 seek 和 AiAPP 隔离。新增本机安装提示、运行中服务发现、可见本机 HTTP 表单与密码登录检查；本轮使用 loopback 协议夹具，不接触 Pi。
- Windows、macOS Tauri 配置通过 schema 校验，生成的 Mac Info.plist 通过解析与隐私字段检查。

## 原生构建状态

安装包、来源提交与校验文件统一存放于 `desktop/installers/0.2.1/`；各平台 manifest 只记录实际完成的检查。

- Linux x64：来源 `5851af04d4005de586dcd2b493d122042322e4b3`，`.deb` 已归档并校验。
- Mac Apple 芯片：来源 `daaee1ff480b16ff25427fcb5439b6250d0ea9ce`，macOS 14.8.9 原生测试、Keychain 隔离读写、DMG 挂载安装、包内签名和实际窗口检查通过；DMG / app ZIP 已下载并验证 SHA-256。
- Mac Intel：同一来源 `daaee1ff480b16ff25427fcb5439b6250d0ea9ce`，macOS 15.7.9 原生测试、Keychain、签名 / DMG / 安装 / 窗口检查通过；DMG / app ZIP 已归档并校验。
- Windows x64：来源 `c994a28a59c5fad64fd9e8765d944cd9a6c956b6`，Windows Server 2022 原生协议 / 安全 / Credential Manager 测试、NSIS 安装、安装文件完整 SHA 校验和 19 项真实 WebView2 界面回归全部通过。EXE 与中英文 MSI 已归档并校验。

Mac 使用 ad-hoc 签名且未经 Apple 公证；完整业务界面回归、系统隐私授权和真实家庭局域网配对仍需用户机器验证，不能将窗口启动检查当作完整验收。

现有 Ubuntu 0.1.4 安装与 Pi UI 部署保持不变，本次没有覆盖本机已安装客户端。三平台共 8 个安装文件已通过根目录 `SHA256SUMS` 校验；Mac 原生流程见 [构建记录](https://github.com/MatrixCoreX/RustClaw/actions/runs/34544675891)，Windows 见 [构建记录](https://github.com/MatrixCoreX/RustClaw/actions/runs/34547369270)。


## Windows 内嵌页面权限测试

原测试要求 iframe 的原生调用必须收到拒绝回调。Wry 0.55 的 WebView2 实现注册顶层消息处理器，iframe 消息需要单独的 frame 处理器，因此 Windows 上可能没有回调；参见 [Microsoft 的 frame 消息说明](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/frames)。

测试仅针对 Windows 的 WebView2 子框架允许记录“无回调”状态，并设置 1.5 秒观察时限；任何成功返回都判为失败。它同时尝试读取和新增设备，随后再次完成合法的 AiAPP bridge 请求、重新核对回调状态，并从真实主窗口确认设备列表与测试前完全一致。AiAPP 顶层调用主窗口命令与未声明能力仍必须明确拒绝。Ubuntu 和 Windows 修改后的 19 项原生回归均通过。Windows 实测 iframe 读写探测均无回调，合法 bridge 两次成功，主窗口设备列表保持不变。
