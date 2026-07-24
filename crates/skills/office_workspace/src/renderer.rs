use crate::error::{OfficeError, OfficeResult};
use crate::package::{hash_bytes, resolve_input_path, OfficePackage};
use crate::package_write::resolve_output_path;
use serde_json::{json, Map, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use uuid::Uuid;

#[derive(Clone, Debug)]
struct Renderer {
    backend: &'static str,
    executable: PathBuf,
}

pub fn execute(action: &str, object: &Map<String, Value>) -> OfficeResult<Value> {
    let renderer = detect_renderer();
    if action == "office.render_status" {
        return Ok(match renderer {
            Some(renderer) => json!({
                "schema_version": 1,
                "available": true,
                "backend": renderer.backend,
                "executable": renderer.executable.display().to_string(),
                "platform": std::env::consts::OS,
                "structural_office_support_independent": true,
            }),
            None => json!({
                "schema_version": 1,
                "available": false,
                "error_code": "renderer_unavailable",
                "platform": std::env::consts::OS,
                "structural_office_support_independent": true,
            }),
        });
    }
    let renderer = renderer.ok_or_else(|| {
        OfficeError::new(
            "renderer_unavailable",
            "no supported Office renderer is available",
            json!({"platform": std::env::consts::OS}),
        )
    })?;
    render(&renderer, object)
}

fn render(renderer: &Renderer, object: &Map<String, Value>) -> OfficeResult<Value> {
    let source = resolve_input_path(required_string(object, "path")?)?;
    let source_package = OfficePackage::open(&source, None)?;
    let output = resolve_output_path(required_string(object, "output_path")?)?;
    let format = object
        .get("format")
        .and_then(Value::as_str)
        .or_else(|| output.extension().and_then(|value| value.to_str()))
        .unwrap_or("pdf")
        .to_ascii_lowercase();
    if format != "pdf" {
        return Err(OfficeError::unsupported(
            "the portable Office renderer currently supports PDF output",
            json!({"format": format}),
        ));
    }
    if output.exists()
        && !object
            .get("overwrite")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    {
        return Err(OfficeError::new(
            "output_exists",
            "render output already exists and overwrite was not approved",
            json!({"output_path": output.display().to_string()}),
        ));
    }
    let parent = output.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|error| {
        OfficeError::new(
            "render_failed",
            format!("cannot create render output directory: {error}"),
            json!({"directory": parent.display().to_string()}),
        )
    })?;
    let temp_dir = parent.join(format!(".rustclaw-render-{}", Uuid::new_v4()));
    fs::create_dir_all(&temp_dir).map_err(|error| {
        OfficeError::new(
            "render_failed",
            format!("cannot create render workspace: {error}"),
            json!({"directory": temp_dir.display().to_string()}),
        )
    })?;
    let command_output = Command::new(&renderer.executable)
        .arg("--headless")
        .arg("--convert-to")
        .arg(&format)
        .arg("--outdir")
        .arg(&temp_dir)
        .arg(&source)
        .output()
        .map_err(|error| {
            fs::remove_dir_all(&temp_dir).ok();
            OfficeError::new(
                "render_failed",
                format!("cannot start Office renderer: {error}"),
                json!({"backend": renderer.backend}),
            )
        })?;
    if !command_output.status.success() {
        fs::remove_dir_all(&temp_dir).ok();
        return Err(OfficeError::new(
            "render_failed",
            "Office renderer returned a non-success status",
            json!({
                "backend": renderer.backend,
                "exit_code": command_output.status.code(),
                "stderr": bounded_text(&command_output.stderr),
                "stdout": bounded_text(&command_output.stdout),
            }),
        ));
    }
    let converted = temp_dir.join(format!(
        "{}.{format}",
        source
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("office-output")
    ));
    if !converted.is_file() {
        fs::remove_dir_all(&temp_dir).ok();
        return Err(OfficeError::new(
            "render_failed",
            "Office renderer did not produce the expected output artifact",
            json!({
                "backend": renderer.backend,
                "expected_path": converted.display().to_string(),
                "stdout": bounded_text(&command_output.stdout),
            }),
        ));
    }
    if output.exists() {
        fs::remove_file(&output).map_err(|error| {
            OfficeError::new(
                "render_failed",
                format!("cannot replace render output: {error}"),
                json!({"output_path": output.display().to_string()}),
            )
        })?;
    }
    fs::rename(&converted, &output).map_err(|error| {
        OfficeError::new(
            "render_failed",
            format!("cannot publish render output: {error}"),
            json!({"output_path": output.display().to_string()}),
        )
    })?;
    fs::remove_dir_all(&temp_dir).ok();
    let bytes = fs::read(&output).map_err(|error| {
        OfficeError::new(
            "render_failed",
            format!("cannot verify render output: {error}"),
            json!({"output_path": output.display().to_string()}),
        )
    })?;
    Ok(json!({
        "schema_version": 1,
        "status": "ok",
        "backend": renderer.backend,
        "source": {
            "path": source.display().to_string(),
            "sha256": source_package.source.sha256,
            "format": source_package.format,
        },
        "output": {
            "path": output.display().to_string(),
            "sha256": hash_bytes(&bytes),
            "size_bytes": bytes.len(),
            "format": format,
        },
        "validation": {
            "valid": true,
            "checks": ["source_package_safe", "renderer_exit_success", "output_artifact_present"],
            "visual_fidelity_claimed": false,
        },
        "artifacts": [{
            "kind": "office_render",
            "path": output.display().to_string(),
            "sha256": hash_bytes(&bytes),
            "size_bytes": bytes.len(),
        }]
    }))
}

fn detect_renderer() -> Option<Renderer> {
    match std::env::consts::OS {
        "linux" => find_in_path(&["libreoffice", "soffice"]).map(|executable| Renderer {
            backend: "libreoffice",
            executable,
        }),
        "macos" => {
            let application = PathBuf::from("/Applications/LibreOffice.app/Contents/MacOS/soffice");
            if application.is_file() {
                Some(Renderer {
                    backend: "libreoffice_macos",
                    executable: application,
                })
            } else {
                find_in_path(&["soffice", "libreoffice"]).map(|executable| Renderer {
                    backend: "libreoffice_macos",
                    executable,
                })
            }
        }
        _ => None,
    }
}

fn find_in_path(names: &[&str]) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for directory in std::env::split_paths(&path) {
        for name in names {
            let candidate = directory.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

fn required_string<'a>(object: &'a Map<String, Value>, key: &str) -> OfficeResult<&'a str> {
    object
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            OfficeError::new(
                "missing_argument",
                "required string argument is missing",
                json!({"argument": key}),
            )
        })
}

fn bounded_text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).chars().take(2_000).collect()
}

#[cfg(test)]
#[path = "renderer_tests.rs"]
mod tests;
