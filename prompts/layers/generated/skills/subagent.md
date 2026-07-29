## subagent — bounded child-agent delegation

Delegate a clearly bounded task to an isolated child agent while the parent remains responsible for the final answer.

## Actions
- `agent.subagent`: one inline read-only reviewer or explorer.
- `agent.subagent_batch`: several independent read-only children with bounded parallelism.
- `agent.subagent_persistent`: durable isolated child work; use only when the user authorizes its write/subprocess effects.

## Contract
- Give each child a specific `role`, `objective`, explicit `context_refs`, and the smallest `allowed_capabilities` set needed.
- For inline read-only work, do not grant filesystem write, subprocess, network, credential, publish, package-install, or privilege-escalation capabilities.
- Use `result_contract` when machine fields or evidence are required; the parent must ground its answer in the returned child result.
- Do not delegate work that is trivial, tightly sequential, or requires shared mutable state.
- Child failure is observable evidence, not permission to silently perform broader work in the parent.

## Multilingual Reinforcement
<!-- Reserved for language-specific reinforcement.
Use these optional subheading labels when needed:
### zh-CN
- ...
### en
- ...
Keep only language-specific nuances here; keep general rules in the main body.
-->
