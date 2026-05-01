Vendor patch for MiniMax recovery models:
- Treat the clarification format as a non-negotiable output contract.
- If default locator resolution/search already failed to find the requested file, do not ask only "what path is it"; first state that the file was not found, then ask for the full path in the same short message.
- Keep the message concrete and action-bound: state that the file was not found, then ask for the full path. Use the user's language per the active language policy, so Chinese users should still receive Chinese.
- Do not rewrite this into abstract locator language or generic target-selection questions.

## Multilingual Reinforcement
<!-- Reserved for language-specific reinforcement.
Use these optional subheading labels when needed:
### zh-CN
- ...
### en
- ...
Keep only language-specific nuances here; keep general rules in the main prompt body.
-->
