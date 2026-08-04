use std::collections::BTreeMap;
use std::io::{self, BufRead, Write};
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use skill_sdk::{
    extract_safe_archive, inspect_safe_archive, read_safe_archive_member, ArtifactSpill,
    BoundedResult, ContinuationDescriptor, ExpectedPathKind, SafeArchiveInspection,
    SafeArchiveLimits, SkillPathPolicy, SkillProgressEmitter, MAX_PROTOCOL_LINE_BYTES,
};

const SKILL_NAME: &str = "archive_basic";

#[derive(Debug)]
struct ArchiveListing {
    entries: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct Req {
    request_id: String,
    args: Value,
    #[serde(default)]
    context: Option<Value>,
}

#[derive(Debug, Serialize)]
struct Resp {
    request_id: String,
    status: String,
    text: String,
    extra: Option<Value>,
    error_text: Option<String>,
}

#[derive(Debug)]
struct SkillError {
    kind: &'static str,
    text: String,
    extra: Option<Value>,
}

impl SkillError {
    fn new(kind: &'static str, text: impl Into<String>, extra: Option<Value>) -> Self {
        Self {
            kind,
            text: text.into(),
            extra,
        }
    }

    fn invalid_input(text: impl Into<String>) -> Self {
        Self::new("invalid_input", text, None)
    }

    fn not_found(path: &Path, role: &'static str) -> Self {
        let path_text = path.display().to_string();
        Self::new(
            "not_found",
            format!("{role} not found: {path_text}"),
            Some(json!({"path": path_text, "role": role})),
        )
    }

    fn unsupported_format(text: impl Into<String>) -> Self {
        Self::new("unsupported_format", text, None)
    }

    fn command_failed(text: impl Into<String>) -> Self {
        Self::new("command_failed", text, None)
    }
}

fn main() -> anyhow::Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    for line in stdin.lock().lines() {
        let line = line?;
        let parsed: Result<Req, _> = serde_json::from_str(&line);
        let resp = match parsed {
            Ok(req) => {
                let mut progress = SkillProgressEmitter::new(&mut stdout, &req.request_id);
                progress.emit_progress(
                    "archive_basic.entries.progress",
                    BTreeMap::new(),
                    None,
                    None,
                )?;
                match execute_with_context(req.args, req.context.as_ref()) {
                    Ok((text, extra)) => {
                        if let Some(total) = extra.get("member_count").and_then(Value::as_u64) {
                            progress.emit_progress(
                                "archive_basic.entries.progress",
                                BTreeMap::new(),
                                Some(total),
                                Some(total),
                            )?;
                        }
                        Resp {
                            request_id: req.request_id,
                            status: "ok".to_string(),
                            text,
                            extra: Some(extra),
                            error_text: None,
                        }
                    }
                    Err(err) => Resp {
                        request_id: req.request_id,
                        status: "error".to_string(),
                        text: String::new(),
                        extra: Some(error_extra_with_details(err.kind, err.extra)),
                        error_text: Some(err.text),
                    },
                }
            }
            Err(err) => Resp {
                request_id: "unknown".to_string(),
                status: "error".to_string(),
                text: String::new(),
                extra: Some(error_extra("invalid_input")),
                error_text: Some(format!("invalid input: {err}")),
            },
        };
        writeln!(stdout, "{}", serde_json::to_string(&resp)?)?;
        stdout.flush()?;
    }
    Ok(())
}

fn error_extra(error_code: &str) -> Value {
    error_extra_with_details(error_code, None)
}

fn error_extra_with_details(error_code: &str, details: Option<Value>) -> Value {
    let mut extra = json!({
        "schema_version": 1,
        "source_skill": SKILL_NAME,
        "status": "error",
        "error_code": error_code,
        "message_key": format!("skill.{}.{}", SKILL_NAME, error_code),
        "retryable": false,
    });
    if let Some(details) = details {
        if let (Some(base), Some(details_obj)) = (extra.as_object_mut(), details.as_object()) {
            for (key, value) in details_obj {
                base.entry(key.clone()).or_insert_with(|| value.clone());
            }
        } else if let Some(base) = extra.as_object_mut() {
            base.insert("details".to_string(), details);
        }
    }
    extra
}

