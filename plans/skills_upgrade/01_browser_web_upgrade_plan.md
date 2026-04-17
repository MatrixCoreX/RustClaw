# browser_web 功能增强计划（仅 skill 侧，不改 clawd）

## 目标
将 rowser_web 从可用 MVP 提升为稳定生产级网页抓取能力：
- 页面抓取稳定
- 动态页面兼容更好
- 输出可读且可追踪
- 错误可分类可回归

## 当前现状
- 已支持 open_extract / search_page / search_extract
- 已接入 Playwright，无头模式可运行
- 已有基础正文清洗与截图保存
- 仍存在站点结构变更、超时、反爬与稳定性风险

## 阶段拆分

### 阶段 A：抓取稳定性
1. 导航策略增强
- 支持 goto 多策略回退（domcontentloaded/load/networkidle）
- 增加按站点的等待策略映射（可配置）
- 超时错误携带尝试轨迹

2. 页面准备判定
- 增加 页面可读性最低阈值检查（正文长度、标题可用性）
- 对疑似空白页/阻断页输出明确错误码

3. Google 结果抽取容错
- 增加多套 selector fallback
- 添加结构漂移诊断字段

### 阶段 B：内容质量
1. 正文抽取器升级
- 从 body 全量文本升级为主内容优先抽取
- 过滤导航、页脚、侧栏高噪内容

2. 文本清洗规则分层
- 保留 clean/raw 双模式
- clean 默认启用，aw 用于调试

3. 引用信息增强
- 输出 	itle/source/final_url/screenshot_path
- 增加 extracted_at 时间戳

### 阶段 C：可观测与可维护
1. 错误码规范化
- 统一错误码：NAV_TIMEOUT BOT_BLOCKED SELECTOR_MISS EMPTY_CONTENT DEPENDENCY_MISSING

2. 运行元数据
- 每页输出 
av_wait_until ttempts latency_ms

3. 回归测试集
- 固定 10-20 个站点样本（GitHub/新闻/博客/重 JS）
- 每次改动后跑解析稳定性回归

## 接口增强建议
- open_extract 新增：
  - content_mode (clean|raw, default clean)
  - max_text_chars (default 12000)
  - ail_fast (default false)
- search_page 新增：
  - egion lang（可选）
- search_extract 新增：
  - summarize (default true)

## 验收标准
- GitHub 类页面成功率 > 95%
- 常见新闻站正文抽取可读率 > 90%
- 错误均可映射到标准错误码
- 输出不再包含明显 CSS/JS 噪声

## 风险与缓解
- 风险：搜索页 DOM 频繁变更
- 缓解：selector 多路回退 + 结构漂移报警
- 风险：目标站点反爬
- 缓解：限流、重试、明确失败提示

## 交付清单
- rowser_web.js 稳定性与抽取增强
- INTERFACE.md 参数与错误码更新
- vendor skill prompt 同步
- 最小回归脚本（仅 skill 侧）
