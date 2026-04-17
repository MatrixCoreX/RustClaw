#!/usr/bin/env python3
import argparse
import base64
import io
import json
import mimetypes
import struct
import sys
import time
import wave
from pathlib import Path

try:
    import tomllib
except ModuleNotFoundError:
    print("Python 3.11+ is required (tomllib missing).", file=sys.stderr)
    sys.exit(2)

import requests


ROOT = Path(__file__).resolve().parents[1]
CONFIG_TOML = ROOT / "configs" / "config.toml"
AUDIO_TOML = ROOT / "configs" / "audio.toml"
IMAGE_TOML = ROOT / "configs" / "image.toml"

# 1x1 PNG, opaque black pixel.
MINIMAL_BLACK_PNG_BASE64 = (
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR42mNkZGJmAAAAsQCEzqt2WQAAAABJRU5ErkJggg=="
)
DEFAULT_PUBLIC_IMAGE_URL = "https://picsum.photos/id/237/512/512"


def load_toml(path: Path) -> dict:
    with path.open("rb") as f:
        return tomllib.load(f)


def trim_slash(s: str) -> str:
    return s.rstrip("/")


def read_configs():
    cfg = load_toml(CONFIG_TOML)
    audio = load_toml(AUDIO_TOML)
    image = load_toml(IMAGE_TOML)
    qwen = cfg["llm"]["qwen"]
    return cfg, audio, image, qwen


def guess_mime(path: Path) -> str:
    mime, _ = mimetypes.guess_type(str(path))
    return mime or "application/octet-stream"


def image_to_data_url(path: Path) -> str:
    mime = guess_mime(path)
    data = base64.b64encode(path.read_bytes()).decode("ascii")
    return f"data:{mime};base64,{data}"


def image_ref(value: str) -> str:
    if value.startswith("http://") or value.startswith("https://") or value.startswith("data:"):
        return value
    return image_to_data_url(Path(value))


def ensure_minimal_image(path: Path) -> Path:
    if path.exists():
        return path
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(base64.b64decode(MINIMAL_BLACK_PNG_BASE64))
    return path


def ensure_minimal_audio(path: Path) -> Path:
    if path.exists():
        return path
    path.parent.mkdir(parents=True, exist_ok=True)
    sample_rate = 16000
    seconds = 1
    total_frames = sample_rate * seconds
    with wave.open(str(path), "wb") as wav:
        wav.setnchannels(1)
        wav.setsampwidth(2)
        wav.setframerate(sample_rate)
        silence = struct.pack("<h", 0) * total_frames
        wav.writeframes(silence)
    return path


def print_ok(name: str, detail: str):
    print(f"[PASS] {name}: {detail}")


def print_fail(name: str, detail: str):
    print(f"[FAIL] {name}: {detail}")


def excerpt(raw: str, limit: int = 500) -> str:
    text = raw.strip().replace("\n", " ")
    return text[:limit]


def extract_image_url(v: dict):
    output = v.get("output") or {}
    results = output.get("results")
    if isinstance(results, list) and results:
        first = results[0]
        if isinstance(first, dict) and first.get("url"):
            return first["url"]

    choices = output.get("choices")
    if isinstance(choices, list):
        for choice in choices:
            msg = (choice or {}).get("message") or {}
            content = msg.get("content")
            if isinstance(content, list):
                for item in content:
                    if isinstance(item, dict):
                        if item.get("image"):
                            return item["image"]
                        if item.get("url"):
                            return item["url"]
    return None


def poll_task(api_key: str, base_api: str, task_id: str, timeout_sec: int = 180):
    deadline = time.time() + timeout_sec
    url = f"{trim_slash(base_api)}/tasks/{task_id}"
    headers = {"Authorization": f"Bearer {api_key}"}
    last_json = None

    while time.time() < deadline:
        r = requests.get(url, headers=headers, timeout=60)
        raw = r.text
        try:
            v = r.json()
        except Exception as err:
            raise RuntimeError(f"poll non-json status={r.status_code}: {excerpt(raw)}") from err
        last_json = v
        if r.status_code >= 300:
            raise RuntimeError(
                f"poll status={r.status_code}: {excerpt(json.dumps(v, ensure_ascii=False))}"
            )
        status = ((v.get("output") or {}).get("task_status") or "").upper()
        if status == "SUCCEEDED":
            return v
        if status in {"FAILED", "CANCELED", "CANCELLED"}:
            raise RuntimeError(f"task failed: {excerpt(json.dumps(v, ensure_ascii=False))}")
        time.sleep(2)

    raise RuntimeError(f"poll timeout; last={excerpt(json.dumps(last_json, ensure_ascii=False))}")


