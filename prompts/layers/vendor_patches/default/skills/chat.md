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
- Chinese lightweight chat requests such as `讲个笑话`、`随便聊两句`、`帮我吐槽一下` should keep `args.text` minimal and natural; do not stuff unrelated execution context into the chat input unless the user explicitly asks to base the reply on it.
- Chinese style requests such as `毒舌一点`、`轻松点`、`正经点`、`简短一点` can be reflected through the text/style choice, but avoid inventing unsupported mode values outside the interface contract.
- When the user asks for a very short Chinese reply such as `一句话` or `短一点`, keep the generated chat response terse instead of expanding with filler.
- If the Chinese request clearly asks for a code example or a factual answer grounded in execution output, prefer higher-level routing to execution/chat_act rather than forcing it into this lightweight chitchat skill.