fn execute_with_context(
    args: Value,
    context: Option<&Value>,
) -> Result<(String, Value), SkillError> {
    execute_with_root_and_context(args, &workspace_root(), context)
}

fn execute_with_root_and_context(
    args: Value,
    workspace_root: &Path,
    context: Option<&Value>,
) -> Result<(String, Value), SkillError> {
    let obj = args
        .as_object()
        .ok_or_else(|| SkillError::invalid_input("args must be object"))?;
    let action = obj.get("action").and_then(|v| v.as_str()).unwrap_or("list");
    let path_policy = SkillPathPolicy::new(workspace_root, context).map_err(path_policy_error)?;
    let authority_scope = if path_policy.authority().is_unrestricted_admin() {
        "unrestricted_admin"
    } else {
        "workspace"
    };
    let inline_budget = archive_inline_budget();
    let artifact_spill =
        ArtifactSpill::from_request_context(context, SKILL_NAME).map_err(archive_sdk_error)?;

    match action {
        "list" => {
            let archive = required_str_any(obj, &["archive", "archive_path", "path"])?;
            let archive = path_policy
                .resolve_existing(archive, ExpectedPathKind::File)
                .map_err(path_policy_error)?;
            let requested_offset = list_continuation_offset(obj)?;
            list_archive(&archive).and_then(|listing| {
                if requested_offset > listing.entries.len() {
                    return Err(SkillError::invalid_input(
                        "archive list continuation is beyond the current member set",
                    ));
                }
                let archive_path = archive.display().to_string();
                let offset = requested_offset;
                let (page_members, next_offset) = archive_member_page(
                    &listing.entries,
                    offset,
                    (inline_budget / 8).max(16 * 1024),
                );
                let continuation = next_offset.map(|next_offset| ContinuationDescriptor {
                    kind: "archive_member_page".to_string(),
                    token: Some(format!("archive-members:{next_offset}")),
                    state: json!({"next_offset": next_offset}),
                });
                let bounded = BoundedResult::page(
                    page_members.clone(),
                    page_members.len() as u64,
                    listing.entries.len() as u64,
                    continuation.clone(),
                );
                let entries = page_members
                    .iter()
                    .map(|name| {
                        json!({
                            "name": name,
                            "kind": if name.ends_with('/') { "dir" } else { "file" }
                        })
                    })
                    .collect::<Vec<_>>();
                let output = format!("exit=0\n{}", page_members.join("\n"));
                let payload = json!({
                    "action": "list",
                    "authority_scope": authority_scope,
                    "archive": archive_path,
                    "count": listing.entries.len(),
                    "member_count": listing.entries.len(),
                    "members": page_members.clone(),
                    "entries": entries,
                    "candidates": page_members.clone(),
                    "output": output,
                    "complete": bounded.complete,
                    "partial_reason": bounded.partial_reason.clone(),
                    "continuation": continuation.clone(),
                    "bounded_result": bounded.clone(),
                    "field_value": {
                        "action": "list",
                        "archive": archive_path,
                        "count": listing.entries.len(),
                        "member_count": listing.entries.len(),
                        "members": page_members,
                    }
                });
                Ok((payload.to_string(), payload))
            })
        }
        "read" => {
            let archive = required_str_any(obj, &["archive", "archive_path", "path"])?;
            let member = required_str_any(obj, &["member", "entry", "file", "file_path"])?;
            let archive = path_policy
                .resolve_existing(archive, ExpectedPathKind::File)
                .map_err(path_policy_error)?;
            let member = normalize_archive_member(member)?;
            read_archive_member(&archive, &member, artifact_spill.as_ref(), inline_budget).map(
                |bounded| {
                    let content_excerpt = content_excerpt_for_machine_field(&bounded.value);
                    let payload = json!({
                        "action":"read",
                        "authority_scope":authority_scope,
                        "archive":archive.display().to_string(),
                        "path":member,
                        "member":member,
                        "member_path":member,
                        "content":bounded.value.clone(),
                        "content_excerpt":content_excerpt,
                        "complete":bounded.complete,
                        "partial_reason":bounded.partial_reason.clone(),
                        "continuation":bounded.continuation.clone(),
                        "artifacts":bounded.artifacts.clone(),
                        "bounded_result":bounded.clone(),
                        "field_value": {
                            "action": "read",
                            "archive": archive.display().to_string(),
                            "path": member,
                            "member": member,
                            "member_path": member,
                            "content_excerpt": content_excerpt,
                        }
                    });
                    (payload.to_string(), payload)
                },
            )
        }
        "pack" => {
            let format = obj.get("format").and_then(|v| v.as_str()).unwrap_or("zip");
            let source = path_policy
                .resolve_existing(
                    required_str_any(obj, &["source", "source_path"])?,
                    ExpectedPathKind::Any,
                )
                .map_err(path_policy_error)?;
            let archive = path_policy
                .resolve_create_target(required_str_any(obj, &["archive", "archive_path"])?)
                .map_err(path_policy_error)?;
            pack_archive(format, &source, &archive).map(|text| {
                let archive_path = archive.display().to_string();
                let source_path = source.display().to_string();
                (
                    format!("archive_path={archive_path}\n{text}"),
                    json!({
                        "action":"pack",
                        "authority_scope":authority_scope,
                        "format":format,
                        "source":source_path,
                        "archive":archive_path,
                        "output":text,
                        "field_value": {
                            "archive": archive_path,
                            "format": format,
                            "source": source_path,
                        },
                        "artifacts": [{
                            "path": archive_path,
                            "metadata": {
                                "action": "pack",
                                "format": format,
                                "source": source_path,
                            }
                        }]
                    }),
                )
            })
        }
        "unpack" => {
            let archive = path_policy
                .resolve_existing(
                    required_str_any(obj, &["archive", "archive_path", "path"])?,
                    ExpectedPathKind::File,
                )
                .map_err(path_policy_error)?;
            let dest = path_policy
                .resolve_create_target(required_str_any(obj, &["dest", "dest_path"])?)
                .map_err(path_policy_error)?;
            unpack_archive(&archive, &dest).map(|inspection| {
                let dest_path = dest.display().to_string();
                let text = format!(
                    "archive extracted safely: format={} entries={} expanded_bytes={}",
                    inspection.format, inspection.entry_count, inspection.expanded_bytes
                );
                (
                    format!("dest_path={dest_path}\n{text}"),
                    json!({
                        "action":"unpack",
                        "authority_scope":authority_scope,
                        "archive":archive.display().to_string(),
                        "dest":dest_path,
                        "format":inspection.format,
                        "member_count":inspection.entry_count,
                        "expanded_bytes":inspection.expanded_bytes,
                        "preflight_verified":true,
                        "atomic_promotion":true,
                        "output":text,
                        "field_value": {
                            "dest": dest_path,
                        }
                    }),
                )
            })
        }
        _ => Err(SkillError::invalid_input(
            "unsupported action; use list|read|pack|unpack",
        )),
    }
}