def test_asr_compat(qwen: dict, audio_cfg: dict, audio_path: Path):
    name = "ASR qwen3-asr-flash compat"
    url = f"{trim_slash(qwen['base_url'])}/audio/transcriptions"
    model = audio_cfg["audio_transcribe"]["default_model"]
    with audio_path.open("rb") as f:
        files = {"file": (audio_path.name, f, guess_mime(audio_path))}
        data = {
            "model": model,
            "prompt": "请识别这段音频，输出文本。",
        }
        r = requests.post(
            url,
            headers={"Authorization": f"Bearer {qwen['api_key']}"},
            data=data,
            files=files,
            timeout=120,
        )
    raw = r.text
    if r.status_code >= 300:
        raise RuntimeError(f"status={r.status_code}: {excerpt(raw)}")
    try:
        v = r.json()
    except Exception as err:
        raise RuntimeError(f"non-json body: {excerpt(raw)}") from err
    text = v.get("text") or v.get("transcript") or json.dumps(v, ensure_ascii=False)
    print_ok(name, excerpt(text, 160))


def test_tts_native(qwen: dict, audio_cfg: dict):
    name = "TTS qwen3-tts-flash native"
    base_api = audio_cfg["audio_synthesize"]["qwen_native_base_url"]
    url = f"{trim_slash(base_api)}/services/aigc/multimodal-generation/generation"
    model = audio_cfg["audio_synthesize"]["default_model"]
    body = {
        "model": model,
        "input": {
            "text": "你好，这是一条 RustClaw 五通道连通性测试语音。",
            "voice": audio_cfg["audio_synthesize"].get("default_voice", "Cherry"),
        },
    }
    r = requests.post(
        url,
        headers={"Authorization": f"Bearer {qwen['api_key']}"},
        json=body,
        timeout=120,
    )
    raw = r.text
    if r.status_code >= 300:
        raise RuntimeError(f"status={r.status_code}: {excerpt(raw)}")
    try:
        v = r.json()
    except Exception as err:
        raise RuntimeError(f"non-json body: {excerpt(raw)}") from err
    audio_url = (((v.get("output") or {}).get("audio") or {}).get("url"))
    if not audio_url:
        raise RuntimeError(f"missing output.audio.url: {excerpt(json.dumps(v, ensure_ascii=False))}")
    print_ok(name, audio_url)


def test_image_generate_native(qwen: dict, image_cfg: dict):
    name = "Image Generate wan2.6-image native"
    base_api = image_cfg["image_generation"]["qwen_native_base_url"]
    url = f"{trim_slash(base_api)}/services/aigc/image-generation/generation"
    model = image_cfg["image_generation"]["default_model"]
    body = {
        "model": model,
        "input": {
            "messages": [{
                "role": "user",
                "content": [{"text": "生成一只黑色龙虾，极简背景，写实风格"}],
            }]
        },
        "parameters": {
            "size": "1K",
            "n": 1,
            "prompt_extend": True,
            "watermark": False,
            "enable_interleave": True,
        },
    }
    r = requests.post(
        url,
        headers={
            "Authorization": f"Bearer {qwen['api_key']}",
            "X-DashScope-Async": "enable",
        },
        json=body,
        timeout=120,
    )
    raw = r.text
    if r.status_code >= 300:
        raise RuntimeError(f"create status={r.status_code}: {excerpt(raw)}")
    try:
        v = r.json()
    except Exception as err:
        raise RuntimeError(f"non-json create body: {excerpt(raw)}") from err
    image_url = extract_image_url(v)
    if not image_url:
        task_id = ((v.get("output") or {}).get("task_id"))
        if not task_id:
            raise RuntimeError(f"missing task_id/image: {excerpt(json.dumps(v, ensure_ascii=False))}")
        v = poll_task(qwen["api_key"], base_api, task_id)
        image_url = extract_image_url(v)
    if not image_url:
        raise RuntimeError(f"missing image url: {excerpt(json.dumps(v, ensure_ascii=False))}")
    print_ok(name, image_url)


