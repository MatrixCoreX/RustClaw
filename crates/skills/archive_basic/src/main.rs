use std::io::{self, BufRead, Write};
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const SKILL_NAME: &str = "archive_basic";

#[derive(Debug)]
struct ArchiveListing {
    output: String,
    entries: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct Req {
    request_id: String,
    args: Value,
}

#[derive(Debug, Serialize)]
struct Resp {
    request_id: String,
    status: String,
    text: String,
    extra: Option<Value>,
    error_text: Option<String>,
    error_kind: Option<String>,
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
            Ok(req) => match execute(req.args) {
                Ok((text, extra)) => Resp {
                    request_id: req.request_id,
                    status: "ok".to_string(),
                    text,
                    extra: Some(extra),
                    error_text: None,
                    error_kind: None,
                },
                Err(err) => Resp {
                    request_id: req.request_id,
                    status: "error".to_string(),
                    text: String::new(),
                    extra: Some(error_extra_with_details(err.kind, err.extra)),
                    error_text: Some(err.text),
                    error_kind: Some(err.kind.to_string()),
                },
            },
            Err(err) => Resp {
                request_id: "unknown".to_string(),
                status: "error".to_string(),
                text: String::new(),
                extra: Some(error_extra("invalid_input")),
                error_text: Some(format!("invalid input: {err}")),
                error_kind: Some("invalid_input".to_string()),
            },
        };
        writeln!(stdout, "{}", serde_json::to_string(&resp)?)?;
        stdout.flush()?;
    }
    Ok(())
}

fn error_extra(error_kind: &str) -> Value {
    error_extra_with_details(error_kind, None)
}

