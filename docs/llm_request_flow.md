# LLM Request Flow

本文记录当前 `clawd` 中会触发 LLM 请求的主要层级，以及 `ask` 主链路的大致调用顺序。

当前仓库默认配置位于 `configs/config.toml`：

- `selected_vendor = "minimax"`
- `selected_model = "MiniMax-M2.7"`
- `llm.minimax` 未设置 `api_format`

因此当前默认实际请求链路是：

`MiniMax -> openai_compat -> /chat/completions`

## Ask Main Flow

```mermaid
flowchart TD
    A["用户请求<br/>POST /v1/tasks kind=ask"] --> B["worker::process_ask_task"]
    B --> C["ask_prepare::prepare_ask_routing"]
    C --> D["intent_router::run_intent_normalizer"]
    D --> G["llm_gateway::run_with_fallback_with_prompt_source"]
    G --> H["providers::call_provider_with_retry"]
    H --> I["providers::call_provider"]
    I --> J["openai_compat<br/>/chat/completions"]
    I --> K["google_gemini<br/>:generateContent"]
    I --> L["anthropic_claude<br/>/v1/messages"]

    D --> M{"route_result.routed_mode"}

    M -->|Chat| N["ask_flow::execute_ask_routed<br/>生成 chat reply"]
    N --> G

    M -->|AskClarify| O{"能否复用现成澄清问题"}
    O -->|能| P["直接返回，不发 LLM"]
    O -->|不能| Q["intent_router::generate_clarify_question"]
    Q --> G

    M -->|Act / ChatAct| R["agent_engine::run_agent_with_tools"]
    R --> S["agent_engine::planning::plan_round_actions"]
    S --> G
    S --> T{"plan 可解析且可执行?"}
    T -->|否| U["agent_engine::planning::repair_plan_actions"]
    U --> G
    T -->|是| V["执行 tool / skill"]

    V --> W{"需要根据观察结果生成最终回答?"}
    W -->|是| X["agent_engine::observed_output"]
    X --> G
    W -->|否| Y["直接整理结果"]

    X --> Z{"需要语义判定可发布性?"}
    Y --> Z
    Z -->|是| AA["semantic_judge"]
    AA --> G
    Z -->|否| AB["finalize_ask_result"]

    AB --> AC{"需要长期记忆摘要刷新?"}
    AC -->|是| AD["memory::service"]
    AD --> G
    AC -->|否| AE["结束"]
```

## Ask Sequence

```mermaid
sequenceDiagram
    participant U as 用户
    participant W as worker/process_ask_task
    participant R as intent_router
    participant G as llm_gateway
    participant P as provider client
    participant A as ask_flow/agent_engine

    U->>W: 提交 ask 任务
    W->>R: run_intent_normalizer
    R->>G: run_with_fallback_with_prompt_source
    G->>P: call_provider_with_retry
    P-->>G: LLM 响应
    G-->>R: normalizer 结果
    R-->>W: route_result

    alt Chat
        W->>A: execute_ask_routed(Chat)
        A->>G: chat_response_prompt
        G->>P: 发起 LLM 请求
        P-->>G: 回复
        G-->>A: chat answer
    else AskClarify
        alt 可复用 clarify
            A-->>W: 直接返回
        else 需生成 clarify
            A->>G: clarify_question_prompt
            G->>P: 发起 LLM 请求
            P-->>G: 回复
            G-->>A: clarify question
        end
    else Act / ChatAct
        W->>A: run_agent_with_tools
        A->>G: plan_round_actions
        G->>P: 发起 LLM 请求
        P-->>G: plan
        alt plan 需修复
            A->>G: repair_plan_actions
            G->>P: 再发一次 LLM 请求
            P-->>G: repaired plan
        end
        A->>A: 执行 tool/skill
        opt 需要根据观察结果生成最终话术
            A->>G: observed_output fallback
            G->>P: 发起 LLM 请求
            P-->>G: 回复
        end
        opt 需要语义判定
            A->>G: semantic_judge
            G->>P: 发起 LLM 请求
            P-->>G: 回复
        end
    end

    opt ask 完成后的后台摘要
        W->>G: long_term_summary_prompt
        G->>P: 发起 LLM 请求
        P-->>G: 摘要结果
    end
```

## Runtime Layers

从“业务层”到“真正出网”的层次可以概括为：

1. 业务调用层：`intent_router`、`ask_flow`、`agent_engine`、`semantic_judge`、`memory::service`
2. 统一网关层：`llm_gateway::run_with_fallback_with_prompt_source`
3. 重试与 provider 选择层：`providers::call_provider_with_retry`
4. 协议适配层：`providers::call_provider`
5. 厂商接口层：
   - `openai_compat -> /chat/completions`
   - `google_gemini -> :generateContent`
   - `anthropic_claude -> /v1/messages`

## Notes

- 大多数 LLM 调用统一走 `llm_gateway`
- 少数旁路会直接打 provider，例如：
  - `skills/builtin.rs` 里的 `run_cmd` NL2CMD
  - `http/ui_routes.rs` 里的 LLM 连通性测试
