use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

use super::async_projection::{
    image_cancelled_adapter_result, image_poll_adapter_result, image_poll_response,
};
use super::*;

pub(super) fn execute_poll(
    cfg: &RootConfig,
    workspace_root: &Path,
    obj: &Map<String, Value>,
) -> Result<(String, Value), String> {
    let requested_vendor = obj.get("vendor").and_then(Value::as_str);
    let vendor = select_vendor(
        requested_vendor,
        cfg.image_generation.default_vendor.as_deref(),
        cfg.llm.selected_vendor.as_deref(),
    );
    let provider_name = vendor_name(vendor);
    let provider_cfg = resolved_vendor_config(cfg, vendor);
    let model = resolve_image_model(cfg, vendor, provider_cfg, obj);
    let model_kind = image_model_kind(cfg, vendor, &model);
    let task_id = required_string_arg(obj, "task_id")?;
    let job_id = obj
        .get("job_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| provider_image_job_id(provider_name, &task_id));
    let poll_after_seconds = image_poll_after_seconds(obj);
    let expires_at = image_expires_at(obj);
    if expires_at <= unix_ts() as i64 {
        let adapter_result = image_poll_adapter_result(
            &task_id,
            &job_id,
            provider_name,
            &model,
            model_kind,
            poll_after_seconds,
            expires_at,
            "expired",
            None,
            optional_bool(obj, "dry_run").unwrap_or(false),
            Some("async_poll_expired"),
            Some("clawd.task.async_poll_expired"),
        )?;
        return Ok(image_poll_response(
            &task_id,
            &job_id,
            provider_name,
            &model,
            model_kind,
            poll_after_seconds,
            expires_at,
            adapter_result,
            json!({"status": "expired"}),
        ));
    }

    if optional_bool(obj, "dry_run").unwrap_or(false) {
        let status = obj
            .get("mock_status")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("running");
        let output_path = image_poll_output_path(cfg, workspace_root, obj)?;
        let adapter_result = image_poll_adapter_result(
            &task_id,
            &job_id,
            provider_name,
            &model,
            model_kind,
            poll_after_seconds,
            expires_at,
            status,
            Some(output_path.to_string_lossy().as_ref()),
            true,
            None,
            None,
        )?;
        return Ok(image_poll_response(
            &task_id,
            &job_id,
            provider_name,
            &model,
            model_kind,
            poll_after_seconds,
            expires_at,
            adapter_result,
            json!({
                "status": status,
                "file_id": obj.get("mock_file_id").cloned().unwrap_or(Value::Null),
            }),
        ));
    }

    let adapter_result = json!({
        "schema_version": 1,
        "adapter_kind": "media_job_poll",
        "status": "failed",
        "job_id": job_id,
        "result_ref": job_id,
        "poll_after_seconds": poll_after_seconds,
        "poll_after_ms": poll_after_seconds.saturating_mul(1_000),
        "expires_at": expires_at,
        "message_key": "clawd.task.async_poll_adapter_failed",
        "error_code": "provider_image_poll_adapter_missing",
        "retryable": false,
        "failure_result_json": {
            "schema_version": 1,
            "source": "image_generate_poll_adapter",
            "provider": provider_name,
            "model": model,
            "model_kind": model_kind,
            "task_id": task_id,
            "job_id": job_id,
            "status": "requires_provider_adapter",
        },
    });
    Ok(image_poll_response(
        &task_id,
        &job_id,
        provider_name,
        &model,
        model_kind,
        poll_after_seconds,
        expires_at,
        adapter_result,
        json!({"status": "requires_provider_adapter"}),
    ))
}

pub(super) fn execute_cancel(
    cfg: &RootConfig,
    obj: &Map<String, Value>,
) -> Result<(String, Value), String> {
    let requested_vendor = obj.get("vendor").and_then(Value::as_str);
    let vendor = select_vendor(
        requested_vendor,
        cfg.image_generation.default_vendor.as_deref(),
        cfg.llm.selected_vendor.as_deref(),
    );
    let provider_name = vendor_name(vendor);
    let provider_cfg = resolved_vendor_config(cfg, vendor);
    let model = resolve_image_model(cfg, vendor, provider_cfg, obj);
    let model_kind = image_model_kind(cfg, vendor, &model);
    let task_id = required_string_arg(obj, "task_id")?;
    let job_id = obj
        .get("job_id")
        .or_else(|| obj.get("cancel_token"))
        .or_else(|| obj.get("cancel_ref"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| provider_image_job_id(provider_name, &task_id));
    let cancelled_at = unix_ts() as i64;
    let provider_cancel_contract = json!({
        "provider": provider_name,
        "skill_name": "image_generate",
        "task_id": task_id,
        "job_id": job_id,
        "cancel_ref": job_id,
    });

    if optional_bool(obj, "dry_run").unwrap_or(false) {
        let adapter_result = image_cancelled_adapter_result(
            &task_id,
            &job_id,
            provider_name,
            &model,
            model_kind,
            cancelled_at,
        );
        return Ok((
            format!("IMAGE_TASK_CANCELLED:{task_id}"),
            json!({
                "provider": provider_name,
                "model": model,
                "model_kind": model_kind,
                "task_id": task_id,
                "job_id": job_id,
                "status": "cancelled",
                "dry_run": true,
                "provider_cancel_contract": provider_cancel_contract,
                "async_cancel_adapter_result": adapter_result,
                "async_poll_adapter_result": adapter_result,
            }),
        ));
    }

    let adapter_result = json!({
        "schema_version": 1,
        "adapter_kind": "media_job_poll",
        "status": "requires_provider_adapter",
        "job_id": job_id,
        "result_ref": job_id,
        "cancel_ref": job_id,
        "cancel_token": job_id,
        "cancelled_at": cancelled_at,
        "message_key": "clawd.task.cancelled",
        "error_code": "provider_cancel_adapter_missing",
        "retryable": false,
        "provider_cancel_contract": provider_cancel_contract,
    });
    Ok((
        format!("IMAGE_TASK_CANCEL_ADAPTER_REQUIRED:{task_id}"),
        json!({
            "provider": provider_name,
            "model": model,
            "model_kind": model_kind,
            "task_id": task_id,
            "job_id": job_id,
            "status": "requires_provider_adapter",
            "provider_cancel_contract": provider_cancel_contract,
            "async_cancel_adapter_result": adapter_result,
        }),
    ))
}

fn select_vendor(
    requested: Option<&str>,
    section_default: Option<&str>,
    selected_vendor: Option<&str>,
) -> VendorKind {
    vendor_order(requested, section_default, selected_vendor)
        .into_iter()
        .next()
        .unwrap_or(VendorKind::OpenAI)
}

fn resolve_image_model(
    cfg: &RootConfig,
    vendor: VendorKind,
    provider_cfg: Option<&VendorConfig>,
    obj: &Map<String, Value>,
) -> String {
    obj.get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| cfg.image_generation.default_model.as_deref())
        .or_else(|| {
            first_model_candidate(
                None,
                vendor_models(&cfg.image_generation, vendor),
                cfg.image_generation.models.as_ref(),
            )
        })
        .or_else(|| provider_cfg.map(|config| config.model.as_str()))
        .unwrap_or("default")
        .to_string()
}

fn image_model_kind(cfg: &RootConfig, vendor: VendorKind, model: &str) -> &'static str {
    match vendor {
        VendorKind::Google | VendorKind::MiniMax => "native",
        VendorKind::OpenAI => "compat",
        VendorKind::Qwen => {
            if should_use_qwen_native(
                &cfg.image_generation,
                model,
                resolve_adapter_mode(&cfg.image_generation),
                cfg.image_generation.allow_compat_adapters,
            ) {
                "native"
            } else {
                "compat"
            }
        }
        VendorKind::Anthropic | VendorKind::Grok | VendorKind::DeepSeek => {
            if matches!(resolve_adapter_mode(&cfg.image_generation), AdapterMode::Native) {
                "native"
            } else {
                "compat"
            }
        }
    }
}

