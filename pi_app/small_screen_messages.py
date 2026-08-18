import re


_CHANNEL_ALIASES = {
    "web": "ui",
    "web-ui": "ui",
    "whatsapp-cloud": "whatsapp",
    "whatsapp-web": "whatsapp",
    "wechat-web": "wechat",
}


def normalize_message_channel(value):
    channel = str(value or "").strip().strip('\",;]}').lower().replace("_", "-")
    return _CHANNEL_ALIASES.get(channel, channel)


def extract_message_channel(line):
    if not line:
        return ""
    # Structured task metadata is appended after call_id. Restrict parsing to
    # that suffix so user text such as "channel=telegram" cannot spoof it.
    call_marker = line.rfind(" call_id=")
    if call_marker < 0:
        return ""
    metadata = line[call_marker:]
    matches = list(re.finditer(r'(?:^|\s)(?:channel|task_channel)="?([^\s"]+)', metadata))
    if not matches:
        return ""
    return normalize_message_channel(matches[-1].group(1))


def message_channel_display_name(channel, lang="CN"):
    normalized = normalize_message_channel(channel)
    labels = {
        "ui": {"CN": "网页端", "EN": "Web UI"},
        "wechat": {"CN": "微信", "EN": "WeChat"},
        "telegram": {"CN": "Telegram", "EN": "Telegram"},
        "whatsapp": {"CN": "WhatsApp", "EN": "WhatsApp"},
        "feishu": {"CN": "飞书", "EN": "Feishu"},
        "lark": {"CN": "Lark", "EN": "Lark"},
        "cli": {"CN": "本机命令行", "EN": "Local CLI"},
        "schedule": {"CN": "定时任务", "EN": "Scheduled task"},
    }
    selected_lang = "EN" if lang == "EN" else "CN"
    if normalized in labels:
        return labels[normalized][selected_lang]
    if normalized:
        return normalized
    return "Unknown source" if selected_lang == "EN" else "来源未知"
