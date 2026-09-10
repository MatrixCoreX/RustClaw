# 0.1.4 账号显示名称与 Pi UI 部署

网页和桌面共用 `UI/src/lib/asset-account-options.ts`。资产页和交易页的默认账号标签统一为“硬件设备绑定账号”，英文为“Account bound to hardware device”。只修改显示文案和已有测试预期，账号 ID、绑定来源、认证、权限与交易逻辑保持原样。

网页相关 36 项测试、网页与桌面类型检查和构建、产品身份与双品牌 UI 构建、MCP 合同检查通过。安装后的桌面客户端通过 20 项原生检查，已在深浅主题窗口中核对新标签。0.1.4 已安装并重新打开，安装前后的设备资料摘要一致，包内与已安装二进制逐字节一致。

UI 修改已提交并推送到权威仓库 main：`4b26d7f939c1460d01c8e4074fe0f7999d749209`。Pi 使用已发布的静态文件，没有 UI 源码仓库；已只读同步该提交对应的构建产物，分别更新 nginx 网页目录和运行时 UI/dist。

部署前核对两份既存网页的公共文件无独有修改，随后备份并先发布资源、最后原子替换 index.html，保留旧哈希资源供已打开的页面使用。备份位置：

- nginx 网页：`/home/pi/.cache/agent-ui.sWaBJ9O8/previous-ui`
- 运行时 UI/dist：`/home/pi/.cache/agent-ui.sWaBJ9O8/previous-runtime-ui`

Pi 的 `ui-release.json` 记录实际部署提交，HTTP 与 HTTPS 入口均验证与本机提交一致。66 个 HTTPS 静态文件逐个核对 SHA-256；两个入口均显示新标签，匿名会话和身份鉴权保持正常。HTTP / HTTPS nginx 配置摘要以及 clawd / webd 进程均保持不变。

详细记录位于 `test-results/account-label/`，包含 UI 补丁、构建日志、安装校验、原生截图、部署及双协议验证 JSON。网页完整部署清单由 `ui-release.json` 提供。
