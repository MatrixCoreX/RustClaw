You generate the first implementation for a reusable RustClaw external skill scaffold.

Return one JSON object only. No markdown fences. No explanations outside JSON.

Required output shape:
{
  "readme_md": "full README.md content",
  "interface_md": "full INTERFACE.md content",
  "main_rs": "full src/main.rs content"
}

Rules:
- The generated skill must follow RustClaw's single-line JSON stdin/stdout protocol.
- Use only Rust standard library plus `anyhow`, `serde`, and `serde_json`.
- Do not assume any other crates or edit `Cargo.toml`.
- The provided `skill_name`, `capability_summary`, and `actions` are the contract baseline. Keep the action list aligned with them.
- `README.md` should explain what the scaffold does, its current scope, and the next safe steps. Mention that the skill is not registered or enabled by default.
- `INTERFACE.md` must include:
  - capability summary
  - action list
  - parameter contract table
  - error contract
  - at least 2 request/response JSON examples
- `src/main.rs` must be a complete Rust binary, not pseudocode. Keep it conservative and grounded.
- If the original request is broader than what can be safely implemented with the current scaffold/dependencies, implement the narrow core behavior and return readable `error_text` for unsupported or missing inputs.
- Prefer bounded file-local logic. Do not modify RustClaw runtime config, registry files, or other repository code.

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
- 如果原始请求是中文，也要产出英文代码与英文接口文档结构；语言差异只体现在示范内容和 README 说明可适度双语化，但不要让代码注释变成大段中文。
- 对明显超出当前依赖能力的需求，不要假装实现完整能力；应保守实现最核心闭环，并在 README / INTERFACE 的错误约定里明确边界。
