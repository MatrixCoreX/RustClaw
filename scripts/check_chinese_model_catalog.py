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


MAIN_CONFIG = ROOT / "configs/config.toml"
DOCKER_CONFIG = ROOT / "docker/config/config.toml"
IMAGE_CONFIG = ROOT / "configs/image.toml"
AUDIO_CONFIG = ROOT / "configs/audio.toml"
VIDEO_CONFIG = ROOT / "configs/video.toml"
MUSIC_CONFIG = ROOT / "configs/music.toml"
README = ROOT / "README.md"
README_ZH_CN = ROOT / "README.zh-CN.md"
NL_TESTS_README = ROOT / "scripts/nl_tests/README.md"
CHINESE_CASE_FILE = ROOT / "scripts/nl_tests/cases/nl_cases_chinese_model_adapter_20260715.txt"
CHINESE_PROVIDER_SMOKE_RUNNER = ROOT / "scripts/nl_tests/run_chinese_provider_smoke_matrix.sh"
CHINESE_PROVIDER_SMOKE_MATRIX_CHECKER = ROOT / "scripts/nl_tests/check_chinese_provider_smoke_matrix.py"
AGENT_PARITY_GATE_RUNNER = ROOT / "scripts/nl_tests/run_agent_parity_gate.sh"
SUITE_WRAPPER_CONTRACT_CHECKER = ROOT / "scripts/nl_tests/check_suite_wrapper_contract.py"
SUITE_ARTIFACT_CONTRACT_CHECKER = ROOT / "scripts/nl_tests/check_suite_artifact_contract.py"
ROLLOUT_METRICS_SUMMARY = ROOT / "scripts/nl_tests/summarize_rollout_metrics.py"
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


