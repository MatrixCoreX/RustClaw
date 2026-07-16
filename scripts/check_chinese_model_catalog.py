#!/usr/bin/env python3
"""Validate Chinese-provider model/catalog metadata.

This is a static repository guard. It checks machine-readable config metadata
for MiniMax, MiMo, Qwen, and DeepSeek so model capability drift is caught before
runtime behavior depends on stale comments or ad-hoc natural-language fixes.
"""

from __future__ import annotations

import argparse
import json
import os
import shlex
import sys
import tempfile
from pathlib import Path
from typing import Any

try:
    import tomllib
except ModuleNotFoundError:  # pragma: no cover
    import tomli as tomllib  # type: ignore


ROOT = Path(__file__).resolve().parents[1]
SCRIPTS_DIR = ROOT / "scripts"
if str(SCRIPTS_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPTS_DIR))

from nl_tests.secret_scan import secret_scan_findings

from chinese_model_catalog_gate_checks import check_chinese_provider_smoke_live_scope


MAIN_CONFIG = ROOT / "configs/config.toml"
DOCKER_CONFIG = ROOT / "docker/config/config.toml"
IMAGE_CONFIG = ROOT / "configs/image.toml"
AUDIO_CONFIG = ROOT / "configs/audio.toml"
VIDEO_CONFIG = ROOT / "configs/video.toml"
MUSIC_CONFIG = ROOT / "configs/music.toml"
CHINESE_CASE_FILE = ROOT / "scripts/nl_tests/cases/nl_cases_chinese_model_adapter_20260715.txt"
VENDOR_PATCH_ROOT = ROOT / "prompts/layers/vendor_patches"
TASK_DEBUG_TRACE_SOURCE = ROOT / "crates/clawd/src/http/ui_routes/task_debug_trace.rs"
UI_TASK_LLM_TRACE_SOURCE = ROOT / "UI/src/lib/task-llm-trace.ts"

TEXT_PROVIDER_FIELDS = [
    "base_url",
    "model",
    "models",
    "input_modalities",
    "output_modalities",
    "context_window_tokens",
    "timeout_seconds",
]

MIMO_PRIMARY_OPENAI_BASE_URL = "https://token-plan-cn.xiaomimimo.com/v1"