fn path_policy_error(error: skill_sdk::SkillSdkError) -> SkillError {
    let kind = match error.code.as_str() {
        "path_outside_workspace" => "path_outside_workspace",
        "path_traversal_forbidden" => "path_traversal_forbidden",
        "path_not_found" => "not_found",
        "path_kind_mismatch" => "path_kind_mismatch",
        "path_target_symlink_forbidden" => "path_target_symlink_forbidden",
        _ => "invalid_input",
    };
    SkillError::new(
        kind,
        error.detail,
        Some(json!({
            "sdk_error_code": error.code,
            "sdk_message_key": error.message_key,
        })),
    )
}

fn content_excerpt_for_machine_field(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default()
        .chars()
        .take(240)
        .collect()
}

fn archive_inline_budget() -> usize {
    std::env::var("SKILL_RESULT_INLINE_BYTES")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .map(|value| value.clamp(64 * 1024, MAX_PROTOCOL_LINE_BYTES / 2))
        .unwrap_or(MAX_PROTOCOL_LINE_BYTES / 2)
}

fn list_continuation_offset(obj: &serde_json::Map<String, Value>) -> Result<usize, SkillError> {
    let Some(token) = obj.get("continuation").and_then(Value::as_str) else {
        return Ok(0);
    };
    token
        .trim()
        .strip_prefix("archive-members:")
        .and_then(|value| value.parse::<usize>().ok())
        .ok_or_else(|| SkillError::invalid_input("invalid archive list continuation"))
}

