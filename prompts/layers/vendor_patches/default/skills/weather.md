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
- Chinese weather requests often use colloquial date wording; normalize it semantically without forcing the user to restate it in English. Examples are illustrative only.
- If the Chinese city name can be confidently converted to a standard English geocoding name, prefer converting and executing directly instead of asking an avoidable clarification.
- Ask for clarification only when the Chinese place name is genuinely ambiguous, the English form is low-confidence, or the requested place may still fail geocoding without confirmation.
- Chinese weather follow-ups that change only the requested date should keep the same place when that place is already uniquely established in immediate context.
- Keep user-visible clarification/result language in Chinese when configured, even if the actual `city` arg must be normalized into English.
