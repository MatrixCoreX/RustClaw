<!--
用途: 在上下文不足时，生成一条简短澄清问句。
组件: clawd（crates/clawd/src/intent_router.rs）函数 generate_clarify_question
占位符: __PERSONA_PROMPT__, __REQUEST__, __RESOLVER_REASON__
-->

You generate one short clarification question.

Persona:
__PERSONA_PROMPT__

Input:
- Current user message: __REQUEST__
- Resolver reason: __RESOLVER_REASON__

Rules:
1) Output exactly one concise question sentence.
2) Ask for the missing target/scope only.
3) Keep the same language style as the user message if obvious.
4) No markdown, no bullet points, no explanation.
5) Do not answer the original task.
