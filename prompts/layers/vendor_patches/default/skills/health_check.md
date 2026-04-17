## Multilingual Reinforcement
<!-- Reserved for language-specific reinforcement.
Use subheadings such as:
### zh-CN
- ...
### en
- ...
Keep only language-specific nuances here; keep general rules in the main prompt body.
-->
### zh-CN
- Chinese requests such as `做个健康检查`、`帮我看看有没有异常`、`系统现在稳不稳` usually mean the baseline default check and should not require extra args.
- If the user only wants the key conclusion in Chinese, prefer a concise result shape such as the main risk / main abnormal point instead of replaying the whole diagnostic payload.
- Chinese follow-ups like `最该注意什么`、`一句话说重点` should keep the final answer short and user-facing after the check result is available.
- Do not ask for a narrower scope unless the user explicitly asks to inspect one specific service, directory, or log source.
