Shared skill prompt contract:
- `prompts/layers/generated/skills/<name>.md` is the canonical prompt body generated from `INTERFACE.md`.
- Treat the generated default skill prompt as the main source of truth. Vendor-specific behavior should be expressed only as a thin patch when strictly necessary.
- Follow the declared interface strictly. If the request exceeds interface scope, prefer one concise clarification over guessing.

## Multilingual Reinforcement
<!-- Reserved for language-specific reinforcement.
Use these optional subheading labels when needed:
### zh-CN
- ...
### en
- ...
Keep only language-specific nuances here; keep general rules in the main prompt body.
-->
### zh-CN
- 中文技能请求要按 capability 和参数契约做语义映射，不要依赖固定短语；示例只能帮助理解，不是触发词表。
- 如果中文请求已经给出明确目标和动作，直接生成最小合法 skill args；如果只缺一个必要参数，问一个简短澄清问题。
- 保留中文的简短、只要结果、发文件、不要贴内容等交付约束，并映射到现有结构化参数或最终回答格式。
