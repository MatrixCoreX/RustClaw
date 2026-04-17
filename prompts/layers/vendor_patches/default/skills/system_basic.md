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
- Chinese requests such as `看一下当前机器信息`、`扫一眼工作区`、`看看目录里都有啥` often map well to `info`, `workspace_glance`, or `inventory_dir` depending on target shape.
- Chinese counting requests like `有多少个文件`、`多少个文件夹`、`一共多少项` usually fit `count_inventory`; keep files / directories / total items distinguished instead of collapsing them.
- Chinese field-extraction requests such as `把 name 取出来`、`只读 package.name`、`只回版本号` usually map to `extract_field` / `extract_fields`, not broad file dumping.
- Chinese range-reading requests such as `开头 20 行`、`最后 10 行`、`第 3 到第 8 行` usually fit `read_range`.
- For Chinese output constraints like `只回数字`、`只回值`、`只回路径`, keep the final result scalar and avoid dumping the surrounding structured payload unless the user asked for it.