def test_image_vision_compat(qwen: dict, image_cfg: dict, image_value: str):
    name = "Image Vision qwen-vl-max compat"
    url = f"{trim_slash(qwen['base_url'])}/chat/completions"
    model = image_cfg["image_vision"]["default_model"]
    body = {
        "model": model,
        "messages": [{
            "role": "user",
            "content": [
                {"type": "text", "text": "请用一句话描述这张图片。"},
                {"type": "image_url", "image_url": {"url": image_ref(image_value)}},
            ],
        }],
        "temperature": 0.2,
    }
    r = requests.post(
        url,
        headers={"Authorization": f"Bearer {qwen['api_key']}"},
        json=body,
        timeout=120,
    )
    raw = r.text
    if r.status_code >= 300:
        raise RuntimeError(f"status={r.status_code}: {excerpt(raw)}")
    try:
        v = r.json()
    except Exception as err:
        raise RuntimeError(f"non-json body: {excerpt(raw)}") from err
    text = (((v.get("choices") or [{}])[0].get("message") or {}).get("content"))
    if not text:
        raise RuntimeError(f"missing text: {excerpt(json.dumps(v, ensure_ascii=False))}")
    print_ok(name, excerpt(str(text), 160))


def test_image_edit_native(qwen: dict, image_cfg: dict, image_value: str):
    name = "Image Edit wan2.6-image native"
    base_api = image_cfg["image_edit"]["qwen_native_base_url"]
    url = f"{trim_slash(base_api)}/services/aigc/image-generation/generation"
    model = image_cfg["image_edit"]["default_model"]
    body = {
        "model": model,
        "input": {
            "messages": [{
                "role": "user",
                "content": [
                    {"text": "把这张图改成黑色风格，主体保持不变"},
                    {"image": image_ref(image_value)},
                ],
            }]
        },
        "parameters": {
            "size": "1K",
            "n": 1,
            "prompt_extend": True,
            "watermark": False,
            "enable_interleave": False,
        },
    }
    r = requests.post(
        url,
        headers={
            "Authorization": f"Bearer {qwen['api_key']}",
            "X-DashScope-Async": "enable",
        },
        json=body,
        timeout=120,
    )
    raw = r.text
    if r.status_code >= 300:
        raise RuntimeError(f"create status={r.status_code}: {excerpt(raw)}")
    try:
        v = r.json()
    except Exception as err:
        raise RuntimeError(f"non-json create body: {excerpt(raw)}") from err
    image_url = extract_image_url(v)
    if not image_url:
        task_id = ((v.get("output") or {}).get("task_id"))
        if not task_id:
            raise RuntimeError(f"missing task_id/image: {excerpt(json.dumps(v, ensure_ascii=False))}")
        v = poll_task(qwen["api_key"], base_api, task_id)
        image_url = extract_image_url(v)
    if not image_url:
        raise RuntimeError(f"missing image url: {excerpt(json.dumps(v, ensure_ascii=False))}")
    print_ok(name, image_url)


def main():
    ap = argparse.ArgumentParser(description="Test 5 Qwen channels used by RustClaw.")
    ap.add_argument("--audio", help="Local audio file for ASR test")
    ap.add_argument("--image", help="Local image file for vision/edit test")
    ap.add_argument("--image-url", help="Public image URL for vision/edit test")
    args = ap.parse_args()

    default_audio = ROOT / "audio" / "test_qwen_5_channels.wav"
    audio_path = Path(args.audio).expanduser().resolve() if args.audio else ensure_minimal_audio(default_audio).resolve()
    if args.image_url:
        image_value = args.image_url.strip()
    elif args.image:
        image_value = str(Path(args.image).expanduser().resolve())
    else:
        default_image = ROOT / "image" / "test_qwen_5_channels.png"
        image_value = str(ensure_minimal_image(default_image).resolve())
    if not audio_path.is_file():
        print(f"audio file not found: {audio_path}", file=sys.stderr)
        sys.exit(2)
    if not (
        image_value.startswith("http://")
        or image_value.startswith("https://")
        or Path(image_value).is_file()
    ):
        print(f"image input not found or invalid: {image_value}", file=sys.stderr)
        sys.exit(2)

    _, audio_cfg, image_cfg, qwen = read_configs()

    tests = [
        ("ASR qwen3-asr-flash compat", lambda: test_asr_compat(qwen, audio_cfg, audio_path)),
        ("TTS qwen3-tts-flash native", lambda: test_tts_native(qwen, audio_cfg)),
        ("Image Generate wan2.6-image native", lambda: test_image_generate_native(qwen, image_cfg)),
        ("Image Vision qwen-vl-max compat", lambda: test_image_vision_compat(qwen, image_cfg, image_value)),
        ("Image Edit wan2.6-image native", lambda: test_image_edit_native(qwen, image_cfg, image_value)),
    ]

    print(f"audio sample: {audio_path}")
    print(f"image sample: {image_value}")
    print()

    failed = 0
    for name, fn in tests:
        try:
            fn()
        except Exception as err:
            failed += 1
            print_fail(name, str(err))

    print()
    print(f"done: total=5 failed={failed} passed={5 - failed}")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
