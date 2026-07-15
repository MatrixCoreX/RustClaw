#!/usr/bin/env python3
"""Validate Chinese-provider model/catalog metadata.

This is a static repository guard. It checks machine-readable config metadata
for MiniMax, MiMo, Qwen, and DeepSeek so model capability drift is caught before
runtime behavior depends on stale comments or ad-hoc natural-language fixes.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any

try:
    import tomllib
except ModuleNotFoundError:  # pragma: no cover
    import tomli as tomllib  # type: ignore


ROOT = Path(__file__).resolve().parents[1]

MAIN_CONFIG = ROOT / "configs/config.toml"
DOCKER_CONFIG = ROOT / "docker/config/config.toml"
IMAGE_CONFIG = ROOT / "configs/image.toml"
AUDIO_CONFIG = ROOT / "configs/audio.toml"
VIDEO_CONFIG = ROOT / "configs/video.toml"
MUSIC_CONFIG = ROOT / "configs/music.toml"
CHINESE_CASE_FILE = ROOT / "scripts/nl_tests/cases/nl_cases_chinese_model_adapter_20260715.txt"
VENDOR_PATCH_ROOT = ROOT / "prompts/layers/vendor_patches"

TEXT_PROVIDER_FIELDS = [
    "base_url",
    "model",
    "models",
    "context_window_tokens",
    "timeout_seconds",
]

CHINESE_TEXT_PROVIDERS = {
    "deepseek": {
        "base_url": "https://api.deepseek.com/v1",
        "model": "deepseek-chat",
        "models": {"deepseek-chat", "deepseek-reasoner"},
        "timeout_min": 60,
    },
    "qwen": {
        "base_url": "https://dashscope.aliyuncs.com/compatible-mode/v1",
        "model": "qwen-max-latest",
        "models": {"qwen-max-latest", "qwen-plus-latest", "qwen-turbo-latest"},
        "timeout_min": 60,
    },
    "minimax": {
        "base_url": "https://api.minimaxi.com/v1",
        "model": "MiniMax-M3",
        "models": {"MiniMax-M3", "MiniMax-M2.7"},
        "timeout_min": 180,
        "context_window_min": 1_000_000,
    },
    "mimo": {
        "model": "mimo-v2.5-pro",
        "models": {"mimo-v2.5-pro", "mimo-v2.5"},
        "timeout_min": 180,
    },
}

REQUIRED_PATCHES = {
    "minimax": [
        "routing/common.md",
        "execution/common.md",
        "recovery/common.md",
        "text/common.md",
        "skills/common.md",
    ],
    "mimo": [
        "routing/common.md",
        "execution/common.md",
        "recovery/common.md",
        "text/common.md",
        "skills/common.md",
    ],
    "qwen": ["skills/common.md"],
    "deepseek": ["skills/common.md"],
}

REQUIRED_CHINESE_CASE_TAGS = {
    "chinese_provider",
    "deepseek",
    "large_context",
    "minimax",
    "mimo",
    "multimodal_understanding",
    "openai_compatible",
    "qwen",
    "strict_json",
    "vendor_patch",
}


def load_toml(path: Path) -> dict[str, Any]:
    return tomllib.loads(path.read_text(encoding="utf-8"))


def as_list(value: Any) -> list[str]:
    if isinstance(value, list):
        return [str(item) for item in value]
    return []


def fail(findings: list[str], message: str) -> None:
    findings.append(message)


def require(condition: bool, findings: list[str], message: str) -> None:
    if not condition:
        fail(findings, message)


def base_url_kind(base_url: str) -> str:
    if "api.minimaxi.com" in base_url:
        return "minimax_official_openai_compat"
    if "xiaomimimo.com" in base_url:
        return "mimo_token_plan_openai_compat"
    if "dashscope.aliyuncs.com/compatible-mode" in base_url:
        return "qwen_dashscope_openai_compat"
    if "api.deepseek.com" in base_url:
        return "deepseek_official_openai_compat"
    if "dashscope.aliyuncs.com/api/v1" in base_url:
        return "qwen_dashscope_native"
    return "custom_or_unknown"


def provider_models(section: dict[str, Any], provider: str) -> set[str]:
    return set(as_list(section.get(f"{provider}_models")))


def catalog_entry(
    provider: str,
    llm_table: dict[str, Any],
    image: dict[str, Any],
    audio: dict[str, Any],
    video: dict[str, Any],
    music: dict[str, Any],
) -> dict[str, Any]:
    image_edit = image.get("image_edit", {}) if isinstance(image.get("image_edit"), dict) else {}
    image_generation = (
        image.get("image_generation", {}) if isinstance(image.get("image_generation"), dict) else {}
    )
    image_vision = image.get("image_vision", {}) if isinstance(image.get("image_vision"), dict) else {}
    tts = audio.get("audio_synthesize", {}) if isinstance(audio.get("audio_synthesize"), dict) else {}
    stt = audio.get("audio_transcribe", {}) if isinstance(audio.get("audio_transcribe"), dict) else {}
    video_gen = video.get("video_generation", {}) if isinstance(video.get("video_generation"), dict) else {}
    music_gen = music.get("music_generation", {}) if isinstance(music.get("music_generation"), dict) else {}

    model = str(llm_table.get("model") or "")
    image_understanding_models = provider_models(image_vision, provider)
    audio_transcription_models = provider_models(stt, provider)
    supports_image_input = model in image_understanding_models
    supports_video_input = provider == "minimax" and model == "MiniMax-M3"
    supports_audio_input = model in audio_transcription_models
    supports_image_understanding = bool(image_understanding_models)
    supports_audio_transcription = bool(audio_transcription_models)
    supports_image_generation = bool(
        provider_models(image_generation, provider) or provider_models(image_edit, provider)
    )
    supports_audio_generation = bool(provider_models(tts, provider))
    supports_video_generation = bool(provider_models(video_gen, provider))
    supports_music_generation = bool(provider_models(music_gen, provider))
    media_support = any(
        [
            supports_image_generation,
            supports_audio_generation,
            supports_video_generation,
            supports_music_generation,
        ]
    )

    return {
        "schema_version": 1,
        "provider": provider,
        "model": model,
        "models": as_list(llm_table.get("models")),
        "api_style": "openai_compatible",
        "base_url_kind": base_url_kind(str(llm_table.get("base_url") or "")),
        "context_window_tokens": llm_table.get("context_window_tokens"),
        "timeout_seconds": llm_table.get("timeout_seconds"),
        "supports_text": True,
        "supports_image_input": supports_image_input,
        "supports_video_input": supports_video_input,
        "supports_audio_input": supports_audio_input,
        "supports_image_understanding": supports_image_understanding,
        "supports_audio_transcription": supports_audio_transcription,
        "supports_image_generation": supports_image_generation,
        "supports_audio_generation": supports_audio_generation,
        "supports_video_generation": supports_video_generation,
        "supports_music_generation": supports_music_generation,
        "async_required": media_support,
        "dry_run_supported": media_support,
        "config_source": [
            "configs/config.toml",
            "configs/image.toml",
            "configs/audio.toml",
            "configs/video.toml",
            "configs/music.toml",
            f"prompts/layers/vendor_patches/{provider}",
        ],
    }


def build_catalog(main: dict[str, Any]) -> list[dict[str, Any]]:
    llm = main.get("llm") if isinstance(main.get("llm"), dict) else {}
    image = load_toml(IMAGE_CONFIG)
    audio = load_toml(AUDIO_CONFIG)
    video = load_toml(VIDEO_CONFIG)
    music = load_toml(MUSIC_CONFIG)
    catalog: list[dict[str, Any]] = []
    for provider in sorted(CHINESE_TEXT_PROVIDERS):
        table = llm.get(provider) if isinstance(llm, dict) else None
        if not isinstance(table, dict):
            continue
        catalog.append(catalog_entry(provider, table, image, audio, video, music))
    return catalog


def check_text_provider_config(
    findings: list[str],
    label: str,
    config: dict[str, Any],
) -> None:
    llm = config.get("llm")
    require(isinstance(llm, dict), findings, f"{label}: missing [llm]")
    if not isinstance(llm, dict):
        return

    require(
        llm.get("selected_vendor") == "minimax",
        findings,
        f"{label}: [llm].selected_vendor must stay minimax for Chinese-provider default",
    )
    require(
        llm.get("selected_model") == "MiniMax-M3",
        findings,
        f"{label}: [llm].selected_model must stay MiniMax-M3 for 1M multimodal default",
    )

    for provider, expected in CHINESE_TEXT_PROVIDERS.items():
        table = llm.get(provider)
        require(isinstance(table, dict), findings, f"{label}: missing [llm.{provider}]")
        if not isinstance(table, dict):
            continue
        if "base_url" in expected:
            require(
                table.get("base_url") == expected["base_url"],
                findings,
                f"{label}: [llm.{provider}].base_url expected {expected['base_url']!r}",
            )
        require(
            table.get("model") == expected["model"],
            findings,
            f"{label}: [llm.{provider}].model expected {expected['model']!r}",
        )
        models = set(as_list(table.get("models")))
        require(
            str(table.get("model") or "") in models,
            findings,
            f"{label}: [llm.{provider}].model must be present in models",
        )
        missing_models = sorted(expected["models"] - models)
        require(
            not missing_models,
            findings,
            f"{label}: [llm.{provider}].models missing {missing_models}",
        )
        timeout = table.get("timeout_seconds")
        require(
            isinstance(timeout, int) and timeout >= expected["timeout_min"],
            findings,
            f"{label}: [llm.{provider}].timeout_seconds must be >= {expected['timeout_min']}",
        )
        if "context_window_min" in expected:
            context_window = table.get("context_window_tokens")
            require(
                isinstance(context_window, int)
                and context_window >= expected["context_window_min"],
                findings,
                f"{label}: [llm.{provider}].context_window_tokens must be >= {expected['context_window_min']}",
            )


def check_main_docker_text_parity(findings: list[str], main: dict[str, Any], docker: dict[str, Any]) -> None:
    main_llm = main.get("llm") if isinstance(main.get("llm"), dict) else {}
    docker_llm = docker.get("llm") if isinstance(docker.get("llm"), dict) else {}
    for provider in CHINESE_TEXT_PROVIDERS:
        main_table = main_llm.get(provider) if isinstance(main_llm, dict) else None
        docker_table = docker_llm.get(provider) if isinstance(docker_llm, dict) else None
        if not isinstance(main_table, dict) or not isinstance(docker_table, dict):
            continue
        for field in TEXT_PROVIDER_FIELDS:
            if field not in main_table and field not in docker_table:
                continue
            require(
                main_table.get(field) == docker_table.get(field),
                findings,
                f"docker parity: [llm.{provider}].{field} differs from configs/config.toml",
            )


def check_media_config(findings: list[str]) -> None:
    image = load_toml(IMAGE_CONFIG)
    audio = load_toml(AUDIO_CONFIG)
    video = load_toml(VIDEO_CONFIG)
    music = load_toml(MUSIC_CONFIG)

    image_edit = image.get("image_edit", {})
    image_generation = image.get("image_generation", {})
    image_vision = image.get("image_vision", {})
    require(image_edit.get("default_model") == "image-01", findings, "image_edit.default_model must be image-01")
    require(
        "MiniMax-M3" not in as_list(image_edit.get("minimax_models")),
        findings,
        "image_edit.minimax_models must not treat MiniMax-M3 as a generation/edit model",
    )
    require(
        image_generation.get("default_model") == "image-01",
        findings,
        "image_generation.default_model must be image-01",
    )
    require(
        "MiniMax-M3" not in as_list(image_generation.get("minimax_models")),
        findings,
        "image_generation.minimax_models must not treat MiniMax-M3 as a generation/edit model",
    )
    require(
        image_vision.get("default_model") == "MiniMax-M3",
        findings,
        "image_vision.default_model must be MiniMax-M3",
    )
    require(
        "MiniMax-M3" in as_list(image_vision.get("minimax_models")),
        findings,
        "image_vision.minimax_models must include MiniMax-M3",
    )
    require(
        bool(set(as_list(image_vision.get("mimo_models"))) & {"mimo-v2.5", "mimo-v2-omni"}),
        findings,
        "image_vision.mimo_models must include a MiMo multimodal model",
    )

    tts = audio.get("audio_synthesize", {})
    stt = audio.get("audio_transcribe", {})
    require(tts.get("default_vendor") == "minimax", findings, "audio_synthesize.default_vendor must be minimax")
    require(
        "speech-2.8-turbo" in as_list(tts.get("minimax_models")),
        findings,
        "audio_synthesize.minimax_models must include speech-2.8-turbo",
    )
    require(
        "mimo-v2.5-tts" in as_list(tts.get("mimo_models")),
        findings,
        "audio_synthesize.mimo_models must include mimo-v2.5-tts",
    )
    require(
        "qwen3-tts-flash" in as_list(tts.get("qwen_models")),
        findings,
        "audio_synthesize.qwen_models must include qwen3-tts-flash",
    )
    require(stt.get("default_model") == "local-whisper", findings, "audio_transcribe.default_model must be local-whisper")
    require(
        "qwen3-asr-flash" in as_list(stt.get("qwen_models")),
        findings,
        "audio_transcribe.qwen_models must include qwen3-asr-flash",
    )

    for section_name, cfg, expected_model in [
        ("video_generation", video.get("video_generation", {}), "MiniMax-Hailuo-2.3"),
        ("music_generation", music.get("music_generation", {}), "music-2.6"),
    ]:
        require(cfg.get("default_vendor") == "minimax", findings, f"{section_name}.default_vendor must be minimax")
        require(cfg.get("default_model") == expected_model, findings, f"{section_name}.default_model must be {expected_model}")
        require(
            expected_model in as_list(cfg.get("minimax_models")),
            findings,
            f"{section_name}.minimax_models must include {expected_model}",
        )
        providers = cfg.get("providers") if isinstance(cfg.get("providers"), dict) else {}
        minimax_provider = providers.get("minimax") if isinstance(providers, dict) else None
        require(
            isinstance(minimax_provider, dict)
            and minimax_provider.get("base_url") == "https://api.minimaxi.com/v1",
            findings,
            f"{section_name}.providers.minimax.base_url must be https://api.minimaxi.com/v1",
        )


def check_vendor_patches(findings: list[str]) -> None:
    for vendor, rel_paths in REQUIRED_PATCHES.items():
        for rel_path in rel_paths:
            path = VENDOR_PATCH_ROOT / vendor / rel_path
            require(path.exists(), findings, f"missing vendor patch: {path.relative_to(ROOT)}")


def check_chinese_case_gate(findings: list[str]) -> None:
    require(CHINESE_CASE_FILE.exists(), findings, f"missing {CHINESE_CASE_FILE.relative_to(ROOT)}")
    if not CHINESE_CASE_FILE.exists():
        return
    tags: set[str] = set()
    for raw in CHINESE_CASE_FILE.read_text(encoding="utf-8").splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        parts = line.split("|", 3)
        if len(parts) < 3:
            continue
        for token in parts[2].split(";"):
            token = token.strip()
            if token.startswith("covers:"):
                tags.update(item.strip() for item in token[len("covers:") :].split(",") if item.strip())
            elif token:
                tags.add(token)
    missing = sorted(REQUIRED_CHINESE_CASE_TAGS - tags)
    require(not missing, findings, f"Chinese model adapter cases missing tags: {missing}")


def build_report() -> dict[str, Any]:
    findings: list[str] = []
    main = load_toml(MAIN_CONFIG)
    docker = load_toml(DOCKER_CONFIG)
    check_text_provider_config(findings, "configs/config.toml", main)
    check_text_provider_config(findings, "docker/config/config.toml", docker)
    check_main_docker_text_parity(findings, main, docker)
    check_media_config(findings)
    check_vendor_patches(findings)
    check_chinese_case_gate(findings)
    catalog = build_catalog(main)
    return {
        "schema_version": 1,
        "status": "ok" if not findings else "error",
        "finding_count": len(findings),
        "findings": findings,
        "catalog": catalog,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--json", action="store_true", help="print machine-readable JSON report")
    parser.add_argument("--catalog-only", action="store_true", help="print only the model catalog JSON array")
    args = parser.parse_args()

    report = build_report()
    if args.catalog_only:
        print(json.dumps(report["catalog"], ensure_ascii=False, indent=2, sort_keys=True))
    elif args.json:
        print(json.dumps(report, ensure_ascii=False, indent=2, sort_keys=True))
    elif report["status"] == "ok":
        print("CHINESE_MODEL_CATALOG_CHECK ok")
    else:
        print("CHINESE_MODEL_CATALOG_CHECK failed", file=sys.stderr)
        for finding in report["findings"]:
            print(f"- {finding}", file=sys.stderr)
    return 0 if report["status"] == "ok" else 1


if __name__ == "__main__":
    raise SystemExit(main())
