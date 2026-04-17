#!/usr/bin/env bash
set -euo pipefail

ROOT="${1:-$(pwd)}"

python3 - "$ROOT" <<'PY'
import shlex
import sys
from pathlib import Path

try:
    import tomllib
except ModuleNotFoundError:
    import tomli as tomllib

root = Path(sys.argv[1]).resolve()


def load_toml(path: Path):
    if not path.exists():
        return {}
    try:
        with path.open("rb") as f:
            data = tomllib.load(f)
            return data if isinstance(data, dict) else {}
    except Exception:
        return {}


def section(data, *keys):
    cur = data
    for key in keys:
        if not isinstance(cur, dict):
            return {}
        cur = cur.get(key, {})
    return cur if isinstance(cur, dict) else {}


def value(data, *keys):
    cur = data
    for key in keys:
        if not isinstance(cur, dict):
            return ""
        cur = cur.get(key)
    if cur is None:
        return ""
    if isinstance(cur, str):
        return cur
    if isinstance(cur, bool):
        return "true" if cur else "false"
    return str(cur)


def emit(name, value_text):
    print(f"export {name}={shlex.quote(value_text or '')}")


config = load_toml(root / "configs/config.toml")
telegram_cfg = load_toml(root / "configs/channels/telegram.toml")
whatsapp_cfg = load_toml(root / "configs/channels/whatsapp.toml")
whatsapp_cloud_cfg = load_toml(root / "configs/channels/whatsapp-cloud.toml")
feishu_cfg = load_toml(root / "configs/channels/feishu.toml")
lark_cfg = load_toml(root / "configs/channels/lark.toml")
wechat_cfg = load_toml(root / "configs/channels/wechat.toml")
audio_cfg = load_toml(root / "configs/audio.toml")
image_cfg = load_toml(root / "configs/image.toml")
crypto_cfg = load_toml(root / "configs/crypto.toml")

llm = section(config, "llm")
telegram = section(telegram_cfg, "telegram") or section(config, "telegram")
telegram_compat = section(config, "telegram_bot")
whatsapp = section(whatsapp_cfg, "whatsapp") or section(config, "whatsapp")
whatsapp_cloud = section(whatsapp_cloud_cfg, "whatsapp_cloud") or section(config, "whatsapp_cloud")
feishu = section(feishu_cfg, "feishu")
lark = section(lark_cfg, "lark")
wechat = section(wechat_cfg, "wechat")
audio_synthesize = section(audio_cfg, "audio_synthesize")
audio_transcribe = section(audio_cfg, "audio_transcribe")
image_generation = section(image_cfg, "image_generation")
image_edit = section(image_cfg, "image_edit")
image_vision = section(image_cfg, "image_vision")
print("# Generated from current TOML config files. URL fields are intentionally excluded.")
print("# Usage: source <(bash scripts/export_runtime_env_from_configs.sh)")
print()

print("# Global LLM provider API keys")
for vendor, env_name in [
    ("openai", "OPENAI_API_KEY"),
    ("google", "GOOGLE_API_KEY"),
    ("anthropic", "ANTHROPIC_API_KEY"),
    ("grok", "GROK_API_KEY"),
    ("deepseek", "DEEPSEEK_API_KEY"),
    ("qwen", "QWEN_API_KEY"),
    ("minimax", "MINIMAX_API_KEY"),
    ("custom", "CUSTOM_API_KEY"),
]:
    emit(env_name, value(llm, vendor, "api_key"))
print()

print("# Communication channels")
emit("TELEGRAM_BOT_TOKEN", value(telegram, "bot_token") or value(telegram_compat, "bot_token"))
emit("WHATSAPP_ACCESS_TOKEN", value(whatsapp, "access_token"))
emit("WHATSAPP_APP_SECRET", value(whatsapp, "app_secret"))
emit("WHATSAPP_VERIFY_TOKEN", value(whatsapp, "verify_token"))
emit("WHATSAPP_PHONE_NUMBER_ID", value(whatsapp, "phone_number_id"))
emit("WHATSAPP_CLOUD_ACCESS_TOKEN", value(whatsapp_cloud, "access_token"))
emit("WHATSAPP_CLOUD_APP_SECRET", value(whatsapp_cloud, "app_secret"))
emit("WHATSAPP_CLOUD_VERIFY_TOKEN", value(whatsapp_cloud, "verify_token"))
emit("WHATSAPP_CLOUD_PHONE_NUMBER_ID", value(whatsapp_cloud, "phone_number_id"))
emit("FEISHU_APP_ID", value(feishu, "app_id"))
emit("FEISHU_APP_SECRET", value(feishu, "app_secret"))
emit("FEISHU_VERIFICATION_TOKEN", value(feishu, "verification_token"))
emit("FEISHU_ENCRYPT_KEY", value(feishu, "encrypt_key"))
emit("LARK_APP_ID", value(lark, "app_id"))
emit("LARK_APP_SECRET", value(lark, "app_secret"))
emit("LARK_VERIFICATION_TOKEN", value(lark, "verification_token"))
emit("LARK_ENCRYPT_KEY", value(lark, "encrypt_key"))
emit("WECHAT_BOT_TOKEN", value(wechat, "bot_token"))
emit("WECHAT_UIN_BASE64", value(wechat, "wechat_uin_base64"))
emit("WECHAT_SK_ROUTE_TAG", value(wechat, "sk_route_tag"))
print()