fn error_extra_with_details(error_kind: &str, details: Option<Value>) -> Value {
    let mut extra = json!({
        "schema_version": 1,
        "source_skill": SKILL_NAME,
        "status": "error",
        "error_kind": error_kind,
        "message_key": format!("skill.{}.{}", SKILL_NAME, error_kind),
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

fn execute(args: Value) -> Result<(String, Value), SkillError> {
    let obj = args
        .as_object()
        .ok_or_else(|| SkillError::invalid_input("args must be object"))?;
    let action = obj.get("action").and_then(|v| v.as_str()).unwrap_or("list");
    let root = workspace_root();

    match action {
        "list" => {
            let archive = required_str_any(obj, &["archive", "archive_path", "path"])?;
            let archive = resolve_path(&root, archive, false)?;
            list_archive(&archive).map(|listing| {
                let archive_path = archive.display().to_string();
                let entries = listing
                    .entries
                    .iter()
                    .map(|name| {
                        json!({
                            "name": name,
                            "kind": if name.ends_with('/') { "dir" } else { "file" }
                        })
                    })
                    .collect::<Vec<_>>();
                let candidates = listing.entries.clone();
                let payload = json!({
                    "action": "list",
                    "archive": archive_path,
                    "count": candidates.len(),
                    "member_count": candidates.len(),
                    "members": candidates,
                    "entries": entries,
                    "candidates": listing.entries.clone(),
                    "output": listing.output,
                    "field_value": {
                        "action": "list",
                        "archive": archive_path,
                        "count": listing.entries.len(),
                        "member_count": listing.entries.len(),
                        "members": listing.entries,
                    }
                });
                (payload.to_string(), payload)
            })
        }
        "read" => {
            let archive = required_str_any(obj, &["archive", "archive_path", "path"])?;
            let member = required_str_any(obj, &["member", "entry", "file", "file_path"])?;
            let archive = resolve_path(&root, archive, false)?;
            let member = normalize_archive_member(member)?;
            read_archive_member(&archive, &member).map(|text| {
                let content_excerpt = content_excerpt_for_machine_field(&text);
                let payload = json!({
                    "action":"read",
                    "archive":archive.display().to_string(),
                    "path":member,
                    "member":member,
                    "member_path":member,
                    "content":text,
                    "content_excerpt":content_excerpt,
                });
                (
                    payload.to_string(),
                    json!({
                        "action":"read",
                        "archive":archive.display().to_string(),
                        "path":payload.get("path").and_then(Value::as_str).unwrap_or_default(),
                        "member":payload.get("member").and_then(Value::as_str).unwrap_or_default(),
                        "member_path":payload.get("member_path").and_then(Value::as_str).unwrap_or_default(),
                        "content":payload.get("content").and_then(Value::as_str).unwrap_or_default(),
                        "content_excerpt":payload.get("content_excerpt").and_then(Value::as_str).unwrap_or_default(),
                        "field_value": {
                            "action": "read",
                            "archive": archive.display().to_string(),
                            "path": payload.get("path").and_then(Value::as_str).unwrap_or_default(),
                            "member": payload.get("member").and_then(Value::as_str).unwrap_or_default(),
                            "member_path": payload.get("member_path").and_then(Value::as_str).unwrap_or_default(),
                            "content_excerpt": payload.get("content_excerpt").and_then(Value::as_str).unwrap_or_default(),
                        }
                    }),
                )
            })
        }
        "pack" => {
            let format = obj.get("format").and_then(|v| v.as_str()).unwrap_or("zip");
            let source = resolve_path(
                &root,
                required_str_any(obj, &["source", "source_path"])?,
                false,
            )?;
            let archive = resolve_path(
                &root,
                required_str_any(obj, &["archive", "archive_path"])?,
                true,
            )?;
            pack_archive(format, &source, &archive).map(|text| {
                let archive_path = archive.display().to_string();
                (
                    format!("archive_path={archive_path}\n{text}"),
                    json!({
                        "action":"pack",
                        "format":format,
                        "source":source.display().to_string(),
                        "archive":archive_path,
                        "output":text
                    }),
                )
            })
        }
        "unpack" => {
            let archive = resolve_path(
                &root,
                required_str_any(obj, &["archive", "archive_path", "path"])?,
                false,
            )?;
            let dest = resolve_path(&root, required_str_any(obj, &["dest", "dest_path"])?, true)?;
            unpack_archive(&archive, &dest).map(|text| {
                let dest_path = dest.display().to_string();
                (
                    format!("dest_path={dest_path}\n{text}"),
                    json!({
                        "action":"unpack",
                        "archive":archive.display().to_string(),
                        "dest":dest_path,
                        "output":text
                    }),
                )
            })
        }
        _ => Err(SkillError::invalid_input(
            "unsupported action; use list|read|pack|unpack",
        )),
    }
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

fn list_archive(archive: &Path) -> Result<ArchiveListing, SkillError> {
    if !archive.is_file() {
        return Err(SkillError::not_found(archive, "archive"));
    }
    let name = archive.to_string_lossy().to_string();
    let raw_entries = if name.ends_with(".zip") {
        run_raw_stdout("unzip", &[String::from("-Z1"), name])?
    } else if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        run_raw_stdout("tar", &[String::from("-tzf"), name])?
    } else {
        return Err(SkillError::unsupported_format(
            "unsupported archive format for list",
        ));
    };
    let entries = parse_archive_member_listing(&raw_entries);
    let output = format!("exit=0\n{}", entries.join("\n"));
    Ok(ArchiveListing { output, entries })
}

fn parse_archive_member_listing(output: &str) -> Vec<String> {
    output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn read_archive_member(archive: &Path, member: &str) -> Result<String, SkillError> {
    if !archive.is_file() {
        return Err(SkillError::not_found(archive, "archive"));
    }
    let name = archive.to_string_lossy().to_string();
    if name.ends_with(".zip") {
        run_raw_stdout("unzip", &[String::from("-p"), name, member.to_string()])
    } else if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        run_raw_stdout("tar", &[String::from("-xOzf"), name, member.to_string()])
    } else {
        Err(SkillError::unsupported_format(
            "unsupported archive format for read",
        ))
    }
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
        "zip" => run("zip", &[String::from("-r"), out, src]),
        "tar.gz" | "tgz" => run("tar", &[String::from("-czf"), out, src]),
        _ => Err(SkillError::unsupported_format(
            "unsupported format; use zip|tar.gz",
        )),
    }
}

fn unpack_archive(archive: &Path, dest: &Path) -> Result<String, SkillError> {
    if !archive.is_file() {
        return Err(SkillError::not_found(archive, "archive"));
    }
    std::fs::create_dir_all(dest)
        .map_err(|err| SkillError::command_failed(format!("mkdir failed: {err}")))?;
    let arc = archive.to_string_lossy().to_string();
    let dst = dest.to_string_lossy().to_string();
    if arc.ends_with(".zip") {
        // Non-interactive default: overwrite existing files to avoid hanging on prompts.
        run("unzip", &[String::from("-o"), arc, String::from("-d"), dst])
    } else if arc.ends_with(".tar.gz") || arc.ends_with(".tgz") {
        // Avoid GNU-only flags so both bsdtar (macOS) and GNU tar work.
        run("tar", &[String::from("-xzf"), arc, String::from("-C"), dst])
    } else {
        Err(SkillError::unsupported_format(
            "unsupported archive format for unpack",
        ))
    }
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

fn run_raw_stdout(bin: &str, args: &[String]) -> Result<String, SkillError> {
    let output = Command::new(bin)
        .args(args)
        .output()
        .map_err(|err| SkillError::command_failed(format!("run {bin} failed: {err}")))?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let exit_code = output.status.code().unwrap_or(-1);
    if output.status.success() {
        Ok(truncate_output(stdout))
    } else {
        let text = format_command_output(&output.stdout, &output.stderr);
        Err(SkillError::command_failed(format!(
            "archive command failed: exit={exit_code}\n{text}"
        )))
    }
    .map(|text| {
        if text.is_empty() && !stderr.trim().is_empty() {
            truncate_output(stderr)
        } else {
            text
        }
    })
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
    truncate_output(text)
}

fn truncate_output(mut text: String) -> String {
    if text.len() > 10000 {
        text.truncate(10000);
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

fn resolve_path(
    workspace_root: &Path,
    input: &str,
    allow_absolute: bool,
) -> Result<PathBuf, SkillError> {
    let raw = Path::new(input);
    let mut normalized = PathBuf::new();
    for comp in raw.components() {
        match comp {
            Component::ParentDir => {
                return Err(SkillError::invalid_input("path with '..' is not allowed"));
            }
            Component::CurDir => {}
            other => normalized.push(other.as_os_str()),
        }
    }
    if raw.is_absolute() {
        if !allow_absolute {
            return Ok(normalized);
        }
        return Ok(normalized);
    }
    Ok(workspace_root.join(normalized))
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
