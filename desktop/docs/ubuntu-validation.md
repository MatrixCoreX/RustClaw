# Ubuntu 桌面测试版验收记录

当前版本为 **0.1.2**，[登录修复](login-fix-validation.md)记录了 CSRF 合同错误与修正；Pi HTTPS 与局域网发现的实测记录见 [LAN 验证](lan-discovery-validation.md)。下文保留 0.1.0 首次交付的历史记录，设备配置与测试范围以新记录为准。

日期：2026-09-11。起始源码基线：`dd4382c66`；发行构建时仓库 HEAD 为 `56a885602`。期间新增提交只涉及 health-check，桌面复用的 UI、产品身份模块与配置没有变化。桌面工程版本：`0.1.0`。

## 交付范围

本轮按用户要求，全部桌面开发位于 `desktop/`，优先完成 Ubuntu 可安装测试版。原计划中的三平台签名发行、真实 Pi 业务验收和自动更新发布不作为已完成事项。

- 独立 Cargo workspace、Cargo.lock、npm 锁文件与 Tauri 2 工程。
- 16 个原有 React 页面共享源代码，通过构建期适配接入原生 transport；没有另写一套模型、技能、NNI 或渠道业务。
- HTTPS 证书链/主机名验证、应用内 CA 指纹配对；SSH 主机指纹验证，密码与私钥登录，远端 loopback direct-tcpip。
- 设备列表、Key / 用户名密码登录、系统凭据库可选保存、忘记设备、会话切换与缓存隔离。
- 二进制 IPC 分块上传、SSE、原生下载保存/进度/取消、受限回环音视频通道、独立受限 AiAPP 窗口。
- DNS-SD 候选发现、双主题、设备和角色提示、确认框显示目标设备。
- 手工 `.deb` 安装/升级入口、使用文档、安全合同和可重跑测试。

## 环境与平台矩阵

| 环境 | 结果 |
| --- | --- |
| Ubuntu 26.04.1，x86_64，GTK 3.24.52，WebKitGTK 2.52.6，GLIBC 2.43 | 本轮实际构建和运行环境 |
| X11 + Xvfb，软件渲染 | 实际 WebKitGTK / Tauri WebDriver 测试 |
| Wayland、中文输入法、麦克风/摄像头、系统休眠 | 尚未完成真实桌面人工验收 |
| Ubuntu 22.04 / 24.04、其他 Linux、Linux arm64 | 未发布为已验证平台；本包需要 GLIBC 2.39+ |
| macOS Intel / Apple Silicon、Windows | 未发布；本轮只做 Linux |

Rust 1.97.1，Node 22.22.1；原生测试夹具只监听本机 loopback，使用临时证书和测试凭据。没有连接或修改真实 Pi、真实账户或交易资产。

## 已通过的验证

| 验证 | 证据与实际范围 |
| --- | --- |
| 原网页单元测试 | 515 / 515，通过；包含既有业务与 NNI 前端测试 |
| 原网页 TypeScript | 通过；双品牌脚本还分别构建原 UI |
| 桌面 TypeScript 与构建 | 通过；桌面依赖引用和构建期替换合同检查 |
| 桌面前端测试 | 6 / 6；路径拒绝、流式响应、multipart 文件往返、取消、不重试写操作、设备/主体隔离、双品牌包名与 launcher 不变 |
| 原生核心测试 | 7 / 7；HTTPS / SSH 真实协议夹具、证书/主机名/指纹错误拒绝、Cookie/CSRF、Key、私钥认证、SSE、4 MiB 上传下载、原子完成、取消保护原文件 |
| GUI policy 测试 | 4 / 4；含打包首页 URL 与远程导航拒绝回归 |
| 凭据库失效测试 | 1 / 1；隔离不可用 D-Bus，记住登录失败后仍可用内存会话，无明文 fallback |
| 原生桌面端到端 | 12 组检查；完整结果在 `test-results/native/report.json` |
| 16 个页面的打开检查 | 12 个可见导航 + 4 个既有上下文/恢复路由；在实际原生窗口内渲染，页面并未借用外部浏览器 |
| 大视频 | 约 5.7 MiB H.264 MP4，在实际 WebKitGTK 中解码、播放、seek；Range 精确字节读取、错误 Host/Origin、令牌撤销通过 |
| AiAPP | 实际独立 WebView 和 opaque iframe 加载，声明能力的 bridge 往返成功；主窗口 IPC、媒体授权与未声明 capability 均拒绝 |

页面打开检查使用协议夹具，不代表每个远端业务操作都已验证。夹具没有模拟所有业务数据，因此部分页面会显示可读的服务不可用提示；真实业务验收必须使用指定测试设备。

原生截图：`test-results/native/01-device-home.png`、`02-shared-console.png`、`03-isolated-aipp.png`，以及 `page-*.png`。测试产物被 `.gitignore` 排除，保留在本机供查看。

## 门禁与已有问题

