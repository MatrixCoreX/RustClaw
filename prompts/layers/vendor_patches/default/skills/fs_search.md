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
- Chinese search wording such as `找一下`、`搜一下`、`查一下`、`看看有没有` usually still implies a concrete search action rather than pure chat.
- When the user asks by file or directory name in Chinese, prefer `find_name`; if the user clearly asks by extension/后缀/扩展名, prefer `find_ext`; if the user asks whether certain text appears inside files, prefer `grep_text`.
- Chinese requests about `图片`、`照片`、`截图` usually map to image-search semantics rather than generic file-name search when the target is image-like.
- If the user asks `只列名字`、`只给路径`、`只说有没有`, keep the downstream answer minimal and avoid over-expanding raw search output.
- Chinese target nouns such as `那个目录`、`那个文件` still need an already-bound concrete target; do not use broad search as a substitute for missing deictic binding.
