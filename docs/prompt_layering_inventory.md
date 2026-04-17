# Prompt Layering Inventory

## Goal

把旧的 vendor 整份复制维护方式，收敛成三层：

- `base`：系统真相与跨模型共通约束
- `overlay`：按任务类型或具体 prompt 叠加的正文
- `vendor_patch`：仅保留模型适配差异

当前运行时的分层入口由 [`crates/claw-core/src/prompt_layers.rs`](/home/guagua/git_upload/crates/claw-core/src/prompt_layers.rs) 提供共享 helper，`clawd` 在 [`crates/clawd/src/bootstrap/prompts.rs`](/home/guagua/git_upload/crates/clawd/src/bootstrap/prompts.rs) 里接入，清单由 [`prompts/layers/manifest.toml`](/home/guagua/git_upload/prompts/layers/manifest.toml) 定义。

## 职责分组

### 主行为约束

- `prompts/layers/base/system_truth.md`
- `prompts/layers/base/execution/common_rules.md`
- `prompts/layers/base/routing/common_rules.md`
- `prompts/layers/base/recovery/common_rules.md`
- `prompts/layers/base/skills/common_rules.md`

这些内容属于所有模型共享的系统真相：

- 隐藏策略不可泄露
- 记忆与历史是辅助，不是权威输入
- 不得捏造路径、技能、参数、执行结果
- 路径定位、clarify、文件发送、计数语义等共通约束

### 执行类 prompt

已接入分层加载：

- `prompts/agent_tool_spec.md`
- `prompts/single_plan_execution_prompt.md`
- `prompts/loop_incremental_plan_prompt.md`
- `prompts/plan_repair_prompt.md`

这批 prompt 的 overlay 正文已经迁到 `prompts/layers/overlays/*.md`；运行时不再以各 vendor 的整份副本为主来源。

### 路由类 prompt

已接入分层加载：

- `prompts/intent_normalizer_prompt.md`

> Phase 2.7：legacy `prompts/intent_router_prompt.md` 与 `prompts/intent_router_rules.md`
> 已删除。normalizer 失败/解析失败时直接走 `deterministic_fallback_route_decision`
> （仓库内硬规则降级），不再发起第二次 LLM 调用。

共通规则包括：

- 语义路由优先于关键词匹配
- 文件/目录/本地环境检查属于可执行请求
- delivery 与 inspect 分流
- fresh deictic 需要唯一绑定，否则澄清

### 恢复 / 澄清类 prompt

已接入分层加载：

- `prompts/clarify_question_prompt.md`
- `prompts/resume_followup_discussion_prompt.md`
- `prompts/resume_continue_execute_prompt.md`
- `prompts/resume_followup_intent_prompt.md`

共通规则包括：

- 只在用户明确要求时恢复执行
- 新请求不要被旧失败任务误绑定
- 澄清必须围绕原操作缺失的 locator 或 scope

### Skill prompt

当前统一主干来源：

- `INTERFACE.md`
- `scripts/sync_skill_docs.py`
- `prompts/layers/generated/skills/<skill>.md`

运行时策略已调整为：

- skill 主体始终来自 `prompts/layers/generated/skills/<skill>.md`
- 可选 vendor 差异只允许放在 `prompts/layers/vendor_patches/<vendor>/skills/<skill>.md`

这意味着旧的 vendor skill 全量副本已经退出主链；当前 vendor skill 差异改由 `prompts/layers/vendor_patches/<vendor>/skills/*.md` 承载。

### 文本 / 摘要 / 多模态家族

已接入分层加载：

- `prompts/chat_response_prompt.md`
- `prompts/long_term_summary_prompt.md`
- `prompts/schedule_intent_prompt.md`
- `prompts/schedule_intent_rules.md`
- `prompts/voice_chat_prompt.md`
- `prompts/voice_mode_intent_prompt.md`
- `prompts/chat_skill_system_prompt.md`
- `prompts/chat_skill_joke_system_prompt.md`
- `prompts/audio_transcribe_prompt.md`
- `prompts/image_reference_resolver_prompt.md`
- `prompts/image_output_rewrite_prompt.md`
- `prompts/language_infer_prompt.md`
- `prompts/image_vision_prompt.md`
- `prompts/image_vision_language_hint_*`
- `prompts/image_vision_action_*`
- `prompts/personas/*.md`（逻辑路径；实际正文位于 `prompts/layers/overlays/personas/*.md`）

这些 prompt 现在也通过 manifest 进入统一分层，相关独立进程（`telegramd`、`chat-skill`、`audio-transcribe-skill`、`image-edit-skill`、`image-vision-skill`）改为复用同一套共享解析 helper。

### Vendor 特化内容

当前只保留薄 patch：

- `prompts/layers/vendor_patches/default/execution/common.md`
- `prompts/layers/vendor_patches/default/routing/common.md`
- `prompts/layers/vendor_patches/default/recovery/common.md`
- `prompts/layers/vendor_patches/default/text/common.md`
- `prompts/layers/vendor_patches/default/summary/common.md`
- `prompts/layers/vendor_patches/default/vision/common.md`

新增 vendor patch 的准入规则：

- 必须能明确说明为什么只有该模型需要这条
- 只能写模型适配，不写系统共通规则
- 优先补 JSON 严格度、schema fidelity、输出紧凑度等差异

## 共通规则 vs Vendor 差异

### 应归入共通规则

- 文件发送规则
- 目录查找与 locator 限制
- clarify 原则
- 输出 contract
- 文件计数语义
- 历史/记忆的权威性边界

### 才应进入 vendor patch

- 更强 JSON 严格度
- 更强禁止多对象输出
- 更强调 schema fidelity
- 更强“不要输出额外说明”的模型约束

## Legacy vendor 树状态

`prompts/vendors/<vendor>/...` 现在已经退出运行时正文链，并已从仓库中物理移除。

因此 legacy vendor 树不再承载任何已迁移 prompt 的正文真相，也不再保留兼容占位目录。

### 已确认仍是有效来源的部分

- `prompts/layers/generated/skills/*.md`
  - 这批文件继续作为 skill prompt 主体，由 `INTERFACE.md` 经 `scripts/sync_skill_docs.py` 生成/更新
- `prompts/layers/vendor_patches/<vendor>/skills/*.md`
  - 这批文件承载薄的模型差异；只保留 vendor tuning，不再复制整份 skill 正文

### 已清理的 legacy 残留

此前盘点出的 7 个非技能 legacy default 残留文件，以及已迁移非技能 default 正文副本，现已完成物理清理。

旧的 vendor skill 副本正文也已迁移到 `prompts/layers/generated/skills/ + vendor_patches` 结构；相关 legacy 目录与 README 占位也已一并删除。

## 维护规则

- 新规则先判断能否代码化；能代码化的优先回到代码而不是 prompt。
- 共通行为规则进 `base`。
- 任务特有规则进 `overlay`。
- 只有模型适配才允许进 `vendor_patch`。
- 改 prompt 时，优先改分层源，不要再修改多个 vendor 完整副本来保持同步。
