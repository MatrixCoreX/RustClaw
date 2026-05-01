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
- When the current Chinese request semantically asks to inspect logs, recent errors, or abnormal findings, prefer baseline log analysis rather than raw tail output. Any examples in this section are illustrative only, not routing or matching rules.
- If the user gives a directory-like Chinese target and asks for the latest abnormal findings, it is reasonable to analyze the newest log-like file under that directory instead of asking for a file immediately.
- Chinese error-condition semantics can often be reflected into `keywords` when the user clearly wants narrowed analysis; examples are illustrative only.
- If the user only wants `最值得注意的一点`、`一句话总结`, keep the final answer concise and conclusion-first after analysis rather than dumping too many evidence rows.
- Distinguish analysis requests from raw tail/read requests by semantic intent: explicit requests for a tail/head/range read belong to direct log reading, not this summary-oriented analyzer.