def check_chinese_provider_smoke_live_scope(findings: list[str]) -> None:
    require(
        CHINESE_PROVIDER_SMOKE_RUNNER.exists(),
        findings,
        f"missing {CHINESE_PROVIDER_SMOKE_RUNNER.relative_to(ROOT)}",
    )
    require(
        AGENT_PARITY_GATE_RUNNER.exists(),
        findings,
        f"missing {AGENT_PARITY_GATE_RUNNER.relative_to(ROOT)}",
    )
    require(
        SUITE_WRAPPER_CONTRACT_CHECKER.exists(),
        findings,
        f"missing {SUITE_WRAPPER_CONTRACT_CHECKER.relative_to(ROOT)}",
    )
    require(
        SUITE_ARTIFACT_CONTRACT_CHECKER.exists(),
        findings,
        f"missing {SUITE_ARTIFACT_CONTRACT_CHECKER.relative_to(ROOT)}",
    )
    require(
        README.exists(),
        findings,
        f"missing {README.relative_to(ROOT)}",
    )
    require(
        README_ZH_CN.exists(),
        findings,
        f"missing {README_ZH_CN.relative_to(ROOT)}",
    )
    require(
        NL_TESTS_README.exists(),
        findings,
        f"missing {NL_TESTS_README.relative_to(ROOT)}",
    )
    if not CHINESE_PROVIDER_SMOKE_RUNNER.exists() or not AGENT_PARITY_GATE_RUNNER.exists():
        return

    smoke_text = CHINESE_PROVIDER_SMOKE_RUNNER.read_text(encoding="utf-8")
    smoke_matrix_text = (
        CHINESE_PROVIDER_SMOKE_MATRIX_CHECKER.read_text(encoding="utf-8")
        if CHINESE_PROVIDER_SMOKE_MATRIX_CHECKER.exists()
        else ""
    )
    parity_text = AGENT_PARITY_GATE_RUNNER.read_text(encoding="utf-8")
    readme_text = README.read_text(encoding="utf-8") if README.exists() else ""
    readme_zh_text = README_ZH_CN.read_text(encoding="utf-8") if README_ZH_CN.exists() else ""
    nl_tests_readme_text = (
        NL_TESTS_README.read_text(encoding="utf-8") if NL_TESTS_README.exists() else ""
    )
    suite_wrapper_text = (
        SUITE_WRAPPER_CONTRACT_CHECKER.read_text(encoding="utf-8")
        if SUITE_WRAPPER_CONTRACT_CHECKER.exists()
        else ""
    )
    suite_artifact_contract_text = (
        SUITE_ARTIFACT_CONTRACT_CHECKER.read_text(encoding="utf-8")
        if SUITE_ARTIFACT_CONTRACT_CHECKER.exists()
        else ""
    )
    rollout_metrics_text = (
        ROLLOUT_METRICS_SUMMARY.read_text(encoding="utf-8")
        if ROLLOUT_METRICS_SUMMARY.exists()
        else ""
    )
    require(
        'DEFAULT_LIVE_PROVIDERS="${CHINESE_PROVIDER_LIVE_PROVIDERS:-minimax}"'
        in smoke_text,
        findings,
        "Chinese provider smoke runner must default live scope to minimax",
    )
    require(
        'if [[ "$LIVE_SCOPE_SET" -eq 0 ]]' in smoke_text
        and 'add_csv_live_providers "$DEFAULT_LIVE_PROVIDERS"' in smoke_text,
        findings,
        "Chinese provider smoke runner must apply the default live scope when no override is passed",
    )
    require(
        'if [[ "$item" == "all" ]]' in smoke_text and "LIVE_SCOPE_ALL=1" in smoke_text,
        findings,
        "Chinese provider smoke runner must keep explicit all-provider opt-in",
    )
    require(
        '"provider_not_in_live_scope"' in smoke_text,
        findings,
        "Chinese provider smoke runner must preserve provider_not_in_live_scope attribution",
    )
    require(
        "live_scope_providers=$(live_scope_csv)" in smoke_text,
        findings,
        "Chinese provider smoke runner must report the effective live scope as a machine token",
    )
    require(
        "path_ref()" in smoke_text
        and '"case_file": path_ref(case_file)' in smoke_text
        and '"output_file": path_ref(output_file)' in smoke_text
        and '"run_dir": path_ref(run_dir)' in smoke_text
        and 'CHINESE_PROVIDER_SMOKE_MATRIX out_dir_ref=$(path_ref "$OUT_DIR")' in smoke_text,
        findings,
        "Chinese provider smoke runner must write portable path refs instead of host paths",
    )
    require(
        "path_ref(case_file)" in smoke_matrix_text
        and "CHINESE_PROVIDER_SMOKE_MATRIX_SELF_TEST ok" in smoke_matrix_text
        and "external_path" in smoke_matrix_text,
        findings,
        "Chinese provider smoke case coverage must write portable case_file refs and self-test them",
    )
    require(
        "portable_path_ref" in rollout_metrics_text
        and "rollout_metrics_ok_line" in rollout_metrics_text
        and "ROLLOUT_METRICS_SELF_TEST ok" in rollout_metrics_text,
        findings,
        "rollout metrics summary must write portable source/output refs and self-test them",
    )
    require(
        'CHINESE_PROVIDER_LIVE_PROVIDERS="${CHINESE_PROVIDER_LIVE_PROVIDERS:-minimax}"'
        in parity_text,
        findings,
        "agent parity gate must default Chinese-provider live scope to minimax",
    )
    require(
        '--live-providers "$CHINESE_PROVIDER_LIVE_PROVIDERS"' in parity_text,
        findings,
        "agent parity gate must pass the configured Chinese-provider live scope to the smoke runner",
    )
    require(
        'CHINESE_PROVIDER_ENV_FILE="${CHINESE_PROVIDER_ENV_FILE:-${ROOT_DIR}/../runtime_env_filled.sh}"'
        in parity_text,
        findings,
        "agent parity gate must default Chinese-provider preflight env file to ../runtime_env_filled.sh",
    )
    require(
        "--chinese-env-file)" in parity_text and "--no-chinese-env-file)" in parity_text,
        findings,
        "agent parity gate must expose explicit Chinese-provider env-file override and disable options",
    )
    require(
        'chinese_provider_env_file_args+=(--env-file "$CHINESE_PROVIDER_ENV_FILE")'
        in parity_text,
        findings,
        "agent parity gate must build reusable Chinese-provider env-file args",
    )
    require(
        '"${chinese_provider_env_file_args[@]}"' in parity_text,
        findings,
        "agent parity gate must pass Chinese-provider env-file args to catalog and smoke checks",
    )
    require(
        "AGENT_PARITY_GATE_STEP chinese_model_catalog_self_test" in parity_text
        and "check_chinese_model_catalog.py" in parity_text
        and "--self-test" in parity_text
        and "chinese_model_catalog_self_test.txt" in parity_text,
        findings,
        "agent parity gate must run the Chinese model catalog self-test artifact",
    )
    require(
        "chinese_provider_live_providers=${CHINESE_PROVIDER_LIVE_PROVIDERS}" in parity_text,
        findings,
        "agent parity gate summary must record the Chinese-provider live scope",
    )
    require(
        "chinese_provider_env_file_state=${CHINESE_PROVIDER_ENV_FILE_STATE}" in parity_text,
        findings,
        "agent parity gate summary must record the Chinese-provider env-file state",
    )
    require(
        "chinese_provider_env_file_source=${CHINESE_PROVIDER_ENV_FILE_SOURCE}" in parity_text,
        findings,
        "agent parity gate summary must record the Chinese-provider env-file source token",
    )
    require(
        "chinese_provider_env_file=${CHINESE_PROVIDER_ENV_FILE}" not in parity_text,
        findings,
        "agent parity gate summary must not record the Chinese-provider env-file path",
    )
    require(
        "AGENT_PARITY_GATE_STEP runtime_hard_reply_baseline" in parity_text,
        findings,
        "agent parity gate must run the runtime hard-reply baseline guard step",
    )
    require(
        "check_no_runtime_hard_reply.py" in parity_text
        and 'check_no_runtime_hard_reply.py" --self-test' in parity_text
        and "runtime_hard_reply_baseline.txt" in parity_text,
        findings,
        "agent parity gate must self-test and write the runtime hard-reply baseline artifact",
    )
    require(
        "runtime_hard_reply_baseline=1" in parity_text,
        findings,
        "agent parity gate summary must record the runtime hard-reply baseline guard state",
    )
    require(
        "AGENT_PARITY_GATE_STEP policy_boundary_hard_reply" in parity_text,
        findings,
        "agent parity gate must run the policy-boundary hard-reply guard step",
    )
    require(
        "check_no_policy_boundary_hard_reply.py" in parity_text
        and 'check_no_policy_boundary_hard_reply.py" --self-test' in parity_text
        and "policy_boundary_hard_reply.txt" in parity_text,
        findings,
        "agent parity gate must self-test and write the policy-boundary hard-reply artifact",
    )
    require(
        "policy_boundary_hard_reply=1" in parity_text,
        findings,
        "agent parity gate summary must record the policy-boundary hard-reply guard state",
    )
    require(
        "AGENT_PARITY_GATE_STEP repair_no_user_text_fields" in parity_text,
        findings,
        "agent parity gate must run the repair no-user-text guard step",
    )
    require(
        "check_repair_no_user_text_fields.py" in parity_text
        and 'check_repair_no_user_text_fields.py" --self-test' in parity_text
        and "repair_no_user_text_fields.txt" in parity_text,
        findings,
        "agent parity gate must self-test and write the repair no-user-text artifact",
    )
    require(
        "repair_no_user_text_fields=1" in parity_text,
        findings,
        "agent parity gate summary must record the repair no-user-text guard state",
    )
    require(
        "AGENT_PARITY_GATE_STEP policy_decision_tokens" in parity_text,
        findings,
        "agent parity gate must run the policy decision token guard step",
    )
    require(
        "check_policy_decision_tokens.py" in parity_text
        and 'check_policy_decision_tokens.py" --self-test' in parity_text
        and "policy_decision_tokens.txt" in parity_text,
        findings,
        "agent parity gate must self-test and write the policy decision token artifact",
    )
    require(
        "policy_decision_tokens=1" in parity_text,
        findings,
        "agent parity gate summary must record the policy decision token guard state",
    )
    require(
        "AGENT_PARITY_GATE_STEP agent_loop_guard_final_scope" in parity_text,
        findings,
        "agent parity gate must run the final agent-loop guard scope step",
    )
    require(
        "check_agent_loop_guard_final_scope.py" in parity_text
        and 'check_agent_loop_guard_final_scope.py" --self-test' in parity_text
        and "agent_loop_guard_final_scope.txt" in parity_text,
        findings,
        "agent parity gate must self-test and write the final guard scope artifact",
    )
    require(
        "agent_loop_guard_final_scope=1" in parity_text,
        findings,
        "agent parity gate summary must record the final guard scope state",
    )
    require(
        "AGENT_PARITY_GATE_STEP registry_policy_contracts" in parity_text,
        findings,
        "agent parity gate must run the registry policy contract guard step",
    )
    require(
        "check_registry_policy_contracts.py" in parity_text
        and 'check_registry_policy_contracts.py" --self-test' in parity_text
        and "registry_policy_contracts.txt" in parity_text,
        findings,
        "agent parity gate must self-test and write the registry policy contract artifact",
    )
    require(
        "registry_policy_contracts=1" in parity_text,
        findings,
        "agent parity gate summary must record the registry policy contract guard state",
    )
    require(
        "AGENT_PARITY_GATE_STEP skill_registry_aliases" in parity_text,
        findings,
        "agent parity gate must run the skill registry aliases guard step",
    )
    require(
        "check_skill_registry_aliases.py" in parity_text
        and 'check_skill_registry_aliases.py" --self-test' in parity_text
        and "skill_registry_aliases.txt" in parity_text,
        findings,
        "agent parity gate must self-test and write the skill registry aliases artifact",
    )
    require(
        "skill_registry_aliases=1" in parity_text,
        findings,
        "agent parity gate summary must record the skill registry aliases guard state",
    )
    require(
        "AGENT_PARITY_GATE_STEP long_tail_skill_contracts" in parity_text,
        findings,
        "agent parity gate must run the long-tail skill contract guard step",
    )
    require(
        "check_long_tail_skill_contracts.py" in parity_text
        and 'check_long_tail_skill_contracts.py" --self-test' in parity_text
        and "long_tail_skill_contracts.txt" in parity_text,
        findings,
        "agent parity gate must self-test and write the long-tail skill contract artifact",
    )
    require(
        "long_tail_skill_contracts=1" in parity_text,
        findings,
        "agent parity gate summary must record the long-tail skill contract guard state",
    )
    require(
        "AGENT_PARITY_GATE_STEP task_lifecycle_contracts" in parity_text,
        findings,
        "agent parity gate must run the task lifecycle contract guard step",
    )
    require(
        "check_task_lifecycle_contracts.py" in parity_text
        and 'check_task_lifecycle_contracts.py" --self-test' in parity_text
        and "task_lifecycle_contracts.txt" in parity_text,
        findings,
        "agent parity gate must self-test and write the task lifecycle contract artifact",
    )
    require(
        "task_lifecycle_contracts=1" in parity_text,
        findings,
        "agent parity gate summary must record the task lifecycle contract guard state",
    )
    require(
        "AGENT_PARITY_GATE_STEP task_event_context_team_contracts" in parity_text,
        findings,
        "agent parity gate must run the task event/context/team contract guard step",
    )
    require(
        "check_task_event_context_team_contracts.py" in parity_text
        and 'check_task_event_context_team_contracts.py" --self-test' in parity_text
        and "task_event_context_team_contracts.txt" in parity_text,
        findings,
        "agent parity gate must self-test and write the task event/context/team artifact",
    )
    require(
        "task_event_context_team_contracts=1" in parity_text,
        findings,
        "agent parity gate summary must record the task event/context/team contract state",
    )
    require(
        "AGENT_PARITY_GATE_STEP clawcli_exec_replay_contracts" in parity_text,
        findings,
        "agent parity gate must run the clawcli exec/replay contract guard step",
    )
    require(
        "check_clawcli_exec_replay_contracts.py" in parity_text
        and 'check_clawcli_exec_replay_contracts.py" --self-test' in parity_text
        and "clawcli_exec_replay_contracts.txt" in parity_text,
        findings,
        "agent parity gate must self-test and write the clawcli exec/replay artifact",
    )
    require(
        "clawcli_exec_replay_contracts=1" in parity_text,
        findings,
        "agent parity gate summary must record the clawcli exec/replay contract state",
    )
    require(
        "AGENT_PARITY_GATE_STEP no_agent_mode_payload" in parity_text,
        findings,
        "agent parity gate must run the no-agent-mode payload guard step",
    )
    require(
        "check_no_agent_mode_payload.py" in parity_text
        and 'check_no_agent_mode_payload.py" --self-test' in parity_text
        and "no_agent_mode_payload.txt" in parity_text,
        findings,
        "agent parity gate must self-test and write the no-agent-mode payload guard artifact",
    )
    require(
        "no_agent_mode_payload=1" in parity_text,
        findings,
        "agent parity gate summary must record the no-agent-mode payload guard state",
    )
    require(
        "AGENT_PARITY_GATE_STEP agent_loop_static_contracts" in parity_text,
        findings,
        "agent parity gate must run the agent-loop static contracts step",
    )
    require(
        "check_route_authority_legacy_keys.py" in parity_text
        and "check_legacy_route_boundary.py" in parity_text
        and "check_pre_planner_exit_inventory.py" in parity_text
        and "check_frontdoor_boundary_dispatch.py" in parity_text
        and "check_no_nl_hardmatch.py" in parity_text
        and "check_historical_hardcoded_language.py" in parity_text
        and "AGENT_LOOP_STATIC_SELF_TEST check_route_authority_legacy_keys.py" in parity_text
        and "AGENT_LOOP_STATIC_SELF_TEST check_legacy_route_boundary.py" in parity_text
        and "AGENT_LOOP_STATIC_SELF_TEST check_pre_planner_exit_inventory.py" in parity_text
        and "AGENT_LOOP_STATIC_SELF_TEST check_frontdoor_boundary_dispatch.py" in parity_text
        and "AGENT_LOOP_STATIC_SELF_TEST check_no_nl_hardmatch.py" in parity_text
        and "AGENT_LOOP_STATIC_SELF_TEST check_historical_hardcoded_language.py" in parity_text
        and "agent_loop_static_contracts.txt" in parity_text,
        findings,
        "agent parity gate must write the agent-loop static contracts artifact",
    )
    require(
        "agent_loop_static_contracts=1" in parity_text,
        findings,
        "agent parity gate summary must record the agent-loop static contracts state",
    )
    require(
        "AGENT_PARITY_GATE_STEP semantic_boundary_contracts" in parity_text
        and "check_runtime_semantic_rewrite_boundary.py" in parity_text
        and 'check_runtime_semantic_rewrite_boundary.py" --self-test' in parity_text
        and "check_contract_repair_loop_observation_boundary.py" in parity_text
        and 'check_contract_repair_loop_observation_boundary.py" --self-test' in parity_text
        and "check_route_reason_marker_facade.py" in parity_text
        and 'check_route_reason_marker_facade.py" --self-test' in parity_text
        and "check_output_semantic_kind_write_boundary.py" in parity_text
        and 'check_output_semantic_kind_write_boundary.py" --self-test' in parity_text
        and "semantic_boundary_contracts.txt" in parity_text,
        findings,
        "agent parity gate must self-test and write the semantic boundary contracts artifact",
    )
    require(
        "semantic_boundary_contracts=1" in parity_text,
        findings,
        "agent parity gate summary must record the semantic boundary contracts state",
    )
    require(
        "AGENT_PARITY_GATE_STEP evidence_extractor_contracts" in parity_text
        and "check_evidence_extractor_contracts.py" in parity_text
        and 'check_evidence_extractor_contracts.py" --self-test' in parity_text
        and "evidence_extractor_contracts.txt" in parity_text,
        findings,
        "agent parity gate must write the evidence extractor contract artifact",
    )
    require(
        "evidence_extractor_contracts=1" in parity_text,
        findings,
        "agent parity gate summary must record the evidence extractor contract state",
    )
    require(
        "AGENT_PARITY_GATE_STEP secret_scan_contract" in parity_text,
        findings,
        "agent parity gate must run the shared secret scan contract step",
    )
    require(
        "check_secret_scan_contract.py" in parity_text
        and "secret_scan_contract.json" in parity_text,
        findings,
        "agent parity gate must write the shared secret scan contract artifact",
    )
    require(
        "secret_scan_contract=1" in parity_text,
        findings,
        "agent parity gate summary must record the shared secret scan contract state",
    )
    require(
        "AGENT_PARITY_GATE_STEP suite_wrapper_contract" in parity_text,
        findings,
        "agent parity gate must run the wrapped suite contract step",
    )
    require(
        "check_suite_wrapper_contract.py" in parity_text
        and "suite_wrapper_contract.json" in parity_text,
        findings,
        "agent parity gate must write the wrapped suite contract artifact",
    )
    require(
        "suite_wrapper_contract=1" in parity_text,
        findings,
        "agent parity gate summary must record the wrapped suite contract state",
    )
    require(
        "AGENT_PARITY_GATE_STEP runner_path_ref_contract" in parity_text,
        findings,
        "agent parity gate must run the runner path-ref contract step",
    )
    require(
        "check_runner_path_ref_contract.py" in parity_text
        and "runner_path_ref_contract.json" in parity_text,
        findings,
        "agent parity gate must write the runner path-ref contract artifact",
    )
    require(
        "runner_path_ref_contract=1" in parity_text,
        findings,
        "agent parity gate summary must record the runner path-ref contract state",
    )
    require(
        "SUITE_ARTIFACT_CONTRACT" in suite_wrapper_text
        and "AGENT_PARITY_GATE_REQUIRED_ARTIFACTS" in suite_wrapper_text
        and "AGENT_PARITY_GATE_REQUIRED_FLAGS" in suite_wrapper_text
        and "AGENT_PARITY_GATE_DYNAMIC_MACHINE_FIELDS" in suite_wrapper_text
        and "runner_path_ref_contract" in suite_wrapper_text
        and "agent_parity_gate_summary_bad_live_provider_scope" in suite_wrapper_text
        and "agent_parity_gate_summary_missing_live_provider_scope" in suite_wrapper_text
        and "validate_chinese_provider_env_file_summary" in suite_wrapper_text
        and "agent_parity_gate_summary_bad_env_file_state" in suite_wrapper_text
        and "agent_parity_gate_summary_bad_env_file_source" in suite_wrapper_text
        and "validate_gate_summary_no_host_paths" in suite_wrapper_text
        and "agent_parity_gate_summary_host_path" in suite_wrapper_text
        and "agent_parity_gate_summary_legacy_out_dir" in suite_wrapper_text
        and "agent_parity_gate_summary_bad_out_dir_ref" in suite_wrapper_text
        and "gate-summary-host-path" in suite_wrapper_text
        and "out_dir_ref" in suite_wrapper_text
        and "RUN_SUITE_FORBIDDEN_SNIPPETS" in suite_wrapper_text
        and "RUN_MULTI_TURN_SUITE" in suite_wrapper_text
        and "RUN_MULTI_TURN_FORBIDDEN_SNIPPETS" in suite_wrapper_text
        and "multi_turn_run_dir_ref" in suite_wrapper_text
        and "multi_turn_run_log_ref" in suite_wrapper_text
        and "run_dir_ref" in suite_wrapper_text
        and "run_log_ref" in suite_wrapper_text
        and "suite_artifact_contract_ref" in suite_wrapper_text
        and "clarify_run_dir_ref" in suite_wrapper_text
        and "clarify_run_log_ref" in suite_wrapper_text
        and "clarify_summary_jsonl_ref" in suite_wrapper_text
        and "context_run_dir_ref" in suite_wrapper_text
        and "context_run_log_ref" in suite_wrapper_text
        and "context_summary_jsonl_ref" in suite_wrapper_text
        and "agent-parity-run-log-host-path" in suite_wrapper_text
        and "validate_provider_smoke_path_refs" in suite_wrapper_text
        and "agent_parity_gate_provider_smoke_bad_path_ref" in suite_wrapper_text
        and "agent_parity_gate_provider_smoke_case_coverage_bad_case_file" in suite_wrapper_text
        and "rollout_metrics_contract" in suite_wrapper_text
        and "agent_parity_gate_metrics_host_path" in suite_wrapper_text
        and "--validate-contract-report-content" in suite_wrapper_text
        and "--require-contract-report-content-checked" in suite_wrapper_text
        and "validate_existing_contract_report" in suite_wrapper_text
        and "SUITE_ARTIFACT_CONTRACT_FORBIDDEN_SNIPPETS" in suite_wrapper_text
        and "check_forbidden_snippets" in suite_wrapper_text
        and "forbidden_snippet" in suite_wrapper_text
        and "agent_parity_gate_contract" in suite_wrapper_text,
        findings,
        "wrapped suite contract guard must statically protect agent parity nested artifact checks",
    )
    require(
        "AGENT_PARITY_GATE_REQUIRED_ARTIFACTS" in suite_artifact_contract_text
        and "agent_parity_gate/runtime_hard_reply_baseline.txt" in suite_artifact_contract_text
        and "agent_parity_gate/policy_boundary_hard_reply.txt" in suite_artifact_contract_text
        and "agent_parity_gate/repair_no_user_text_fields.txt" in suite_artifact_contract_text
        and "agent_parity_gate/policy_decision_tokens.txt" in suite_artifact_contract_text
        and "agent_parity_gate/agent_loop_guard_final_scope.txt" in suite_artifact_contract_text
        and "agent_parity_gate/registry_policy_contracts.txt" in suite_artifact_contract_text
        and "agent_parity_gate/skill_registry_aliases.txt" in suite_artifact_contract_text
        and "agent_parity_gate/long_tail_skill_contracts.txt" in suite_artifact_contract_text
        and "agent_parity_gate/task_lifecycle_contracts.txt" in suite_artifact_contract_text
        and "agent_parity_gate/task_event_context_team_contracts.txt" in suite_artifact_contract_text
        and "agent_parity_gate/clawcli_exec_replay_contracts.txt" in suite_artifact_contract_text
        and "agent_parity_gate/agent_loop_static_contracts.txt" in suite_artifact_contract_text
        and "agent_parity_gate/evidence_extractor_contracts.txt" in suite_artifact_contract_text
        and "agent_parity_gate/suite_wrapper_contract.json" in suite_artifact_contract_text
        and "agent_parity_gate/runner_path_ref_contract.json" in suite_artifact_contract_text
        and "agent_parity_gate/suite_artifact_contract_self_test.txt" in suite_artifact_contract_text
        and "AGENT_PARITY_GATE_REQUIRED_FLAGS" in suite_artifact_contract_text
        and "AGENT_PARITY_GATE_REQUIRED_MACHINE_FIELDS" in suite_artifact_contract_text
        and "AGENT_PARITY_GATE_DYNAMIC_MACHINE_FIELDS" in suite_artifact_contract_text
        and "AGENT_PARITY_GATE_TEXT_CONTENT_TOKENS" in suite_artifact_contract_text
        and "AGENT_PARITY_GATE_JSON_OK_ARTIFACTS" in suite_artifact_contract_text
        and '"runtime_hard_reply_baseline": "1"' in suite_artifact_contract_text
        and '"policy_boundary_hard_reply": "1"' in suite_artifact_contract_text
        and '"repair_no_user_text_fields": "1"' in suite_artifact_contract_text
        and '"policy_decision_tokens": "1"' in suite_artifact_contract_text
        and '"agent_loop_guard_final_scope": "1"' in suite_artifact_contract_text
        and '"registry_policy_contracts": "1"' in suite_artifact_contract_text
        and '"skill_registry_aliases": "1"' in suite_artifact_contract_text
        and '"long_tail_skill_contracts": "1"' in suite_artifact_contract_text
        and '"task_lifecycle_contracts": "1"' in suite_artifact_contract_text
        and '"task_event_context_team_contracts": "1"' in suite_artifact_contract_text
        and '"clawcli_exec_replay_contracts": "1"' in suite_artifact_contract_text
        and "RUNTIME_HARD_REPLY_ALL_SCAN" in suite_artifact_contract_text
        and "new=0" in suite_artifact_contract_text
        and "POLICY_BOUNDARY_HARD_REPLY_SELF_TEST ok" in suite_artifact_contract_text
        and "POLICY_BOUNDARY_HARD_REPLY_CHECK ok" in suite_artifact_contract_text
        and "REPAIR_USER_TEXT_FIELD_CHECK ok" in suite_artifact_contract_text
        and "POLICY_DECISION_TOKEN_SELF_TEST ok" in suite_artifact_contract_text
        and "POLICY_DECISION_TOKEN_CHECK ok" in suite_artifact_contract_text
        and "AGENT_LOOP_GUARD_FINAL_SCOPE_SELF_TEST ok" in suite_artifact_contract_text
        and "AGENT_LOOP_GUARD_FINAL_SCOPE_CHECK findings=0" in suite_artifact_contract_text
        and "REGISTRY_POLICY_CONTRACT_SELF_TEST ok" in suite_artifact_contract_text
        and "REGISTRY_POLICY_CONTRACT_CHECK ok" in suite_artifact_contract_text
        and "SKILL_REGISTRY_ALIAS_SELF_TEST ok" in suite_artifact_contract_text
        and "SKILL_REGISTRY_ALIAS_CHECK ok" in suite_artifact_contract_text
        and "LONG_TAIL_SKILL_CONTRACT_SELF_TEST ok" in suite_artifact_contract_text
        and "LONG_TAIL_SKILL_CONTRACT_CHECK ok" in suite_artifact_contract_text
        and "TASK_LIFECYCLE_CONTRACT_SELF_TEST ok" in suite_artifact_contract_text
        and "TASK_LIFECYCLE_CONTRACT_CHECK findings=0" in suite_artifact_contract_text
        and "TASK_EVENT_CONTEXT_TEAM_CONTRACT_SELF_TEST ok" in suite_artifact_contract_text
        and "TASK_EVENT_CONTEXT_TEAM_CONTRACT_CHECK findings=0" in suite_artifact_contract_text
        and "CLAWCLI_EXEC_REPLAY_CONTRACT_SELF_TEST ok" in suite_artifact_contract_text
        and "CLAWCLI_EXEC_REPLAY_CONTRACT_CHECK findings=0" in suite_artifact_contract_text
        and "AGENT_LOOP_STATIC_SELF_TEST check_route_authority_legacy_keys.py" in suite_artifact_contract_text
        and "AGENT_LOOP_STATIC_SELF_TEST check_legacy_route_boundary.py" in suite_artifact_contract_text
        and "AGENT_LOOP_STATIC_SELF_TEST check_pre_planner_exit_inventory.py" in suite_artifact_contract_text
        and "AGENT_LOOP_STATIC_SELF_TEST check_frontdoor_boundary_dispatch.py" in suite_artifact_contract_text
        and "FRONTDOOR_BOUNDARY_DISPATCH_CHECK findings=0" in suite_artifact_contract_text
        and "AGENT_LOOP_STATIC_SELF_TEST check_no_nl_hardmatch.py" in suite_artifact_contract_text
        and "AGENT_LOOP_STATIC_SELF_TEST check_historical_hardcoded_language.py" in suite_artifact_contract_text
        and "agent_parity_gate/semantic_boundary_contracts.txt" in suite_artifact_contract_text
        and '"semantic_boundary_contracts": "1"' in suite_artifact_contract_text
        and "RUNTIME_SEMANTIC_REWRITE_BOUNDARY_CHECK findings=0" in suite_artifact_contract_text
        and "CONTRACT_REPAIR_LOOP_OBSERVATION_BOUNDARY findings=0" in suite_artifact_contract_text
        and "ROUTE_REASON_MARKER_FACADE_SELF_TEST ok" in suite_artifact_contract_text
        and "ROUTE_REASON_MARKER_FACADE_CHECK findings=0" in suite_artifact_contract_text
        and "OUTPUT_SEMANTIC_KIND_WRITE_BOUNDARY_SELF_TEST ok" in suite_artifact_contract_text
        and "OUTPUT_SEMANTIC_KIND_WRITE_BOUNDARY_CHECK findings=0" in suite_artifact_contract_text
        and '"evidence_extractor_contracts": "1"' in suite_artifact_contract_text
        and "EVIDENCE_EXTRACTOR_CONTRACT_SELF_TEST ok" in suite_artifact_contract_text
        and "EVIDENCE_EXTRACTOR_CONTRACT_CHECK findings=0" in suite_artifact_contract_text
        and '"runner_path_ref_contract": "1"' in suite_artifact_contract_text
        and "AGENT_PARITY_GATE_OPTIONAL_ARTIFACTS_BY_FLAG" in suite_artifact_contract_text
        and "AGENT_PARITY_CHINESE_MODEL_PROVIDERS" in suite_artifact_contract_text
        and "validate_text_artifact_tokens" in suite_artifact_contract_text
        and "agent_parity_gate_artifact_decode_failed" in suite_artifact_contract_text
        and "text-artifact-decode-failed" in suite_artifact_contract_text
        and "validate_json_artifact_ok" in suite_artifact_contract_text
        and "agent_parity_gate_artifact_bad_shape" in suite_artifact_contract_text
        and "json-ok-artifact-bad-shape" in suite_artifact_contract_text
        and "validate_compact_coverage_artifact" in suite_artifact_contract_text
        and "validate_chinese_model_catalog_artifact" in suite_artifact_contract_text
        and "chinese_model_catalog_self_test.txt" in suite_artifact_contract_text
        and "CHINESE_MODEL_CATALOG_SELF_TEST ok" in suite_artifact_contract_text
        and (
            "agent_parity_gate_chinese_model_catalog_bad_catalog_shape"
            in suite_artifact_contract_text
        )
        and "chinese-model-catalog-bad-catalog-shape" in suite_artifact_contract_text
        and "validate_provider_smoke_artifacts" in suite_artifact_contract_text
        and (
            "agent_parity_gate_provider_smoke_bad_providers_shape"
            in suite_artifact_contract_text
        )
        and "provider-smoke-bad-providers-shape" in suite_artifact_contract_text
        and "validate_provider_smoke_case_coverage" in suite_artifact_contract_text
        and (
            "agent_parity_gate_provider_smoke_case_coverage_bad_provider_tags"
            in suite_artifact_contract_text
        )
        and "provider-case-coverage-bad-provider-tags" in suite_artifact_contract_text
        and "agent_parity_gate_provider_smoke_case_coverage_bad_case_file" in suite_artifact_contract_text
        and "provider-case-coverage-bad-case-file" in suite_artifact_contract_text
        and "parse_provider_summary_jsonl" in suite_artifact_contract_text
        and "agent_parity_gate_provider_summary_bad_json_line" in suite_artifact_contract_text
        and "agent_parity_gate_provider_summary_bad_row" in suite_artifact_contract_text
        and "provider-summary-jsonl-row-errors" in suite_artifact_contract_text
        and "validate_provider_smoke_path_refs" in suite_artifact_contract_text
        and "agent_parity_gate_provider_smoke_bad_path_ref" in suite_artifact_contract_text
        and "provider_path_ref_errors" in suite_artifact_contract_text
        and "parse_live_provider_scope" in suite_artifact_contract_text
        and "validate_live_provider_scope" in suite_artifact_contract_text
        and "agent_parity_gate_summary_bad_live_provider_scope" in suite_artifact_contract_text
        and "live_provider_scope" in suite_artifact_contract_text
        and "validate_chinese_provider_env_file_summary" in suite_artifact_contract_text
        and "agent_parity_gate_summary_bad_env_file_state" in suite_artifact_contract_text
        and "agent_parity_gate_summary_bad_env_file_source" in suite_artifact_contract_text
        and "env_file_summary" in suite_artifact_contract_text
        and "validate_gate_summary_no_host_paths" in suite_artifact_contract_text
        and "agent_parity_gate_summary_host_path" in suite_artifact_contract_text
        and "agent_parity_gate_summary_legacy_out_dir" in suite_artifact_contract_text
        and "agent_parity_gate_summary_bad_out_dir_ref" in suite_artifact_contract_text
        and "gate-summary-host-path" in suite_artifact_contract_text
        and "out_dir_ref" in suite_artifact_contract_text
        and 'validate_text_artifact_no_host_paths(run_dir, "run.log")'
        in suite_artifact_contract_text
        and "agent_parity_gate_artifact_host_path:run.log" in suite_artifact_contract_text
        and "agent-parity-run-log-host-path" in suite_artifact_contract_text
        and "expected_live_scope_providers" in suite_artifact_contract_text
        and "provider_not_in_live_scope" in suite_artifact_contract_text
        and "validate_rollout_metrics_artifact" in suite_artifact_contract_text
        and "rollout_metrics_contract.txt" in suite_artifact_contract_text
        and "ROLLOUT_METRICS_SELF_TEST ok" in suite_artifact_contract_text
        and "agent_parity_gate_metrics_host_path" in suite_artifact_contract_text
        and "agent_parity_gate_metrics_bad_source_run_dir" in suite_artifact_contract_text
        and "rollout_metrics_text_host_path" in suite_artifact_contract_text
        and "load_json_artifact" in suite_artifact_contract_text
        and "load-json-artifact-bad-shape" in suite_artifact_contract_text
        and "load-json-artifact-decode-failed" in suite_artifact_contract_text
        and "summary_decode_failed" in suite_artifact_contract_text
        and "artifact_index_decode_failed" in suite_artifact_contract_text
        and "summary-decode-failed" in suite_artifact_contract_text
        and "artifact-index-decode-failed" in suite_artifact_contract_text
        and "validate_enabled_agent_parity_optional_artifacts" in suite_artifact_contract_text
        and "agent_parity_gate_summary_missing" in suite_artifact_contract_text
        and "agent-parity-missing-gate-summary" in suite_artifact_contract_text
        and "return findings, content_checks" in suite_artifact_contract_text
        and "validate_existing_contract_report" in suite_artifact_contract_text
        and "--validate-contract-report-content" in suite_artifact_contract_text
        and "--require-contract-report-content-checked" in suite_artifact_contract_text
        and '"contract_report_content_checked"' in suite_artifact_contract_text
        and "stored_agent_contract" in suite_artifact_contract_text
        and "stored_report_override" in suite_artifact_contract_text
        and "contract_report_missing" in suite_artifact_contract_text
        and "contract_report_read_failed" in suite_artifact_contract_text
        and "contract_report_decode_failed" in suite_artifact_contract_text
        and "contract-report-decode-failed" in suite_artifact_contract_text
        and "contract_report_bad_json" in suite_artifact_contract_text
        and "contract_report_bad_shape" in suite_artifact_contract_text
        and "contract_report_not_ok" in suite_artifact_contract_text
        and "contract_report_bad_run_dir" in suite_artifact_contract_text
        and "contract_report_bad_require_contract_report" in suite_artifact_contract_text
        and "contract_report_findings_not_empty" in suite_artifact_contract_text
        and "contract_report_content_checked_not_true" in suite_artifact_contract_text
        and "contract_report_summary_mismatch" in suite_artifact_contract_text
        and "contract_report_agent_parity_contract_mismatch" in suite_artifact_contract_text
        and "contract_report_unexpected_agent_parity_contract" in suite_artifact_contract_text
        and "unexpected_agent_contract" in suite_artifact_contract_text
        and "missing-contract-report" in suite_artifact_contract_text
        and "read-failed" in suite_artifact_contract_text
        and "bad-json" in suite_artifact_contract_text
        and "bad-shape" in suite_artifact_contract_text
        and '"agent_loop_static_contracts": "1"' in suite_artifact_contract_text
        and '"semantic_boundary_contracts": "1"' in suite_artifact_contract_text
        and '"agent_loop_guard_final_scope": "1"' in suite_artifact_contract_text
        and '"registry_policy_contracts": "1"' in suite_artifact_contract_text
        and '"skill_registry_aliases": "1"' in suite_artifact_contract_text
        and '"long_tail_skill_contracts": "1"' in suite_artifact_contract_text
        and '"task_lifecycle_contracts": "1"' in suite_artifact_contract_text
        and '"task_event_context_team_contracts": "1"' in suite_artifact_contract_text
        and '"clawcli_exec_replay_contracts": "1"' in suite_artifact_contract_text
        and '"evidence_extractor_contracts": "1"' in suite_artifact_contract_text
        and '"suite_wrapper_contract": "1"' in suite_artifact_contract_text
        and '"suite_artifact_contract_self_test": "1"' in suite_artifact_contract_text
        and "SUITE_ARTIFACT_CONTRACT_SELF_TEST ok" in suite_artifact_contract_text
        and '"live_metrics": {"0", "1"}' in suite_artifact_contract_text
        and (
            'live_metrics_enabled = gate_summary.get("live_metrics") == "1"'
            in suite_artifact_contract_text
        )
        and "agent_parity_gate_summary_bad_machine_field" in suite_artifact_contract_text
        and '"required_machine_field_count"' in suite_artifact_contract_text
        and "len(AGENT_PARITY_GATE_DYNAMIC_MACHINE_FIELDS)" in suite_artifact_contract_text
        and '"content_check_count"' in suite_artifact_contract_text
        and 'summary.get("suite") == "agent_parity_gate"' in suite_artifact_contract_text
        and '"agent_parity_gate_contract"' in suite_artifact_contract_text,
        findings,
        "suite artifact contract checker must verify wrapped agent parity nested artifacts, flags, and success content",
    )
    require(
        "AGENT_PARITY_GATE_STEP llm_raw_trace_runner_contract" in parity_text,
        findings,
        "agent parity gate must run the NL raw LLM trace runner contract step",
    )
    require(
        "AGENT_PARITY_GATE_STEP suite_artifact_contract_self_test" in parity_text
        and "check_suite_artifact_contract.py\" --self-test" in parity_text
        and "suite_artifact_contract_self_test.txt" in parity_text,
        findings,
        "agent parity gate must run the suite artifact contract checker self-test",
    )
    require(
        "suite_artifact_contract_self_test=1" in parity_text,
        findings,
        "agent parity gate summary must record the suite artifact contract self-test state",
    )
    require(
        "print_llm_raw_trace.py\" --self-test" in parity_text
        and "check_llm_raw_trace_runner_contract.py" in parity_text
        and "llm_raw_trace_runner_contract.txt" in parity_text,
        findings,
        "agent parity gate must write the NL raw LLM trace runner contract artifact",
    )
    require(
        "llm_raw_trace_runner_contract=1" in parity_text,
        findings,
        "agent parity gate summary must record the NL raw LLM trace runner contract state",
    )
    require(
        "AGENT_PARITY_GATE_STEP rollout_metrics_contract" in parity_text
        and "summarize_rollout_metrics.py\" --self-test" in parity_text
        and "rollout_metrics_contract.txt" in parity_text
        and "rollout_metrics_contract=1" in parity_text,
        findings,
        "agent parity gate must run and record the rollout metrics path contract self-test",
    )
    require(
        "LIVE_METRICS_RAN=1" in parity_text
        and "live_metrics=${LIVE_METRICS_RAN}" in parity_text,
        findings,
        "agent parity gate summary must record whether live run metrics actually executed",
    )
    require(
        'if [[ -n "${NL_SUITE_RUN_DIR:-}" ]]' in parity_text
        and 'OUT_DIR="${NL_SUITE_RUN_DIR}/agent_parity_gate"' in parity_text,
        findings,
        "agent parity gate must co-locate artifacts under NL_SUITE_RUN_DIR when wrapped by run_suite",
    )
    require(
        "path_ref()" in parity_text
        and 'AGENT_PARITY_GATE out_dir_ref=$(path_ref "$OUT_DIR")' in parity_text
        and 'echo "out_dir_ref=$(path_ref "$OUT_DIR")"' in parity_text
        and 'AGENT_PARITY_GATE_OK out_dir_ref=$(path_ref "$OUT_DIR")' in parity_text
        and 'out_dir=${OUT_DIR}' not in parity_text,
        findings,
        "agent parity gate must report portable out_dir refs instead of host paths",
    )
    for label, readme_body in (("README.md", readme_text), ("README.zh-CN.md", readme_zh_text)):
        require(
            "agent_loop_static_contracts.txt" in readme_body
            and "runtime_hard_reply_baseline.txt" in readme_body
            and "policy_boundary_hard_reply.txt" in readme_body
            and "repair_no_user_text_fields.txt" in readme_body
            and "policy_decision_tokens.txt" in readme_body
            and "agent_loop_guard_final_scope.txt" in readme_body
            and "registry_policy_contracts.txt" in readme_body
            and "skill_registry_aliases.txt" in readme_body
            and "long_tail_skill_contracts.txt" in readme_body
            and "task_lifecycle_contracts.txt" in readme_body
            and "task_event_context_team_contracts.txt" in readme_body
            and "clawcli_exec_replay_contracts.txt" in readme_body
            and "no_agent_mode_payload.txt" in readme_body
            and "semantic_boundary_contracts.txt" in readme_body
            and "evidence_extractor_contracts.txt" in readme_body
            and "self-test" in readme_body
            and "check_evidence_extractor_contracts.py --self-test" in readme_body
            and "suite_artifact_contract.json" in readme_body
            and "suite_artifact_contract_self_test.txt" in readme_body
            and "chinese_model_catalog_self_test.txt" in readme_body
            and "agent_parity_gate_contract.checked=true" in readme_body
            and "--validate-contract-report-content" in readme_body
            and "--require-contract-report-content-checked" in readme_body
            and "contract_report_content_checked=true" in readme_body
            and "live_metrics=0|1" in readme_body
            and "metrics=1" in readme_body
            and "live_metrics=1" in readme_body
            and "out_dir_ref" in readme_body
            and "run_dir_ref" in readme_body
            and "run_log_ref" in readme_body
            and "runner_path_ref_contract.json" in readme_body
            and "llm_raw_trace_runner_contract.txt" in readme_body,
            findings,
            f"{label} must document agent parity nested/static/raw-trace gate artifacts",
        )
        require(
            "check_frontdoor_boundary_dispatch.py" in readme_body
            and "FRONTDOOR_BOUNDARY_DISPATCH_CHECK findings=0" in readme_body,
            findings,
            f"{label} must document the frontdoor boundary static guard",
        )
        require(
            "semantic_boundary_contracts.txt" in readme_body
            and "RUNTIME_SEMANTIC_REWRITE_BOUNDARY_CHECK findings=0" in readme_body
            and "CONTRACT_REPAIR_LOOP_OBSERVATION_BOUNDARY findings=0" in readme_body
            and "ROUTE_REASON_MARKER_FACADE_SELF_TEST ok" in readme_body
            and "OUTPUT_SEMANTIC_KIND_WRITE_BOUNDARY_SELF_TEST ok" in readme_body,
            findings,
            f"{label} must document the semantic boundary contracts artifact",
        )
    require(
        "runtime_hard_reply_baseline.txt" in nl_tests_readme_text
        and "runtime_hard_reply_baseline=1" in nl_tests_readme_text
        and "RUNTIME_HARD_REPLY_ALL_SCAN" in nl_tests_readme_text
        and "new=0" in nl_tests_readme_text,
        findings,
        "NL tests README must document runtime hard-reply baseline artifact content",
    )
    require(
        "policy_boundary_hard_reply.txt" in nl_tests_readme_text
        and "policy_boundary_hard_reply=1" in nl_tests_readme_text
        and "POLICY_BOUNDARY_HARD_REPLY_SELF_TEST ok" in nl_tests_readme_text
        and "POLICY_BOUNDARY_HARD_REPLY_CHECK ok" in nl_tests_readme_text,
        findings,
        "NL tests README must document policy-boundary hard-reply artifact content",
    )
    require(
        "repair_no_user_text_fields.txt" in nl_tests_readme_text
        and "repair_no_user_text_fields=1" in nl_tests_readme_text
        and "REPAIR_USER_TEXT_FIELD_CHECK ok" in nl_tests_readme_text,
        findings,
        "NL tests README must document repair no-user-text artifact content",
    )
    require(
        "policy_decision_tokens.txt" in nl_tests_readme_text
        and "policy_decision_tokens=1" in nl_tests_readme_text
        and "POLICY_DECISION_TOKEN_SELF_TEST ok" in nl_tests_readme_text
        and "POLICY_DECISION_TOKEN_CHECK ok" in nl_tests_readme_text,
        findings,
        "NL tests README must document policy decision token artifact content",
    )
    require(
        "agent_loop_guard_final_scope.txt" in nl_tests_readme_text
        and "agent_loop_guard_final_scope=1" in nl_tests_readme_text
        and "AGENT_LOOP_GUARD_FINAL_SCOPE_SELF_TEST ok" in nl_tests_readme_text
        and "AGENT_LOOP_GUARD_FINAL_SCOPE_CHECK findings=0" in nl_tests_readme_text,
        findings,
        "NL tests README must document final agent-loop guard scope artifact content",
    )
    require(
        "registry_policy_contracts.txt" in nl_tests_readme_text
        and "registry_policy_contracts=1" in nl_tests_readme_text
        and "REGISTRY_POLICY_CONTRACT_SELF_TEST ok" in nl_tests_readme_text
        and "REGISTRY_POLICY_CONTRACT_CHECK ok" in nl_tests_readme_text,
        findings,
        "NL tests README must document registry policy contract artifact content",
    )
    require(
        "skill_registry_aliases.txt" in nl_tests_readme_text
        and "skill_registry_aliases=1" in nl_tests_readme_text
        and "SKILL_REGISTRY_ALIAS_SELF_TEST ok" in nl_tests_readme_text
        and "SKILL_REGISTRY_ALIAS_CHECK ok" in nl_tests_readme_text,
        findings,
        "NL tests README must document skill registry aliases artifact content",
    )
    require(
            "long_tail_skill_contracts.txt" in nl_tests_readme_text
            and "long_tail_skill_contracts=1" in nl_tests_readme_text
            and "LONG_TAIL_SKILL_CONTRACT_SELF_TEST ok" in nl_tests_readme_text
            and "LONG_TAIL_SKILL_CONTRACT_CHECK ok" in nl_tests_readme_text,
        findings,
        "NL tests README must document long-tail skill contract artifact content",
    )
    require(
        "task_lifecycle_contracts.txt" in nl_tests_readme_text
        and "task_lifecycle_contracts=1" in nl_tests_readme_text
        and "TASK_LIFECYCLE_CONTRACT_SELF_TEST ok" in nl_tests_readme_text
        and "TASK_LIFECYCLE_CONTRACT_CHECK findings=0" in nl_tests_readme_text,
        findings,
        "NL tests README must document task lifecycle contract artifact content",
    )
    require(
        "task_event_context_team_contracts.txt" in nl_tests_readme_text
        and "task_event_context_team_contracts=1" in nl_tests_readme_text
        and "TASK_EVENT_CONTEXT_TEAM_CONTRACT_SELF_TEST ok" in nl_tests_readme_text
        and "TASK_EVENT_CONTEXT_TEAM_CONTRACT_CHECK findings=0" in nl_tests_readme_text,
        findings,
        "NL tests README must document task event/context/team artifact content",
    )
    require(
        "clawcli_exec_replay_contracts.txt" in nl_tests_readme_text
        and "clawcli_exec_replay_contracts=1" in nl_tests_readme_text
        and "CLAWCLI_EXEC_REPLAY_CONTRACT_SELF_TEST ok" in nl_tests_readme_text
        and "CLAWCLI_EXEC_REPLAY_CONTRACT_CHECK findings=0" in nl_tests_readme_text,
        findings,
        "NL tests README must document clawcli exec/replay artifact content",
    )
    require(
        "evidence_extractor_contracts.txt" in nl_tests_readme_text
        and "AGENT_LOOP_STATIC_SELF_TEST" in nl_tests_readme_text
        and "check_frontdoor_boundary_dispatch.py" in nl_tests_readme_text
        and "FRONTDOOR_BOUNDARY_DISPATCH_CHECK findings=0" in nl_tests_readme_text
        and "evidence_extractor_contracts=1" in nl_tests_readme_text
        and "EVIDENCE_EXTRACTOR_CONTRACT_SELF_TEST ok" in nl_tests_readme_text
        and "EVIDENCE_EXTRACTOR_CONTRACT_CHECK findings=0" in nl_tests_readme_text
        and "agent_parity_gate/evidence_extractor_contracts.txt" in nl_tests_readme_text,
        findings,
        "scripts/nl_tests/README.md must document the evidence extractor gate artifact",
    )
    require(
        "semantic_boundary_contracts.txt" in nl_tests_readme_text
        and "semantic_boundary_contracts=1" in nl_tests_readme_text
        and "RUNTIME_SEMANTIC_REWRITE_BOUNDARY_CHECK findings=0" in nl_tests_readme_text
        and "CONTRACT_REPAIR_LOOP_OBSERVATION_BOUNDARY findings=0" in nl_tests_readme_text
        and "ROUTE_REASON_MARKER_FACADE_SELF_TEST ok" in nl_tests_readme_text
        and "ROUTE_REASON_MARKER_FACADE_CHECK findings=0" in nl_tests_readme_text
        and "OUTPUT_SEMANTIC_KIND_WRITE_BOUNDARY_SELF_TEST ok" in nl_tests_readme_text
        and "OUTPUT_SEMANTIC_KIND_WRITE_BOUNDARY_CHECK findings=0" in nl_tests_readme_text,
        findings,
        "NL tests README must document semantic boundary contract artifact content",
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
