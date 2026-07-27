# Web 入口与核心隔离

<!-- ai-learning-stage: safety-operations -->
<!-- ai-learning-audience: operator -->

<!-- ai-learning-navigation:start -->
上一页：[交互式编码与输出呈现](09-interactive-coding.zh-CN.md) |
[架构索引](README.md)

<!-- ai-learning-navigation:end -->

RustClaw 把面向浏览器的安全边界集中在 `webd`，把 `clawd` 保留为内部核心
API。这样不会出现绕过浏览器会话网关的第二套公开 UI/API 入口。

## 当前访问拓扑

```mermaid
flowchart LR
    B[浏览器]
    N[nginx<br/>可选 TLS + 静态 UI]
    W[webd :8788<br/>UI + 登录 + 会话 + 代理]
    C[clawd 127.0.0.1:8787<br/>仅内部 /v1 API]
    U[UI/dist]
    D[本机通信守护进程 / clawcli]

    B -->|本地部署| W
    B -->|域名或 TLS| N
    N -->|静态文件| U
    N -->|/v1 与 /webd| W
    W -->|无 nginx 时的静态文件| U
    W -->|已鉴权 /v1| C
    D -->|本机已鉴权 API| C
```

`clawd` 不再托管 `UI/dist`，也不能绑定非 loopback 地址。它的地址不是用户配置项。
内部测试 override 也只接受 loopback socket，因此并行本地测试不会重新打开对外端口。

## 责任归属

- `webd` 负责浏览器登录、持久会话、凭据注入、登录失败锁定、请求体限制、
  代理来源处理和 API 转发。
- `clawd` 负责已鉴权的任务/管理 API、agent 执行、工具、技能和持久化。
- nginx 是可选外层，负责 TLS/域名终止和静态资源，再把 `/v1` 与 `/webd` 转给
  `webd`，不直接连 `clawd`。
- 本机通信守护进程和 `clawcli` 可以直连 loopback 核心 API；这是机器内部组件通信，
  不是浏览器入口。

## 首页操作

首页会显示 nginx 是否已安装、正在运行、已配置 RustClaw 站点，以及 UI 是否已部署。
仅管理员可执行下列后台操作：

- **启用/修复 nginx**：安装或启动 nginx，写入 RustClaw 站点，部署已有 `UI/dist`，
  验证配置后启动或重载。
- **构建并部署最新 UI**：源码安装会先构建当前 UI；Release 安装使用包内预构建产物，
  然后部署到 nginx。
- **打开 nginx 入口**：只有进程、站点和 UI 检查全部通过后才会出现。

操作返回机器状态和错误键。面向用户的中英文说明由 UI 渲染，运行时不解析固定自然语言。

## 部署规则

- Linux/macOS 原生部署：`clawd` 只绑定 loopback，`webd` 是浏览器入口。
- Docker：发布 `8788`，不发布 `8787`；`webd` 和 `clawd` 在容器内通过 loopback 通信。
- 本地使用不需要 nginx，直接打开 `webd`。
- 服务器/域名部署应在 `webd` 外层使用 nginx 和可信 TLS。
- 不要对外暴露或端口转发 `8787`。

## 验证

```bash
cargo test -p webd
cargo test -p clawd internal_listener_tests::
cargo test -p clawd workspace_nginx_tests::
python3 scripts/check_long_files.py
```

`webd` 测试证明 UI 资源由它自己提供，而 `/v1` 仍然是代理路径。`clawd` 监听测试会拒绝
wildcard 和局域网地址。
