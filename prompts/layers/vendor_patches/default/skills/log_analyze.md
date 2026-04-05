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
- Chinese requests such as `看一下日志`、`帮我翻一下最近的报错`、`看看有没有异常` usually imply baseline log analysis rather than raw tail output.
- If the user gives a directory-like Chinese target such as `logs 目录` and asks for the latest abnormal findings, it is reasonable to analyze the newest log-like file under that directory instead of asking for a file immediately.
- Chinese keyword hints such as `报错`、`异常`、`超时`、`panic`、`失败` can often be reflected into `keywords` when the user clearly wants narrowed analysis.
- If the user only wants `最值得注意的一点`、`一句话总结`, keep the final answer concise and conclusion-first after analysis rather than dumping too many evidence rows.
- Distinguish analysis requests from raw tail/read requests: Chinese wording like `最后 20 行` or `读一下尾部` usually belongs to direct log reading, not this summary-oriented analyzer.
