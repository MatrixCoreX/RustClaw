# RustClaw UI

本目录是 RustClaw 的浏览器控制台前端项目。

它不是独立产品，而是给 `clawd` 提供本地或静态托管的 Web 管理界面，用于查看运行状态、提交任务、配置部分运行参数、查看日志和做日常管理。

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

RustClaw 当前常见的 UI 使用方式有两种：

1. 随仓库一起本地构建，供 `clawd` 或本地静态目录使用
2. 构建后复制到 nginx 静态目录

仓库内相关入口：

```bash
# 构建整个仓库时顺带构建 UI
./build-all.sh

# 单独构建并复制到 nginx 目录
./build-ui-nginx.sh

# 安装 rustclaw 启动器时一并部署 UI 到 nginx
bash install-rustclaw-cmd.sh
```

如果你只想本地安装而不部署 nginx，建议显式加上：

```bash
bash install-rustclaw-cmd.sh --user --no-deploy-ui
```

## 与后端的关系

- 后端核心服务是 `clawd`
- 常见 API 地址是 `http://127.0.0.1:8787`
- UI 主要依赖 `clawd` 暴露的健康检查、任务、鉴权、配置和日志接口

如果后端没有启动，UI 中依赖 API 的页面将无法正常工作。
