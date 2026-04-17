# reference_resolver 功能增强计划（仅 skill 侧，不改 clawd）

## 目标
把引用解析从规则 MVP 提升为可打分、可澄清、可复用的稳定组件。

## 当前现状
- 已支持上个/上上个回复与部分依赖/文件指代
- 缺少候选打分与置信度分层
- 复杂跨轮引用仍易误判

## 阶段拆分

### 阶段 A：候选召回与打分
1. 候选池构建
- 最近回复候选
- 最近任务结果候选
- 最近文件候选

2. 评分模型（规则分）
- 时间衰减
- role 匹配（reply/task/file）
- 关键词命中（上个/那个/依赖/文件）
- 语义近似（可先轻量）

3. 输出置信度
- 每个候选输出 score
- 主结果输出 confidence

### 阶段 B：歧义处理
1. mbiguous 统一格式
- 返回 top candidates
- 返回一句可直接发送的 clarify_question

2. 
ot_found 统一策略
- 明确没找到可绑定对象

### 阶段 C：可扩展能力
1. target_type 强化
- eply|task|file|dependency|generic 各自规则模板

2. 输出结构增强
- esolved_ref.kind
- esolved_ref.turn_index
- esolved_ref.id

## 接口增强建议
- 输入新增：language_hint、max_candidates
- 输出新增：esolution_trace（调试可选）

## 验收标准
- 上个/上上个回复命中率 > 99%
- 那个文件/那个依赖歧义场景能稳定给澄清
- 不再硬猜高风险引用

## 风险与缓解
- 风险：多候选内容高度相似
- 缓解：输出多候选 + 强制澄清

## 交付清单
- main.rs 评分与置信度实现
- INTERFACE.md 歧义/澄清规范
- prompt 明确低置信不硬猜