fn archive_member_page(
    entries: &[String],
    offset: usize,
    byte_budget: usize,
) -> (Vec<String>, Option<usize>) {
    let mut page = Vec::new();
    let mut used = 0_usize;
    for entry in entries.iter().skip(offset) {
        let cost = entry.len().saturating_add(32);
        if !page.is_empty() && used.saturating_add(cost) > byte_budget {
            break;
        }
        used = used.saturating_add(cost);
        page.push(entry.clone());
    }
    let next = offset.saturating_add(page.len());
    (page, (next < entries.len()).then_some(next))
}

fn list_archive(archive: &Path) -> Result<ArchiveListing, SkillError> {
    if !archive.is_file() {
        return Err(SkillError::not_found(archive, "archive"));
    }
    let parent = archive.parent().ok_or_else(|| {
        SkillError::invalid_input(format!("archive has no parent: {}", archive.display()))
    })?;
    let limits = SafeArchiveLimits::adaptive_for(archive, parent).map_err(archive_sdk_error)?;
    let inspection = inspect_safe_archive(archive, limits).map_err(archive_sdk_error)?;
    let entries = inspection
        .entries
        .into_iter()
        .map(|entry| entry.path)
        .collect::<Vec<_>>();
    Ok(ArchiveListing { entries })
}

fn read_archive_member(
    archive: &Path,
    member: &str,
    spill: Option<&ArtifactSpill>,
    inline_bytes: usize,
) -> Result<BoundedResult<String>, SkillError> {
    if !archive.is_file() {
        return Err(SkillError::not_found(archive, "archive"));
    }
    let parent = archive.parent().ok_or_else(|| {
        SkillError::invalid_input(format!("archive has no parent: {}", archive.display()))
    })?;
    let limits = SafeArchiveLimits::adaptive_for(archive, parent).map_err(archive_sdk_error)?;
    read_safe_archive_member(archive, member, limits, inline_bytes, spill)
        .map_err(archive_sdk_error)
}

fn pack_archive(format: &str, source: &Path, archive: &Path) -> Result<String, SkillError> {
    if !source.exists() {
        return Err(SkillError::not_found(source, "source"));
    }
    let src = source.to_string_lossy().to_string();
    let out = archive.to_string_lossy().to_string();
    if let Some(parent) = archive.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| SkillError::command_failed(format!("mkdir failed: {err}")))?;
    }

    match format {
        "zip" => run("zip", &[String::from("-q"), String::from("-r"), out, src]),
        "tar.gz" | "tgz" => run("tar", &[String::from("-czf"), out, src]),
        _ => Err(SkillError::unsupported_format(
            "unsupported format; use zip|tar.gz",
        )),
    }
}