- 产品身份 coupling self-test / inventory：通过，新增耦合为 0。
- 双品牌 UI 测试：通过。桌面另验证包名、可执行文件、菜单文件、凭据和数据 namespace 保持中性。
- AiAPP 解耦 self-test / inventory：通过。
- 跨平台静态、MCP 合同、技能存储 ownership：通过。
- `check_long_files.py`：仓库已有 12 个超长/增长文件导致失败，均位于 `crates/`，本轮未修改这些文件。详情见 `test-results/logs/long-files.log`。
- 历史硬编码语言门禁：已有 runtime 4 项失败，位于 `capability_result_synthesis.rs:530–538`，本轮未修改。详情见 `test-results/logs/language.log`。
- 已尝试 Apple target 的核心 `cargo check`；当前 Linux C 工具链没有 macOS SDK，不识别 `-arch` / Apple 部署参数，`ring` 构建失败。不记为 macOS 通过。

以上已有问题按用户要求没有跨目录修补。工作区中原有三个 WhatsApp 配置变更保持原样。

## 尚需实际环境验证的边界

1. 真实 Pi / LAN 的全部管理动作、长期任务、断线/休眠恢复、证书轮换、实际账号撤销及动态技能安装卸载。
2. 系统“保存文件”交互、完整凭据库解锁/保存/恢复、录音输入、系统通知及辅助功能。下载流与凭据库失败语义已分别自动验证。
3. AiAPP 现有目录 API 不公开原始 receipt/generation；资源和调用继续依赖服务端强制校验，客户端不伪造这些元数据。
4. 客户端使用手工安装包更新，没有启用签名自动更新 feed。也没有修改设备 HTTPS、HTTP、nginx、WEBD 或服务配置。

本版本供 Ubuntu 本机和指定设备试用，不将上述未验证项标为通过。

## 重跑

从仓库根目录执行：

```bash
npm test --prefix desktop
npm run lint --prefix desktop
cargo test --manifest-path desktop/Cargo.toml --no-default-features
cargo test --manifest-path desktop/Cargo.toml --test policy
DBUS_SESSION_BUS_ADDRESS=unix:path=/tmp/agent-desktop-missing-secret-service \
  cargo test --manifest-path desktop/Cargo.toml --no-default-features unavailable_system_vault -- --ignored
npm run dev --prefix desktop
python3 desktop/tests/native_e2e.py
npm run build --prefix desktop
```

驱动安装和系统依赖见主 README。全部 fixture、输出和独立 workspace 都在桌面目录；不需要删除任何已有编译缓存。

## 最终安装包与安装验证

- 安装包：`releases/agent-desktop_0.1.0_amd64.deb`，8,852,986 字节。
- SHA-256：`caa6a45d869630250b3e5fc7a46886afb672ce1473ec4f297c5ea1c267d8de39`。校验文件：`releases/SHA256SUMS`。
- 已检查包内文件、运行库、桌面菜单格式；只有程序、图标、菜单入口，没有测试驱动、fixture、私钥、源代码目录或远端服务。
- `dpkg` 首次安装、同版本覆盖安装、卸载、重装均返回 0。跨版本 schema 迁移尚未验证。
- 最终状态为 `agent-desktop 0.1.0 install ok installed`；可从应用菜单或运行 `agent-desktop` 打开。
- 最终 12 组原生 E2E 使用的是已安装的 `/usr/bin/agent-desktop` 发行二进制，而非开发服务器；结果全部通过。
- 安装与回归原始日志在 `test-results/logs/install.log`、`reinstall.log`、`uninstall.log`、`final-install.log` 和 `native-installed.log`。

## 本机测量基线

本次为 Xvfb 软件渲染、loopback fixture 和当前机器负载下的单次采样，不作为真实 LAN 吞吐或常驻内存承诺。RSS 合计包含主程序及其 WebKit 子进程，排除驱动；这是几个时点的采样，不是精确峰值。

| 项目 | 结果 |
| --- | --- |
| 发行程序启动到设备首页 | 1.370 秒 |
| SSE 首次读取 | 0.0408 秒 |
| Multipart 上传 | 190,218 字节 / 0.054 秒 |
| 首页 RSS 合计 | 381.3 MiB |
| 共享控制台 RSS 合计 | 452.8 MiB |
| 媒体播放时 RSS 合计 | 577.3 MiB |
| 视频夹具大小 | 5,895,635 字节 |

## 技术来源

原生权限和测试参考 [Tauri capabilities](https://v2.tauri.app/security/capabilities/) 与 [WebDriver](https://v2.tauri.app/develop/tests/webdriver/) 文档。媒体方案由本机失败/成功对照实测决定，并参考 [WebKit GStreamer 播放实现](https://github.com/WebKit/WebKit/blob/main/Source/WebCore/platform/graphics/gstreamer/MediaPlayerPrivateGStreamer.cpp)；没有将失败的自定义 scheme 播放标为通过。
