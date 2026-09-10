# 网页与桌面复用清单

由 `python3 desktop/scripts/inventory.py` 读取当前 UI 源码生成。

页面直接复用原组件，桌面请求统一经过原生 HTTPS / SSH 连接；业务服务端验收与界面可加载分开记录。

| 页面 | 桌面来源 | 原生传输 | 真实业务端到端验收 |
| --- | --- | --- | --- |
| `dashboard` | 原有 UI 页面（无副本） | 统一请求适配器 | 需在用户测试设备逐项验证 |
| `chat` | 原有 UI 页面（无副本） | 统一请求适配器 | 需在用户测试设备逐项验证 |
| `ai_learning` | 原有 UI 页面（无副本） | 统一请求适配器 | 需在用户测试设备逐项验证 |
| `nni` | 原有 UI 页面（无副本） | 统一请求适配器 | 需在用户测试设备逐项验证 |
| `nni_apr` | 原有 UI 页面（无副本） | 统一请求适配器 | 需在用户测试设备逐项验证 |
| `bancor` | 原有 UI 页面（无副本） | 统一请求适配器 | 需在用户测试设备逐项验证 |
| `assets` | 原有 UI 页面（无副本） | 统一请求适配器 | 需在用户测试设备逐项验证 |
| `services` | 原有 UI 页面（无副本） | 统一请求适配器 | 需在用户测试设备逐项验证 |
| `channels` | 原有 UI 页面（无副本） | 统一请求适配器 | 需在用户测试设备逐项验证 |
| `models` | 原有 UI 页面（无副本） | 统一请求适配器 | 需在用户测试设备逐项验证 |
| `skills` | 原有 UI 页面（无副本） | 统一请求适配器 | 需在用户测试设备逐项验证 |
| `aipps` | 原有 UI 页面（无副本） | 统一请求适配器 | 需在用户测试设备逐项验证 |
| `skill_store` | 原有 UI 页面（无副本） | 统一请求适配器 | 需在用户测试设备逐项验证 |
| `memory` | 原有 UI 页面（无副本） | 统一请求适配器 | 需在用户测试设备逐项验证 |
| `logs` | 原有 UI 页面（无副本） | 统一请求适配器 | 需在用户测试设备逐项验证 |
| `tasks` | 原有 UI 页面（无副本） | 统一请求适配器 | 需在用户测试设备逐项验证 |

## 必须适配的浏览器入口

| 文件 | fetch | localStorage | sessionStorage |
| --- | ---: | ---: | ---: |
| `UI/src/App.tsx` | 6 | 37 | 0 |
| `UI/src/components/AiLearningPage.tsx` | 0 | 2 | 0 |
| `UI/src/components/AippPage.tsx` | 0 | 3 | 2 |
| `UI/src/components/BancorPage.tsx` | 0 | 7 | 0 |
| `UI/src/components/ConsoleLayout.tsx` | 0 | 2 | 0 |
| `UI/src/components/DashboardPage.tsx` | 0 | 4 | 0 |
| `UI/src/components/NniAprPage.tsx` | 0 | 2 | 0 |
| `UI/src/hooks/useBancorRuntime.ts` | 0 | 1 | 0 |
| `UI/src/hooks/useChatRuntime.ts` | 0 | 5 | 0 |
| `UI/src/lib/nni-owner-public-key.ts` | 0 | 0 | 1 |

源码清单摘要：`36eee754be9ea182bffa190a2ef8144bb5b07559df0bb808969ff029a60cf3c5`。

桌面专属入口：设备添加与可信配对、HTTPS / SSH、系统凭据库、设备切换、下载保存、受限回环 Range 媒体通道、独立 AiAPP 窗口、DNS-SD 自动发现、可取消的有界 IPv4 局域网扫描。

构建时适配：`scripts/shared-ui-adapter.ts` 校验已知入口出现次数；共享 UI 调整后如合同不匹配，桌面构建明确失败。
