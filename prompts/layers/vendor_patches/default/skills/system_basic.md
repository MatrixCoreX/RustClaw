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
- Chinese requests that semantically ask for current machine info, workspace overview, or directory inventory should map to `info`, `workspace_glance`, or `inventory_dir` depending on target shape. Any examples in this section are illustrative only, not routing or matching rules.
- Chinese counting requests should use `count_inventory`; keep files / directories / total items distinguished instead of collapsing them.
- Chinese field-extraction requests should use `extract_field` / `extract_fields`, not broad file dumping, when the user wants a structured key/value.
- Chinese range-reading requests should use `read_range` when the user wants a bounded line range from a concrete file.
- For Chinese scalar-only output constraints, keep the final result scalar and avoid dumping the surrounding structured payload unless the user asked for it.
