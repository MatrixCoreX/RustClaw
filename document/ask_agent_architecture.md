# Ask/Agent 主链架构说明（收口后）

本文档说明当前 clawd 中 ask 主链与 agent LLM 的收口状态，供后续维护和演进时对齐。重点：主链怎么走、哪些是旧兼容、哪些不要再接回主链。

---

## 1. Ask 主链当前顺序

1. **入口**：`POST /v1/tasks`，`kind=ask`；worker 在 `main.rs` 的 `worker_once()` 中处理。
2. **前置理解（唯一 LLM）**：`intent_router::run_intent_normalizer(...)` 一次完成：
   - 承接/继续判断（resume_behavior）
   - 意图补全（resolved_user_intent）
   - 调度意图（schedule_kind）
   - 是否需澄清（needs_clarify）
   - **终端 mode**（routed_mode：chat / act / ask_clarify / chat_act）
3. **分支**：
   - 若 `needs_clarify` → `generate_clarify_question`，直接返回澄清问题，不再进 agent。
   - 若 `schedule_kind != None` 且符合规则 → `try_handle_schedule_request`，走调度逻辑。
   - 否则 → `execute_ask_routed(..., Some(normalizer_out.routed_mode))`，用 **normalizer 的 mode** 直接路由，**不再**调用旧的 intent_router / context_resolver / resume_followup_intent。
4. **执行**：
   - **Chat**：单次 chat 回复 LLM，结果即最终交付。
   - **Act**：`agent_engine::run_agent_with_tools`，多轮 plan → execute；最终交付来自 respond 或 raw fallback 或 synthesize fallback（见下）。
   - **ChatAct**：同上 agent，但 goal 中带「先执行再给一句自然语言总结」的 hint；仅当用户**显式**要求「执行 + 总结/解释」时使用，不做任何不确定兜底。
   - **AskClarify**：生成澄清问题后返回。

主链**不**再依赖：旧的多层前置 LLM（resume_followup_intent → context_resolver → intent_router）。

---

## 2. intent_normalizer 的职责

- **唯一前置理解入口**：一次 LLM 调用产出 `IntentNormalizerOutput`，包含：
  - `resolved_user_intent`：补全后的用户意图（供后续 memory + agent 使用）
  - `resume_behavior`：none / resume_execute / resume_discuss
  - `schedule_kind`：none / create / update / delete / query
  - `needs_clarify`、`reason`、`confidence`
  - **`routed_mode`**：chat | act | ask_clarify | chat_act，用于直接路由，**不再**经过单独 router LLM。
- 若 normalizer 解析失败（如 JSON 解析失败），主路径仍可走，但 mode 会退化为由 **route_request_mode**（旧 router）兜底；该路径仅作 fallback，不应作为主链设计。

---

## 3. Progress / delivery / trace 的职责划分

| 层级 | 用途 | 写入来源 | 是否进入用户可见最终结果 |
|------|------|----------|---------------------------|
| **progress** | 阶段/进度提示，「处理中」展示 | 仅 `append_progress_hint` → progress_messages | 否（仅 progress API，非最终 result） |
| **delivery** | 最终交付给用户的内容 | respond 或 fallback_finalize_from_raw 或 synthesize_final_response | 是（AskReply.messages / result） |
| **trace** | 日志、调试、排障 | subtask_results、register_step_output、history_compact | 否 |

- 详见 `document/ask_act_delivery_semantics.md`。
- 规则：raw tool/skill 输出默认**不**直接进 delivery，只进 step output / loop state / trace；最终交付只由 respond 或系统 fallback 产出。

---

## 4. Raw output / respond / fallback finalizer 的关系

- **Respond**：planner 产出的「最终回复」动作；若 `should_publish_respond_message` 为 true，则**只**写入 delivery，作为主要最终交付。
- **Raw fallback**：当 loop 结束且 delivery 为空但 subtask_results 非空时，用 `fallback_finalize_from_raw` 对 raw 做**最小过滤**（排除空、纯过程提示、内部确认语等）后拼成一条写入 delivery；若过滤后为空则不写，留给 synthesize。
- **Synthesize fallback**：当 delivery 仍为空且存在 tool/skill 输出时，调用 chat 技能生成一条自然语言总结并写入 delivery。
- 顺序：**respond 优先 → 无则 raw fallback（过滤后）→ 仍无则 synthesize**。不鼓励无必要的重复自然语言包装。

---

## 5. Legacy 路径还剩哪些

以下**不得**作为 ask 主链的默认或推荐路径；仅作兼容或 parse 失败时的 fallback：

| 名称 | 位置 | 说明 |
|------|------|------|
| `classify_resume_followup_intent` | intent_router.rs | 已被 normalizer 的 resume_behavior 替代；不用于 ask worker。 |
| `resolve_user_request_with_context` | intent_router.rs | 已被 normalizer 的 resolved_user_intent + needs_clarify 替代；不用于 ask worker。 |
| `route_request_mode` | intent_router.rs | **Fallback**：仅当 normalizer 未提供 mode（如 JSON 解析失败）时调用；主链应始终使用 `Some(normalizer_out.routed_mode)`。 |

代码中已用 `**[LEGACY]**` / `**[FALLBACK]**` 和模块头注释标明；后续开发不要把这些函数再接回 ask 主链。

---

## 6. chat_act 的最终定位

- **保留**，但**仅**在用户**显式**要求「执行 + 总结/解释/播报」时触发。
- **不允许**作为不确定时的兜底；优先使用 `act`，需要总结时由 act 的 respond / fallback 机制承担。
- 实现上：normalizer 可输出 mode=chat_act；`execute_ask_routed` 中 ChatAct 分支仅加 goal hint（先执行再给一句自然语言总结），不再扩展用途。
- Prompt 与代码约定一致：`intent_normalizer_prompt`（及各 vendor）中均写明「chat_act 仅当用户明确要求 action + narrated summary；不得作为 fallback」。

---

## 7. 后续演进建议

- **主链**：保持「normalizer 一次 → 按 mode 分支 → act 内 progress/delivery/trace 分离」；不再增加新的前置 LLM 层。
- **Legacy**：可逐步将 `classify_resume_followup_intent` / `resolve_user_request_with_context` 标记为 deprecated 或仅在非 ask 路径保留；`route_request_mode` 仅作 parse 失败兜底，可考虑在 normalizer 输出中增加更强健的解析以进一步减少调用。
- **chat_act**：若长期极少被命中，可考虑在 normalizer 中不再输出 chat_act，统一用 act + respond 覆盖；当前保留以便「执行+播报」类需求有明确语义。

---

*本文档与 `ask_act_delivery_semantics.md` 一起构成当前 ask/agent 交付与架构的收口说明。*
