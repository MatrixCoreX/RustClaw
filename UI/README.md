# Agent Runtime UI

本目录是 Agent Runtime 的浏览器控制台前端项目。

它不是独立产品，而是通过 `webd` 访问核心服务的 Web 管理界面，用于查看运行状态、提交任务、配置部分运行参数、查看日志和做日常管理。

## 技术栈

- Vite
- React
- TypeScript

## 目录说明

- `src/App.tsx`：主界面与主要页面逻辑
- `src/main.tsx`：前端入口
- `src/lib/`：前端辅助逻辑
- `dist/`：构建产物

## 本地开发

前置条件：

- Node.js
- npm

安装依赖：

```bash
cd UI
npm install
```

启动开发服务器：

```bash
cd UI
npm run dev
```

当前开发服务器默认监听：

- `http://127.0.0.1:3000`
- `http://<your-ip>:3000`

`package.json` 中的开发命令使用 `--host 0.0.0.0 --port 3000`，便于局域网设备访问。

## 构建

```bash
cd UI
npm run build
```

构建输出目录为 `UI/dist`。

类型检查：

```bash
cd UI
npm run lint
```

这里的 `lint` 实际执行的是 `tsc --noEmit`。

## 部署方式

Agent Runtime 当前常见的 UI 使用方式有两种：

1. 使用完整 Release 包中的 UI，由 `webd` 提供页面与 API 入口
2. 构建后复制到 nginx 静态目录，API 反向代理给 `webd`

仓库内相关入口：

```bash
# 构建整个仓库时顺带构建 UI
./build-all.sh

# 单独构建并复制到 nginx 目录
./build-ui-nginx.sh

# 安装 agentctl 启动器时一并部署 UI 到 nginx
bash install-agent-cmd.sh
```

如果你只想本地安装而不部署 nginx，建议显式加上：

```bash
bash install-agent-cmd.sh --user --no-deploy-ui
```

## 与后端的关系

- 浏览器入口是 `webd`（默认 `http://127.0.0.1:8788`），也可以使用 nginx 对外地址
- `clawd` 只提供本机内部 API，不直接提供网页或公网入口
- UI 通过 `webd` 使用健康检查、任务、鉴权、配置和日志接口
- 登录、会话与内部 API 请求只访问填写的最终地址，不跟随重定向；外链和媒体请求不受这项限制
- Cookie 会话继续使用 CSRF 校验；直接 Key 仅保留在当前内存中。退出或换账号后，旧身份响应不能更新当前身份
- 本机 HTTP 仍可使用；这不提供网络加密，不可信网络应使用 HTTPS

如果后端没有启动，UI 中依赖 API 的页面将无法正常工作。

## 认证回归测试

```bash
cd UI
npm ci --ignore-scripts
npm test
npm run lint
node node_modules/playwright/cli.js install chromium
node test/runtime-api-transport.browser.mjs
node test/authentication-ownership.browser.mjs
```

浏览器测试使用独立的临时页面服务、回环 HTTP 服务和测试凭据，不访问真实
后端、不调用模型。它们覆盖正常密码/Key 登录、同源与跨源五种重定向，以及
响应头已经返回、正文仍在等待时退出并更换账号。也可通过
`BROWSER_EXECUTABLE` 指定测试浏览器；该配置不改变业务技能使用的浏览器。
