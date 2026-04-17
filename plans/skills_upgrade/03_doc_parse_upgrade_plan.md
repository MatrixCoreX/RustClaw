# doc_parse 功能增强计划（仅 skill 侧，不改 clawd）

## 目标
从 md/txt/html 解析扩展到常见办公文档，成为稳定的文档结构化入口。

## 当前现状
- 支持 md/txt/html 基础解析
- mode 已生效
- pdf/docx 尚未接入真实解析器

## 阶段拆分

### 阶段 A：格式覆盖
1. PDF 解析接入
- 先文本层提取
- 页面维度 metadata

2. DOCX 解析接入
- 段落与标题层提取
- 表格基础提取

### 阶段 B：结构质量
1. sections 提取标准化
- 标题层级统一
- section id + title + content

2. tables 结构统一
- header/rows 标准结构

### 阶段 C：边界与安全
1. 大文件截断策略
- max_chars + 分段截断说明

2. 编码容错
- 非 UTF-8 文本回退策略

## 接口增强建议
- 新增 include_metadata=true/false
- 新增 page_range（pdf）
- 新增 	able_mode（basic|strict）

## 验收标准
- pdf/docx 基础可读输出可用
- sections/tables 结构稳定
- 无伪造内容

## 风险与缓解
- 风险：第三方解析库质量不稳定
- 缓解：格式探测 + 明确错误码 + 回退文本模式

## 交付清单
- main.rs 新格式解析管线
- INTERFACE.md 支持矩阵更新
- prompt 对格式能力同步
