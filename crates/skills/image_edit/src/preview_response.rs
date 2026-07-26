use std::path::Path;

use serde_json::{json, Value};

use super::ImageSource;

fn source_projection(source: &ImageSource) -> Value {
    match source {
        ImageSource::Path(path) => json!({
            "kind": "path",
            "path": path.to_string_lossy(),
        }),
        ImageSource::Url(url) => json!({
            "kind": "url",
            "url": url,
        }),
        ImageSource::Base64(_) => json!({
            "kind": "base64",
            "present": true,
        }),
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn build_preview_response(
    output_path: &Path,
    provider: &str,
    model: &str,
    instruction: &str,
    image_source: &ImageSource,
    has_mask: bool,
    size: &str,
    quality: Option<&str>,
    n: u64,
) -> (String, Value) {
    let output_path = output_path.to_string_lossy().to_string();
    let planned_outputs = json!([{
        "type": "image_file",
        "path": output_path,
    }]);
    let source = source_projection(image_source);
    let mut request = json!({
        "instruction_chars": instruction.chars().count(),
        "source": source,
        "has_mask": has_mask,
        "size": size,
        "n": n,
        "output_path": output_path,
    });
    if let Some(quality) = quality {
        request["quality"] = json!(quality);
    }
    (
        "IMAGE_EDIT_DRY_RUN".to_string(),
        json!({
            "schema_version": 1,
            "action": "preview_edit",
            "status": "dry_run",
            "message_key": "image_edit.msg.dry_run",
            "dry_run": true,
            "would_mutate": false,
            "provider": provider,
            "model": model,
            "model_kind": "dry_run",
            "latency_ms": 0,
            "media_type": "image",
            "output_path": output_path,
            "outputs": [],
            "planned_outputs": planned_outputs,
            "request": request,
            "field_value": {
                "action": "preview_edit",
                "status": "dry_run",
                "message_key": "image_edit.msg.dry_run",
                "dry_run": true,
                "would_mutate": false,
                "provider": provider,
                "model": model,
                "output_path": output_path,
                "planned_outputs": planned_outputs,
                "source": source,
            },
        }),
    )
}
