use std::path::Path;

use serde_json::{json, Value};

use super::async_projection::image_pending_async_job_contract;
use super::i18n::TextCatalog;

pub(super) fn build_success_response(
    i18n: &TextCatalog,
    output_path: &Path,
    provider: &str,
    model: &str,
    model_kind: &str,
    fallback: Option<Value>,
) -> (String, Value) {
    let saved_path = output_path.to_string_lossy().to_string();
    let preface = i18n.render(
        "image_generate.msg.saved",
        &[("path", saved_path.clone())],
        "Generated successfully and saved: {path}",
    );
    let text = format!("{preface}\nFILE:{saved_path}\nEPHEMERAL:IMAGE_SAVED");
    let mut extra = json!({
        "message_key": "image_generate.msg.saved",
        "provider": provider,
        "model": model,
        "model_kind": model_kind,
        "latency_ms": 0,
        "media_type": "image",
        "output_path": saved_path.clone(),
        "outputs": [{"type":"image_file","path": saved_path}]
    });
    if let Some(fallback) = fallback {
        extra["fallback"] = fallback;
    }
    (text, extra)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn build_dry_run_response(
    action: &str,
    output_path: &Path,
    provider: &str,
    model: &str,
    prompt: &str,
    size: &str,
    style: Option<&str>,
    quality: Option<&str>,
    n: u64,
    poll_after_seconds: u64,
    expires_at: i64,
    job_id: &str,
) -> (String, Value) {
    let saved_path = output_path.to_string_lossy().to_string();
    let planned_outputs = json!([{"type":"image_file","path": saved_path}]);
    let async_contract = image_pending_async_job_contract(
        provider,
        model,
        job_id,
        "dry_run",
        &saved_path,
        poll_after_seconds,
        expires_at,
    );
    let mut request = json!({
        "prompt_chars": prompt.chars().count(),
        "size": size,
        "n": n,
        "output_path": saved_path,
    });
    if let Some(style) = style {
        request["style"] = json!(style);
    }
    if let Some(quality) = quality {
        request["quality"] = json!(quality);
    }
    (
        "IMAGE_GENERATE_DRY_RUN".to_string(),
        json!({
            "schema_version": 1,
            "action": action,
            "status": "dry_run",
            "message_key": "image_generate.msg.dry_run",
            "dry_run": true,
            "would_mutate": false,
            "provider": provider,
            "model": model,
            "model_kind": "dry_run",
            "latency_ms": 0,
            "media_type": "image",
            "output_path": saved_path,
            "outputs": [],
            "planned_outputs": planned_outputs,
            "pending_async_job_contract": async_contract,
            "async_contract": async_contract,
            "request": request,
            "field_value": {
                "action": action,
                "status": "dry_run",
                "message_key": "image_generate.msg.dry_run",
                "dry_run": true,
                "would_mutate": false,
                "provider": provider,
                "model": model,
                "output_path": saved_path,
                "planned_outputs": planned_outputs,
                "async_contract": async_contract,
            },
        }),
    )
}