fn image_poll_output_path(
    cfg: &RootConfig,
    workspace_root: &Path,
    obj: &Map<String, Value>,
) -> Result<PathBuf, String> {
    resolve_output_path(
        workspace_root,
        cfg.image_generation
            .default_output_dir
            .as_deref()
            .unwrap_or("image"),
        obj.get("output_path").and_then(Value::as_str),
    )
}

fn resolved_vendor_config(cfg: &RootConfig, vendor: VendorKind) -> Option<&VendorConfig> {
    let section = &cfg.image_generation.providers;
    match vendor {
        VendorKind::OpenAI => section.openai.as_ref().or(cfg.llm.openai.as_ref()),
        VendorKind::Google => section.google.as_ref().or(cfg.llm.google.as_ref()),
        VendorKind::Anthropic => section.anthropic.as_ref().or(cfg.llm.anthropic.as_ref()),
        VendorKind::Grok => section.grok.as_ref().or(cfg.llm.grok.as_ref()),
        VendorKind::DeepSeek => section.deepseek.as_ref().or(cfg.llm.deepseek.as_ref()),
        VendorKind::Qwen => section.qwen.as_ref().or(cfg.llm.qwen.as_ref()),
        VendorKind::MiniMax => section.minimax.as_ref().or(cfg.llm.minimax.as_ref()),
    }
}

fn required_string_arg(obj: &Map<String, Value>, key: &str) -> Result<String, String> {
    obj.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("{key} is required"))
}

fn optional_bool(obj: &Map<String, Value>, key: &str) -> Option<bool> {
    obj.get(key).and_then(Value::as_bool)
}

pub(super) fn image_poll_after_seconds(obj: &Map<String, Value>) -> u64 {
    obj.get("poll_after_seconds")
        .and_then(Value::as_u64)
        .or_else(|| {
            obj.get("poll_after_ms")
                .and_then(Value::as_u64)
                .filter(|millis| *millis > 0)
                .map(|millis| millis.saturating_add(999) / 1_000)
        })
        .unwrap_or(5)
        .clamp(1, 3600)
}

pub(super) fn image_expires_at(obj: &Map<String, Value>) -> i64 {
    obj.get("expires_at")
        .and_then(Value::as_i64)
        .unwrap_or_else(|| (unix_ts() as i64).saturating_add(600))
}

pub(super) fn provider_image_job_id(provider: &str, task_id: &str) -> String {
    format!("provider:image_generate:{provider}:{task_id}")
}