fn unpack_archive(archive: &Path, dest: &Path) -> Result<SafeArchiveInspection, SkillError> {
    if !archive.is_file() {
        return Err(SkillError::not_found(archive, "archive"));
    }
    if dest.exists() {
        return Err(SkillError::new(
            "destination_exists",
            format!("destination already exists: {}", dest.display()),
            Some(json!({"dest": dest.display().to_string()})),
        ));
    }
    let parent = dest.parent().ok_or_else(|| {
        SkillError::invalid_input(format!("destination has no parent: {}", dest.display()))
    })?;
    std::fs::create_dir_all(parent)
        .map_err(|error| SkillError::command_failed(format!("mkdir failed: {error}")))?;
    let canonical_parent = parent
        .canonicalize()
        .map_err(|error| SkillError::command_failed(format!("canonicalize failed: {error}")))?;
    let limits =
        SafeArchiveLimits::adaptive_for(archive, &canonical_parent).map_err(archive_sdk_error)?;
    let temporary = tempfile::Builder::new()
        .prefix(".agent-archive-")
        .tempdir_in(&canonical_parent)
        .map_err(|error| SkillError::command_failed(format!("tempdir failed: {error}")))?;
    let staging = temporary.path().join("payload");
    let inspection = extract_safe_archive(archive, &staging, limits).map_err(archive_sdk_error)?;
    std::fs::rename(&staging, dest)
        .map_err(|error| SkillError::command_failed(format!("atomic promote failed: {error}")))?;
    Ok(inspection)
}

fn archive_sdk_error(error: skill_sdk::SkillSdkError) -> SkillError {
    let kind = match error.code.as_str() {
        "archive_format_unsupported" => "unsupported_format",
        "archive_path_unsafe" => "archive_path_unsafe",
        "archive_entry_type_forbidden" => "archive_entry_type_forbidden",
        "archive_budget_exceeded" => "archive_budget_exceeded",
        "archive_destination_exists" => "destination_exists",
        _ => "archive_invalid",
    };
    SkillError::new(
        kind,
        error.detail,
        Some(json!({
            "sdk_error_code": error.code,
            "sdk_message_key": error.message_key,
            "phase": error.phase,
        })),
    )
}

fn run(bin: &str, args: &[String]) -> Result<String, SkillError> {
    let output = Command::new(bin)
        .args(args)
        .output()
        .map_err(|err| SkillError::command_failed(format!("run {bin} failed: {err}")))?;
    let text = format_command_output(&output.stdout, &output.stderr);
    let exit_code = output.status.code().unwrap_or(-1);
    if output.status.success() {
        Ok(format!("exit={exit_code}\n{text}"))
    } else {
        Err(SkillError::command_failed(format!(
            "archive command failed: exit={exit_code}\n{text}"
        )))
    }
}

fn format_command_output(stdout: &[u8], stderr: &[u8]) -> String {
    let mut text = String::new();
    text.push_str(&String::from_utf8_lossy(stdout));
    if !stderr.is_empty() {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&String::from_utf8_lossy(stderr));
    }
    text
}

fn normalize_archive_member(input: &str) -> Result<String, SkillError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(SkillError::invalid_input("member is required"));
    }
    let raw = Path::new(trimmed);
    if raw.is_absolute() {
        return Err(SkillError::invalid_input(
            "archive member must be a relative path",
        ));
    }
    let mut parts = Vec::new();
    for comp in raw.components() {
        match comp {
            Component::ParentDir => {
                return Err(SkillError::invalid_input(
                    "archive member with '..' is not allowed",
                ));
            }
            Component::CurDir => {}
            Component::Normal(part) => parts.push(part.to_string_lossy().to_string()),
            Component::RootDir | Component::Prefix(_) => {
                return Err(SkillError::invalid_input(
                    "archive member must be a relative path",
                ));
            }
        }
    }
    if parts.is_empty() {
        return Err(SkillError::invalid_input("member is required"));
    }
    Ok(parts.join("/"))
}

fn required_str_any<'a>(
    obj: &'a serde_json::Map<String, Value>,
    keys: &[&str],
) -> Result<&'a str, SkillError> {
    for key in keys {
        if let Some(value) = obj
            .get(*key)
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Ok(value);
        }
    }
    Err(SkillError::invalid_input(format!(
        "{} is required",
        keys.first().copied().unwrap_or("value")
    )))
}

fn workspace_root() -> PathBuf {
    std::env::var("WORKSPACE_ROOT")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