print("# Audio skills")
for vendor, env_name in [
    ("openai", "AUDIO_SYNTHESIZE_OPENAI_API_KEY"),
    ("google", "AUDIO_SYNTHESIZE_GOOGLE_API_KEY"),
    ("anthropic", "AUDIO_SYNTHESIZE_ANTHROPIC_API_KEY"),
    ("grok", "AUDIO_SYNTHESIZE_GROK_API_KEY"),
    ("deepseek", "AUDIO_SYNTHESIZE_DEEPSEEK_API_KEY"),
    ("qwen", "AUDIO_SYNTHESIZE_QWEN_API_KEY"),
    ("minimax", "AUDIO_SYNTHESIZE_MINIMAX_API_KEY"),
    ("custom", "AUDIO_SYNTHESIZE_CUSTOM_API_KEY"),
]:
    emit(env_name, value(audio_synthesize, "providers", vendor, "api_key"))
for vendor, env_name in [
    ("openai", "AUDIO_TRANSCRIBE_OPENAI_API_KEY"),
    ("google", "AUDIO_TRANSCRIBE_GOOGLE_API_KEY"),
    ("anthropic", "AUDIO_TRANSCRIBE_ANTHROPIC_API_KEY"),
    ("grok", "AUDIO_TRANSCRIBE_GROK_API_KEY"),
    ("deepseek", "AUDIO_TRANSCRIBE_DEEPSEEK_API_KEY"),
    ("qwen", "AUDIO_TRANSCRIBE_QWEN_API_KEY"),
    ("minimax", "AUDIO_TRANSCRIBE_MINIMAX_API_KEY"),
    ("custom", "AUDIO_TRANSCRIBE_CUSTOM_API_KEY"),
]:
    emit(env_name, value(audio_transcribe, "providers", vendor, "api_key"))
emit("AUDIO_TRANSCRIBE_OSS_ACCESS_KEY_ID", value(audio_transcribe, "oss_access_key_id"))
emit("AUDIO_TRANSCRIBE_OSS_ACCESS_KEY_SECRET", value(audio_transcribe, "oss_access_key_secret"))
print()

print("# Image skills")
for vendor, env_name in [
    ("openai", "IMAGE_GENERATION_OPENAI_API_KEY"),
    ("google", "IMAGE_GENERATION_GOOGLE_API_KEY"),
    ("anthropic", "IMAGE_GENERATION_ANTHROPIC_API_KEY"),
    ("grok", "IMAGE_GENERATION_GROK_API_KEY"),
    ("deepseek", "IMAGE_GENERATION_DEEPSEEK_API_KEY"),
    ("qwen", "IMAGE_GENERATION_QWEN_API_KEY"),
    ("minimax", "IMAGE_GENERATION_MINIMAX_API_KEY"),
]:
    emit(env_name, value(image_generation, "providers", vendor, "api_key"))
for vendor, env_name in [
    ("openai", "IMAGE_EDIT_OPENAI_API_KEY"),
    ("google", "IMAGE_EDIT_GOOGLE_API_KEY"),
    ("anthropic", "IMAGE_EDIT_ANTHROPIC_API_KEY"),
    ("grok", "IMAGE_EDIT_GROK_API_KEY"),
    ("deepseek", "IMAGE_EDIT_DEEPSEEK_API_KEY"),
    ("qwen", "IMAGE_EDIT_QWEN_API_KEY"),
    ("minimax", "IMAGE_EDIT_MINIMAX_API_KEY"),
]:
    emit(env_name, value(image_edit, "providers", vendor, "api_key"))
emit("IMAGE_EDIT_OSS_ACCESS_KEY_ID", value(image_edit, "oss_access_key_id"))
emit("IMAGE_EDIT_OSS_ACCESS_KEY_SECRET", value(image_edit, "oss_access_key_secret"))
for vendor, env_name in [
    ("openai", "IMAGE_VISION_OPENAI_API_KEY"),
    ("google", "IMAGE_VISION_GOOGLE_API_KEY"),
    ("anthropic", "IMAGE_VISION_ANTHROPIC_API_KEY"),
    ("grok", "IMAGE_VISION_GROK_API_KEY"),
    ("deepseek", "IMAGE_VISION_DEEPSEEK_API_KEY"),
    ("qwen", "IMAGE_VISION_QWEN_API_KEY"),
    ("minimax", "IMAGE_VISION_MINIMAX_API_KEY"),
]:
    emit(env_name, value(image_vision, "providers", vendor, "api_key"))
PY
