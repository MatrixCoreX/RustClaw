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
- When the current Chinese request semantically asks for a general health or abnormality check without naming a narrower target, use the baseline default check and do not require extra args.
- If the user only wants the key conclusion in Chinese, prefer a concise result shape centered on the main risk or main abnormal point instead of replaying the whole diagnostic payload.
- Chinese follow-ups that semantically ask for the key point or shortest conclusion should keep the final answer short and user-facing after the check result is available.
- Do not ask for a narrower scope unless the user explicitly asks to inspect one specific service, directory, or log source.