CHINESE_TEXT_PROVIDERS = {
    "deepseek": {
        "base_url": "https://api.deepseek.com/v1",
        "required_models": {"deepseek-chat", "deepseek-reasoner"},
        "required_input_modalities": {"text"},
        "required_output_modalities": {"text"},
        "timeout_min": 60,
    },
    "qwen": {
        "base_url": "https://dashscope.aliyuncs.com/compatible-mode/v1",
        "required_models": {"qwen-max-latest", "qwen-plus-latest", "qwen-turbo-latest"},
        "required_input_modalities": {"text"},
        "required_output_modalities": {"text"},
        "timeout_min": 60,
    },
    "minimax": {
        "base_url": "https://api.minimaxi.com/v1",
        "required_models": {"MiniMax-M3", "MiniMax-M2.7"},
        "required_input_modalities": {"text", "image", "video"},
        "required_output_modalities": {"text"},
        "timeout_min": 180,
        "context_window_min": 1_000_000,
    },
    "mimo": {
        "base_url": MIMO_PRIMARY_OPENAI_BASE_URL,
        "required_models": {"mimo-v2.5-pro", "mimo-v2.5"},
        "required_input_modalities": {"text"},
        "required_output_modalities": {"text"},
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

RUNTIME_CATALOG_ENTRY_FIELDS = {
    "active_text_provider",
    "api_style",
    "async_required",
    "base_url_kind",
    "config_source",
    "context_window_tokens",
    "credential_state",
    "dry_run_supported",
    "input_modalities",
    "model",
    "models",
    "output_modalities",
    "provider",
    "schema_version",
    "supports_audio_generation",
    "supports_audio_input",
    "supports_audio_transcription",
    "supports_image_generation",
    "supports_image_input",
    "supports_image_understanding",
    "supports_music_generation",
    "supports_text",
    "supports_video_generation",
    "supports_video_input",
    "timeout_seconds",
}

TASK_DEBUG_TRACE_CATALOG_FIELD_EXCLUSIONS = {
    "config_source",
}

UI_TEACHING_CATALOG_FIELD_EXCLUSIONS = {
    "config_source",
}

PROVIDER_CREDENTIAL_ENV_VARS = {
    "deepseek": ["DEEPSEEK_API_KEY"],
    "minimax": ["MINIMAX_API_KEY"],
    "mimo": ["MIMO_API_KEY", "XIAOMI_API_KEY"],
    "qwen": ["QWEN_API_KEY", "DASHSCOPE_API_KEY"],
}

STALE_MINIMAX_ENDPOINT_TOKENS = ("api.minimax.io", "api.minimax.cn", "api.minimax.ocom")
STALE_MINIMAX_ENDPOINT_SCAN_ROOTS = (
    ROOT / "configs",
    ROOT / "docker/config",
    ROOT / "UI/src",
    ROOT / "crates/clawd/src/http",
    ROOT / "scripts",
)
STALE_MINIMAX_ENDPOINT_SCAN_SUFFIXES = (".toml", ".ts", ".tsx", ".rs", ".py", ".sh")

PRIMARY_MODEL_ENDPOINT_SOURCE_FILES = (
    ROOT / "crates/telegramd/src/commands.rs",
    ROOT / "crates/telegramd/src/main_model_config_tests.rs",
    ROOT / "crates/clawd/src/http/ui_routes_tests.rs",
    ROOT / "crates/claw-core/src/model_catalog_tests.rs",
    ROOT / "UI/src/lib/llm-config.test.ts",
)

STALE_MIMO_PRIMARY_OPENAI_ENDPOINT_TOKENS = (
    "https://token-plan-sgp.xiaomimimo.com/v1",
    "https://api.xiaomimimo.com/v1",
)


def relative_path_label(path: Path) -> str:
    try:
        return str(path.relative_to(ROOT))
    except ValueError:
        return str(path)


def load_toml(path: Path, findings: list[str]) -> dict[str, Any]:
    label = relative_path_label(path)
    try:
        text = path.read_text(encoding="utf-8")
    except FileNotFoundError:
        fail(findings, f"toml_missing:{label}")
        return {}
    except OSError as exc:
        fail(findings, f"toml_read_failed:{label}:{exc.__class__.__name__}")
        return {}
    except UnicodeDecodeError:
        fail(findings, f"toml_decode_failed:{label}")
        return {}
    try:
        return tomllib.loads(text)
    except tomllib.TOMLDecodeError:
        fail(findings, f"toml_parse_failed:{label}")
        return {}


def as_list(value: Any) -> list[str]:
    if isinstance(value, list):
        return [str(item) for item in value]
    return []


def load_env_file(path: Path | None, findings: list[str]) -> dict[str, str]:
    if path is None:
        return {}
    if not path.exists():
        fail(findings, "env_file_missing")
        return {}
    values: dict[str, str] = {}
    try:
        raw_lines = path.read_text(encoding="utf-8").splitlines()
    except OSError as exc:
        fail(findings, f"env_file_read_failed:{exc.__class__.__name__}")
        return {}
    except UnicodeDecodeError:
        fail(findings, "env_file_decode_failed")
        return {}
    for raw in raw_lines:
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        if line.startswith("export "):
            line = line[len("export ") :].strip()
        if "=" not in line:
            continue
        key, raw_value = line.split("=", 1)
        key = key.strip()
        if not key:
            continue
        try:
            parsed = shlex.split(raw_value, posix=True)
            value = parsed[0] if parsed else ""
        except ValueError:
            value = raw_value.strip().strip('"').strip("'")
        values[key] = value
    return values


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


def api_style_token(raw: Any) -> str:
    value = str(raw or "").strip()
    if value in {"", "openai_compat", "openai_compatible"}:
        return "openai_compatible"
    if value in {"anthropic_claude", "anthropic_messages"}:
        return "anthropic_messages"
    if value in {"google_gemini", "gemini"}:
        return "google_gemini"
    return "custom_or_unknown"


def provider_models(section: dict[str, Any], provider: str) -> set[str]:
    return set(as_list(section.get(f"{provider}_models")))


def modality_list(llm_table: dict[str, Any], key: str, fallback: list[str]) -> list[str]:
    values = [value.strip().lower() for value in as_list(llm_table.get(key)) if value.strip()]
    if not values:
        return fallback
    out: list[str] = []
    for value in values:
        if value not in out:
            out.append(value)
    return out


def modality_set(llm_table: dict[str, Any], key: str) -> set[str]:
    return {value.strip().lower() for value in as_list(llm_table.get(key)) if value.strip()}


def credential_state(llm_table: dict[str, Any], provider: str, env_values: dict[str, str]) -> str:
    if str(llm_table.get("api_key") or "").strip():
        return "configured_inline"
    for env_name in PROVIDER_CREDENTIAL_ENV_VARS.get(provider, []):
        if str(os.environ.get(env_name) or "").strip():
            return "configured_env"
        if str(env_values.get(env_name) or "").strip():
            return "configured_env"
    return "missing"


def catalog_entry(
    provider: str,
    llm_table: dict[str, Any],
    image: dict[str, Any],
    audio: dict[str, Any],
    video: dict[str, Any],
    music: dict[str, Any],
    selected_provider: str,
    selected_model: str,
    env_values: dict[str, str],
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
    fallback_inputs = ["text"]
    if model in image_understanding_models:
        fallback_inputs.append("image")
    if model in audio_transcription_models:
        fallback_inputs.append("audio")
    input_modalities = modality_list(llm_table, "input_modalities", fallback_inputs)
    output_modalities = modality_list(llm_table, "output_modalities", ["text"])
    supports_image_input = "image" in input_modalities
    supports_video_input = "video" in input_modalities
    supports_audio_input = "audio" in input_modalities
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
        "api_style": api_style_token(llm_table.get("api_format")),
        "base_url_kind": base_url_kind(str(llm_table.get("base_url") or "")),
        "context_window_tokens": llm_table.get("context_window_tokens"),
        "timeout_seconds": llm_table.get("timeout_seconds"),
        "credential_state": credential_state(llm_table, provider, env_values),
        "input_modalities": input_modalities,
        "output_modalities": output_modalities,
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
        "active_text_provider": provider == selected_provider and model == selected_model,
        "config_source": [
            "configs/config.toml",
            "configs/image.toml",
            "configs/audio.toml",
            "configs/video.toml",
            "configs/music.toml",
            f"prompts/layers/vendor_patches/{provider}",
        ],
    }


def build_catalog(
    main: dict[str, Any],
    env_values: dict[str, str],
    findings: list[str],
) -> list[dict[str, Any]]:
    llm = main.get("llm") if isinstance(main.get("llm"), dict) else {}
    selected_provider = str(llm.get("selected_vendor") or "") if isinstance(llm, dict) else ""
    selected_model = str(llm.get("selected_model") or "") if isinstance(llm, dict) else ""
    image = load_toml(IMAGE_CONFIG, findings)
    audio = load_toml(AUDIO_CONFIG, findings)
    video = load_toml(VIDEO_CONFIG, findings)
    music = load_toml(MUSIC_CONFIG, findings)
    catalog: list[dict[str, Any]] = []
    for provider in sorted(CHINESE_TEXT_PROVIDERS):
        table = llm.get(provider) if isinstance(llm, dict) else None
        if not isinstance(table, dict):
            continue
        catalog.append(
            catalog_entry(
                provider,
                table,
                image,
                audio,
                video,
                music,
                selected_provider,
                selected_model,
                env_values,
            )
        )
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
    selected_vendor = str(llm.get("selected_vendor") or "").strip()
    selected_model = str(llm.get("selected_model") or "").strip()
    selected_table = llm.get(selected_vendor)
    if isinstance(selected_table, dict):
        selected_provider_model = str(selected_table.get("model") or "").strip()
        require(
            selected_provider_model != "",
            findings,
            f"{label}: [llm.{selected_vendor}].model must be non-empty",
        )
        require(
            selected_model == selected_provider_model,
            findings,
            f"{label}: [llm].selected_model must match [llm.{selected_vendor}].model",
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
        model = str(table.get("model") or "").strip()
        require(
            model != "",
            findings,
            f"{label}: [llm.{provider}].model must be non-empty",
        )
        models = set(as_list(table.get("models")))
        require(
            model in models,
            findings,
            f"{label}: [llm.{provider}].model must be present in models",
        )
        missing_models = sorted(expected["required_models"] - models)
        require(
            not missing_models,
            findings,
            f"{label}: [llm.{provider}].models missing required fallbacks {missing_models}",
        )
        input_modalities = modality_set(table, "input_modalities")
        output_modalities = modality_set(table, "output_modalities")
        missing_input_modalities = sorted(expected["required_input_modalities"] - input_modalities)
        missing_output_modalities = sorted(expected["required_output_modalities"] - output_modalities)
        require(
            not missing_input_modalities,
            findings,
            f"{label}: [llm.{provider}].input_modalities missing required {missing_input_modalities}",
        )
        require(
            not missing_output_modalities,
            findings,
            f"{label}: [llm.{provider}].output_modalities missing required {missing_output_modalities}",
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


def check_primary_model_endpoint_source_text(findings: list[str], label: str, text: str) -> None:
    require(
        MIMO_PRIMARY_OPENAI_BASE_URL in text,
        findings,
        f"{label} must use MiMo primary endpoint {MIMO_PRIMARY_OPENAI_BASE_URL}",
    )
    for stale in STALE_MIMO_PRIMARY_OPENAI_ENDPOINT_TOKENS:
        require(
            stale not in text,
            findings,
            f"stale MiMo primary endpoint token {stale!r} in {label}",
        )


def check_primary_model_endpoint_source_alignment(findings: list[str]) -> None:
    for path in PRIMARY_MODEL_ENDPOINT_SOURCE_FILES:
        require(path.exists(), findings, f"missing primary model endpoint source: {path.relative_to(ROOT)}")
        if not path.exists():
            continue
        try:
            text = path.read_text(encoding="utf-8")
        except UnicodeDecodeError:
            fail(findings, f"primary_model_endpoint_source_decode_failed:{path.relative_to(ROOT)}")
            continue
        except OSError as exc:
            fail(
                findings,
                f"primary_model_endpoint_source_read_failed:{path.relative_to(ROOT)}:{exc.__class__.__name__}",
            )
            continue
        check_primary_model_endpoint_source_text(findings, str(path.relative_to(ROOT)), text)


def check_media_config(findings: list[str], main: dict[str, Any]) -> None:
    image = load_toml(IMAGE_CONFIG, findings)
    audio = load_toml(AUDIO_CONFIG, findings)
    video = load_toml(VIDEO_CONFIG, findings)
    music = load_toml(MUSIC_CONFIG, findings)
    main_llm = main.get("llm") if isinstance(main.get("llm"), dict) else {}
    minimax_table = main_llm.get("minimax") if isinstance(main_llm, dict) else {}
    active_minimax_text_model = (
        str(minimax_table.get("model") or "").strip() if isinstance(minimax_table, dict) else ""
    )

    image_edit = image.get("image_edit", {})
    image_generation = image.get("image_generation", {})
    image_vision = image.get("image_vision", {})
    require(image_edit.get("default_model") == "image-01", findings, "image_edit.default_model must be image-01")
    require(
        image_generation.get("default_model") == "image-01",
        findings,
        "image_generation.default_model must be image-01",
    )
    require(
        active_minimax_text_model != "",
        findings,
        "configs/config.toml: [llm.minimax].model must be non-empty before media-boundary checks",
    )
    if active_minimax_text_model:
        require(
            active_minimax_text_model not in as_list(image_edit.get("minimax_models")),
            findings,
            "image_edit.minimax_models must not treat active MiniMax text model as a generation/edit model",
        )
        require(
            active_minimax_text_model not in as_list(image_generation.get("minimax_models")),
            findings,
            "image_generation.minimax_models must not treat active MiniMax text model as a generation/edit model",
        )
        require(
            image_vision.get("default_model") == active_minimax_text_model,
            findings,
            "image_vision.default_model must match [llm.minimax].model",
        )
        require(
            active_minimax_text_model in as_list(image_vision.get("minimax_models")),
            findings,
            "image_vision.minimax_models must include the active MiniMax text model",
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


def check_runtime_catalog_shape(findings: list[str], catalog: list[dict[str, Any]]) -> None:
    for entry in catalog:
        provider = str(entry.get("provider") or "unknown")
        missing = sorted(RUNTIME_CATALOG_ENTRY_FIELDS - set(entry))
        require(not missing, findings, f"catalog entry {provider}: missing runtime fields {missing}")


def check_model_catalog_teaching_projection(findings: list[str]) -> None:
    require(
        TASK_DEBUG_TRACE_SOURCE.exists(),
        findings,
        f"missing {TASK_DEBUG_TRACE_SOURCE.relative_to(ROOT)}",
    )
    require(
        UI_TASK_LLM_TRACE_SOURCE.exists(),
        findings,
        f"missing {UI_TASK_LLM_TRACE_SOURCE.relative_to(ROOT)}",
    )
    if not TASK_DEBUG_TRACE_SOURCE.exists() or not UI_TASK_LLM_TRACE_SOURCE.exists():
        return

    task_debug_source = TASK_DEBUG_TRACE_SOURCE.read_text(encoding="utf-8")
    ui_trace_source = UI_TASK_LLM_TRACE_SOURCE.read_text(encoding="utf-8")
    task_debug_required = sorted(RUNTIME_CATALOG_ENTRY_FIELDS - TASK_DEBUG_TRACE_CATALOG_FIELD_EXCLUSIONS)
    missing_task_debug_projection = [
        field
        for field in task_debug_required
        if f'"{field}": entry.{field}' not in task_debug_source
    ]
    require(
        not missing_task_debug_projection,
        findings,
        f"task debug model_catalog_trace missing projected fields {missing_task_debug_projection}",
    )

    ui_required = sorted(RUNTIME_CATALOG_ENTRY_FIELDS - UI_TEACHING_CATALOG_FIELD_EXCLUSIONS)
    missing_ui_projection = [
        field
        for field in ui_required
        if f'"{field}"' not in ui_trace_source
    ]
    require(
        not missing_ui_projection,
        findings,
        f"UI model catalog teaching tokens missing fields {missing_ui_projection}",
    )


def check_no_stale_minimax_endpoints(findings: list[str]) -> None:
    for root in STALE_MINIMAX_ENDPOINT_SCAN_ROOTS:
        if root.is_file():
            paths = [root]
        elif root.is_dir():
            paths = sorted(
                path
                for path in root.rglob("*")
                if path.is_file() and path.suffix in STALE_MINIMAX_ENDPOINT_SCAN_SUFFIXES
            )
        else:
            continue
        for path in paths:
            if path.resolve() == Path(__file__).resolve():
                continue
            try:
                text = path.read_text(encoding="utf-8")
            except UnicodeDecodeError:
                continue
            for token in STALE_MINIMAX_ENDPOINT_TOKENS:
                if token in text:
                    fail(
                        findings,
                        f"stale MiniMax endpoint token {token!r} in {path.relative_to(ROOT)}; use https://api.minimaxi.com/v1 or a neutral non-official test URL",
                    )


def check_toml_loader_contract(findings: list[str]) -> None:
    try:
        source = Path(__file__).read_text(encoding="utf-8")
    except OSError as exc:
        fail(findings, f"toml_loader_contract_read_failed:{exc.__class__.__name__}")
        return
    require(
        "toml_missing" in source
        and "toml_read_failed" in source
        and "toml_decode_failed" in source
        and "toml_parse_failed" in source
        and "env_file_missing" in source
        and "env_file_read_failed" in source
        and "env_file_decode_failed" in source
        and "CHINESE_MODEL_CATALOG_SELF_TEST ok" in source
        and "--self-test" in source,
        findings,
        "Chinese model catalog config loaders must expose structured failure findings and self-test",
    )


def run_self_test() -> int:
    with tempfile.TemporaryDirectory(prefix="chinese-model-catalog-") as tmp:
        root = Path(tmp)
        cases: list[tuple[str, Path, str]] = []

        missing_path = root / "missing.toml"
        cases.append(("missing", missing_path, f"toml_missing:{missing_path}"))

        read_failed_path = root / "read-failed.toml"
        read_failed_path.mkdir()
        cases.append(("read_failed", read_failed_path, f"toml_read_failed:{read_failed_path}:"))

        decode_failed_path = root / "decode-failed.toml"
        decode_failed_path.write_bytes(b"\xff\n")
        cases.append(("decode_failed", decode_failed_path, f"toml_decode_failed:{decode_failed_path}"))

        parse_failed_path = root / "parse-failed.toml"
        parse_failed_path.write_text("llm = [\n", encoding="utf-8")
        cases.append(("parse_failed", parse_failed_path, f"toml_parse_failed:{parse_failed_path}"))

        for label, path, expected_prefix in cases:
            findings: list[str] = []
            payload = load_toml(path, findings)
            if payload != {} or not any(
                finding.startswith(expected_prefix) for finding in findings
            ):
                print(
                    f"SELF_TEST_FAIL {label}:payload={payload} findings={findings}",
                    file=sys.stderr,
                )
                return 1

        env_missing_findings: list[str] = []
        env_missing = load_env_file(root / "missing.env", env_missing_findings)
        if env_missing != {} or "env_file_missing" not in env_missing_findings:
            print(
                "SELF_TEST_FAIL env_file_missing:"
                f"payload={env_missing} findings={env_missing_findings}",
                file=sys.stderr,
            )
            return 1

        env_read_failed_findings: list[str] = []
        env_read_failed_path = root / "read-failed.env"
        env_read_failed_path.mkdir()
        env_read_failed = load_env_file(env_read_failed_path, env_read_failed_findings)
        if env_read_failed != {} or not any(
            finding.startswith("env_file_read_failed:")
            for finding in env_read_failed_findings
        ):
            print(
                "SELF_TEST_FAIL env_file_read_failed:"
                f"payload={env_read_failed} findings={env_read_failed_findings}",
                file=sys.stderr,
            )
            return 1

        env_decode_failed_findings: list[str] = []
        env_decode_failed_path = root / "decode-failed.env"
        env_decode_failed_path.write_bytes(b"\xff\n")
        env_decode_failed = load_env_file(env_decode_failed_path, env_decode_failed_findings)
        if env_decode_failed != {} or "env_file_decode_failed" not in env_decode_failed_findings:
            print(
                "SELF_TEST_FAIL env_file_decode_failed:"
                f"payload={env_decode_failed} findings={env_decode_failed_findings}",
                file=sys.stderr,
            )
            return 1

        endpoint_missing_findings: list[str] = []
        check_primary_model_endpoint_source_text(
            endpoint_missing_findings,
            "self_test_missing_endpoint.rs",
            'base_url = "https://example.invalid/v1"',
        )
        if not any("must use MiMo primary endpoint" in item for item in endpoint_missing_findings):
            print(
                "SELF_TEST_FAIL endpoint_missing:"
                f"findings={endpoint_missing_findings}",
                file=sys.stderr,
            )
            return 1

        endpoint_stale_findings: list[str] = []
        check_primary_model_endpoint_source_text(
            endpoint_stale_findings,
            "self_test_stale_endpoint.rs",
            f'{MIMO_PRIMARY_OPENAI_BASE_URL}\nhttps://token-plan-sgp.xiaomimimo.com/v1',
        )
        if not any("stale MiMo primary endpoint token" in item for item in endpoint_stale_findings):
            print(
                "SELF_TEST_FAIL endpoint_stale:"
                f"findings={endpoint_stale_findings}",
                file=sys.stderr,
            )
            return 1

    print("CHINESE_MODEL_CATALOG_SELF_TEST ok")
    return 0


def build_report(env_file: Path | None = None) -> dict[str, Any]:
    findings: list[str] = []
    env_values = load_env_file(env_file, findings)
    main = load_toml(MAIN_CONFIG, findings)
    docker = load_toml(DOCKER_CONFIG, findings)
    check_text_provider_config(findings, "configs/config.toml", main)
    check_text_provider_config(findings, "docker/config/config.toml", docker)
    check_main_docker_text_parity(findings, main, docker)
    check_primary_model_endpoint_source_alignment(findings)
    check_media_config(findings, main)
    check_vendor_patches(findings)
    check_chinese_case_gate(findings)
    catalog = build_catalog(main, env_values, findings)
    check_runtime_catalog_shape(findings, catalog)
    check_model_catalog_teaching_projection(findings)
    check_chinese_provider_smoke_live_scope(findings)
    check_no_stale_minimax_endpoints(findings)
    check_toml_loader_contract(findings)
    findings.extend(secret_scan_findings(catalog, "$.catalog"))
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
    parser.add_argument("--env-file", type=Path, help="optional env file used only for credential_state detection")
    parser.add_argument("--self-test", action="store_true", help="run local contract self-test")
    args = parser.parse_args()

    if args.self_test:
        return run_self_test()

    report = build_report(args.env_file)
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
