# Windows 与 macOS 桌面版

版本：0.2.1，内部测试。原生平台的实际验收状态见 [cross-platform-validation.md](cross-platform-validation.md)。

## 安装包和使用

| 平台 | 安装形式 | 运行条件 |
| --- | --- | --- |
| Windows x64 | NSIS `-setup.exe`；MSI 中英文包 | Windows 10/11 x64，Microsoft WebView2 |
| Mac Apple 芯片 | `aarch64.dmg`、`.app.zip` | macOS 13+，M 系列芯片 |
| Mac Intel | `x64.dmg`、`.app.zip` | macOS 13+，Intel 芯片 |

Windows 优先使用 `-setup.exe`：为当前用户安装，缺少 WebView2 时由内置 Microsoft 引导程序联网安装。首次离线安装前需事先安装 WebView2。安装器不添加防火墙放行规则，也不关闭系统防火墙。企业分发可使用 MSI。

macOS 打开 DMG，将应用拖入 Applications 后启动。首次查找或连接设备时，允许系统的“本地网络”访问；如果曾拒绝，可在系统设置的隐私与安全性中为该应用打开。仅在使用录音时需要麦克风权限。程序不会安装系统根证书，也不会更改设备上的 HTTP 或 HTTPS 配置。

系统防火墙限制设备广播时，仍可填写设备地址，或在当前可信家庭网络使用“扫描局域网”。扫描结果只提供地址；登录前必须核对 HTTPS CA 指纹或 SSH 主机指纹。无需为公共网络开放入站访问。

当前 Windows 包未做 Authenticode 签名，Mac 包使用 ad-hoc 签名，未做 Developer ID 签名和 Apple 公证。它们用于内部测试；系统可能显示来源提醒。核对下载文件与同批 `SHA256SUMS`，仅对核对过的应用使用系统提供的单次“仍要打开”入口，不关闭 Gatekeeper 或 SmartScreen。面向普通用户公开发行前需配置正式签名与公证，不能把 ad-hoc 签名称作发布者认证。

## 本机开发和构建

所有命令从 `desktop/` 执行，修改局限于本目录。

公共依赖：Rust 1.97.1、Node 22、Git。共享 UI 的依赖也需要安装，但桌面构建只读取共享源文件。

Windows 安装 Visual Studio 2022 Build Tools 的“使用 C++ 的桌面开发”及 Windows SDK，使用 `x86_64-pc-windows-msvc` 工具链；安装 WebView2。可在 PowerShell 运行下面命令。

Mac 安装 Xcode Command Line Tools；在需要制作 DMG 的构建机上安装并选择 Xcode。Apple 芯片使用 `aarch64-apple-darwin`，Intel 使用 `x86_64-apple-darwin`。在对应架构的 Mac 上构建。

```text
node scripts/check-dependencies.mjs
npm ci --prefix ../UI
npm ci
npm run lint
npm test
cargo test --locked --no-default-features
npm run build
```

按当前宿主选择 `tauri.windows.conf.json` / `tauri.macos.conf.json`；产物在 `target/release/bundle/`，显式传 `--target` 时则位于对应 target 子目录。Windows 的构建脚本通过 Node 直接启动 Tauri，不调用 `.cmd` 文件。保留所有 target / 构建缓存。

## 原生 CI

`ci/native-build.yml` 定义 Windows x64、Mac arm64、Mac x64 三个作业，只接受完整提交 SHA。每个作业验证宿主架构，运行原生 TLS / SSH / CSRF / 下载 / 扫描 / 凭据库测试，构建安装包，再安装并检查实际窗口；输出 SHA-256、来源提交、签名状态和截图。

GitHub 仅运行 `.github/workflows/` 中的流程。模板保留在 `desktop/ci/`；桌面专用入口位于 `.github/workflows/desktop-native.yml`。提交并推送后执行：

```text
gh workflow run desktop-native.yml -f source_commit=<完整提交SHA>
```

流程只读仓库、不读取签名 secrets、不发布 GitHub Release、不访问实际设备。`scripts/verify-native-package.py` 只允许在一次性的 GitHub runner 上安装和启动测试包，避免覆盖日常开发机上的现有客户端。原生测试记录不等于实际家庭 LAN 配对验收；云端无法连接用户的 Pi。

参考：[Tauri Windows 安装包](https://v2.tauri.app/distribute/windows-installer/)、[Tauri macOS 签名](https://v2.tauri.app/distribute/sign/macos/)、[Apple 本地网络隐私](https://developer.apple.com/documentation/technotes/tn3179-understanding-local-network-privacy)。
