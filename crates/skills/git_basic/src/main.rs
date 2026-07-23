use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use toml::Value as TomlValue;

static I18N: OnceLock<TextCatalog> = OnceLock::new();
const SKILL_NAME: &str = "git_basic";
const DEFAULT_PAGE_LIMIT: usize = 20;
const MAX_PAGE_LIMIT: usize = 200;
const MAX_REVISION_BYTES: usize = 512;
const MAX_REPO_PATH_BYTES: usize = 4096;

#[derive(Debug)]
struct GitBasicError {
    code: &'static str,
    detail: String,
    extra: Option<Value>,
}

impl GitBasicError {
    fn new(code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
            extra: None,
        }
    }

    fn with_extra(mut self, extra: Value) -> Self {
        self.extra = Some(extra);
        self
    }
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
}

#[derive(Debug, Deserialize, Default)]
struct GitBasicConfig {
    #[serde(default)]
    git_basic: GitBasicSection,
}

#[derive(Debug, Deserialize, Default)]
struct GitBasicSection {
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    i18n_path: Option<String>,
}

#[derive(Debug, Clone)]
struct TextCatalog {
    current: HashMap<String, String>,
}

fn tr(key: &str) -> String {
    I18N.get()
        .and_then(|c| c.current.get(key))
        .cloned()
        .unwrap_or_else(|| key.to_string())
}

fn tr_with(key: &str, vars: &[(&str, &str)]) -> String {
    let mut out = tr(key);
    for (name, value) in vars {
        out = out.replace(&format!("{{{name}}}"), value);
    }
    out
}

fn default_catalog(lang: &str) -> TextCatalog {
    let mut current = HashMap::new();
    let _ = lang;
    current.insert(
        "git_basic.err.invalid_input".to_string(),
        "invalid input: {error}".to_string(),
    );
    current.insert(
        "git_basic.err.args_object".to_string(),
        "args must be object".to_string(),
    );
    current.insert(
        "git_basic.msg.not_git_repo".to_string(),
        "current directory is not a git repository. Please use git_basic in a git repo."
            .to_string(),
    );
    current.insert(
        "git_basic.err.unsupported_action".to_string(),
        "unsupported action; use status|log|diff|branch|show|rev_parse|diff_cached|current_branch|remote|changed_files|show_file_at_rev".to_string(),
    );
    current.insert(
        "git_basic.err.run_git_failed".to_string(),
        "run git failed: {error}".to_string(),
    );
    TextCatalog { current }
}

fn load_external_i18n(path: &Path) -> Option<HashMap<String, String>> {
    let raw = std::fs::read_to_string(path).ok()?;
    let value: TomlValue = toml::from_str(&raw).ok()?;
    let dict = value.get("dict")?.as_table()?;
    let mut out = HashMap::new();
    for (k, v) in dict {
        if let Some(text) = v.as_str() {
            out.insert(k.to_string(), text.to_string());
        }
    }
    Some(out)
}

fn load_git_basic_config(workspace_root: &Path) -> GitBasicConfig {
    let path = workspace_root.join("configs/git_basic.toml");
    let raw = match std::fs::read_to_string(path) {
        Ok(v) => v,
        Err(_) => return GitBasicConfig::default(),
    };
    toml::from_str::<GitBasicConfig>(&raw).unwrap_or_default()
}

fn init_i18n(workspace_root: &Path) {
    let cfg = load_git_basic_config(workspace_root);
    let lang = cfg
        .git_basic
        .language
        .as_deref()
        .unwrap_or("zh-CN")
        .trim()
        .to_string();
    let mut catalog = default_catalog(&lang);
    let path = cfg
        .git_basic
        .i18n_path
        .as_deref()
        .map(|p| workspace_root.join(p))
        .unwrap_or_else(|| workspace_root.join(format!("configs/i18n/git_basic.{lang}.toml")));
    if let Some(overrides) = load_external_i18n(&path) {
        for (k, v) in overrides {
            catalog.current.insert(k, v);
        }
    }
    let _ = I18N.set(catalog);
}

fn main() -> anyhow::Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let workspace_root = std::env::var("WORKSPACE_ROOT")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    init_i18n(&workspace_root);

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
                },
                Err(err) => Resp {
                    request_id: req.request_id,
                    status: "error".to_string(),
                    text: String::new(),
                    extra: Some(error_extra_with_detail(err.code, err.extra)),
                    error_text: Some(err.detail),
                },
            },
            Err(err) => Resp {
                request_id: "unknown".to_string(),
                status: "error".to_string(),
                text: String::new(),
                extra: Some(error_extra("invalid_input")),
                error_text: Some(tr_with(
                    "git_basic.err.invalid_input",
                    &[("error", &err.to_string())],
                )),
            },
        };
        writeln!(stdout, "{}", serde_json::to_string(&resp)?)?;
        stdout.flush()?;
    }
    Ok(())
}

fn error_extra(error_kind: &str) -> Value {
    error_extra_with_detail(error_kind, None)
}

fn error_extra_with_detail(error_kind: &str, detail: Option<Value>) -> Value {
    let mut value = json!({
        "schema_version": 1,
        "source_skill": SKILL_NAME,
        "status": "error",
        "error_kind": error_kind,
        "error_code": error_kind,
        "message_key": format!("skill.{}.{}", SKILL_NAME, error_kind),
        "retryable": false,
    });
    if let (Some(root), Some(detail)) = (value.as_object_mut(), detail) {
        root.insert("detail".to_string(), detail);
    }
    value
}

fn execute(args: Value) -> Result<(String, Value), GitBasicError> {
    let workspace_root = std::env::var("WORKSPACE_ROOT")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    execute_with_workspace_root(&workspace_root, args)
}

fn execute_with_workspace_root(
    workspace_root: &Path,
    args: Value,
) -> Result<(String, Value), GitBasicError> {
    let obj = args
        .as_object()
        .ok_or_else(|| GitBasicError::new("invalid_args", tr("git_basic.err.args_object")))?;
    let raw_action = optional_string(obj, "action", "git_action_invalid")?.unwrap_or("status");
    let action = normalize_action(raw_action);
    let root = resolve_repository_root(workspace_root, obj.get("repo"))?;
    let page = page_spec(obj)?;

    let mut input_meta = Map::new();
    input_meta.insert("cursor".to_string(), json!(page.cursor));
    input_meta.insert("limit".to_string(), json!(page.limit));
    let (subcmd, mut extra): (&str, Vec<String>) = match action.as_str() {
        "status" => (
            "status",
            vec!["--short".to_string(), "--branch".to_string()],
        ),
        "log" => (
            "log",
            vec![
                "--oneline".to_string(),
                "--skip".to_string(),
                page.cursor.to_string(),
                "-n".to_string(),
                page.limit.saturating_add(1).to_string(),
            ],
        ),
        "diff" => ("diff", validated_pathspec_args(obj.get("path"))?),
        "diff_cached" => {
            let mut args = vec!["--cached".to_string()];
            args.extend(validated_pathspec_args(obj.get("path"))?);
            ("diff", args)
        }
        "branch" => ("branch", vec!["--all".to_string()]),
        "current_branch" => (
            "rev-parse",
            vec!["--abbrev-ref".to_string(), "HEAD".to_string()],
        ),
        "remote" => ("remote", vec!["-v".to_string()]),
        "changed_files" => {
            let mut args = vec!["--name-only".to_string(), "HEAD".to_string()];
            args.extend(validated_pathspec_args(obj.get("path"))?);
            ("diff", args)
        }
        "show" => {
            let requested =
                optional_string(obj, "target", "git_revision_invalid")?.unwrap_or("HEAD");
            let revision = resolve_revision(&root, requested)?;
            input_meta.insert("target".to_string(), json!(requested));
            input_meta.insert("revision".to_string(), json!(revision));
            ("show", vec!["--stat".to_string(), revision])
        }
        "show_file_at_rev" => {
            let requested =
                optional_string(obj, "target", "git_revision_invalid")?.unwrap_or("HEAD");
            let revision = resolve_revision(&root, requested)?;
            let path = normalize_repo_relative_path(
                optional_string(obj, "path", "git_path_invalid")?.ok_or_else(|| {
                    GitBasicError::new("git_path_missing", "git_basic.path_required")
                })?,
            )?;
            if path == "." {
                return Err(GitBasicError::new(
                    "git_path_invalid",
                    "git_basic.file_path_required",
                ));
            }
            input_meta.insert("target".to_string(), json!(requested));
            input_meta.insert("revision".to_string(), json!(revision));
            input_meta.insert("path".to_string(), json!(path));
            input_meta.insert("source".to_string(), json!("git_show_file_at_rev"));
            input_meta.insert("source_kind".to_string(), json!("git_revision_file"));
            ("show", vec![format!("{revision}:{path}")])
        }
        "rev_parse" => {
            let requested = optional_string(obj, "ref", "git_revision_invalid")?.unwrap_or("HEAD");
            let revision = resolve_revision(&root, requested)?;
            input_meta.insert("target".to_string(), json!(requested));
            input_meta.insert("revision".to_string(), json!(revision));
            ("rev-parse", vec![revision])
        }
        _ => {
            return Err(GitBasicError::new(
                "unsupported_action",
                tr("git_basic.err.unsupported_action"),
            ));
        }
    };

    let mut cmd = Command::new("git");
    cmd.current_dir(&root)
        .arg(subcmd)
        .args(extra.drain(..))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let out = cmd.output().map_err(|err| {
        GitBasicError::new(
            "git_spawn_failed",
            tr_with(
                "git_basic.err.run_git_failed",
                &[("error", &err.to_string())],
            ),
        )
    })?;

    let mut text = String::new();
    text.push_str(&String::from_utf8_lossy(&out.stdout));
    if !out.stderr.is_empty() {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&String::from_utf8_lossy(&out.stderr));
    }

    let exit_code = out.status.code().unwrap_or(-1);
    if out.status.success() {
        if action == "log" {
            let (bounded, page_value) = bound_text_lines(&text, page);
            text = bounded;
            input_meta.insert("page".to_string(), page_value);
            input_meta.insert("page_prebounded".to_string(), json!(true));
        }
        let output = format!("exit={exit_code}\n{text}");
        let mut extra = git_success_extra(
            action.as_str(),
            raw_action,
            subcmd,
            exit_code,
            &text,
            &output,
            Some(&input_meta),
        );
        append_repository_provenance(&mut extra, &root, &output);
        apply_structured_page(action.as_str(), &mut extra, page);
        Ok((output, extra))
    } else {
        Err(GitBasicError::new(
            "git_command_failed",
            format!("git command failed: exit={exit_code}\n{text}"),
        )
        .with_extra(json!({
            "exit_code": exit_code,
            "action": action,
            "subcommand": subcmd,
        })))
    }
}

#[derive(Debug, Clone, Copy)]
struct PageSpec {
    cursor: usize,
    limit: usize,
}

fn page_spec(obj: &Map<String, Value>) -> Result<PageSpec, GitBasicError> {
    let cursor = bounded_usize(obj.get("cursor"), 0, usize::MAX, "git_cursor_invalid")?;
    let limit = bounded_usize(
        obj.get("limit").or_else(|| obj.get("n")),
        DEFAULT_PAGE_LIMIT,
        MAX_PAGE_LIMIT,
        "git_limit_invalid",
    )?;
    if limit == 0 {
        return Err(GitBasicError::new(
            "git_limit_invalid",
            "git_basic.limit_must_be_positive",
        ));
    }
    Ok(PageSpec { cursor, limit })
}

fn bounded_usize(
    value: Option<&Value>,
    default: usize,
    maximum: usize,
    error_code: &'static str,
) -> Result<usize, GitBasicError> {
    let Some(value) = value else {
        return Ok(default);
    };
    let parsed = value
        .as_u64()
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| GitBasicError::new(error_code, error_code))?;
    if parsed > maximum {
        return Err(GitBasicError::new(error_code, error_code));
    }
    Ok(parsed)
}

fn optional_string<'a>(
    obj: &'a Map<String, Value>,
    key: &str,
    error_code: &'static str,
) -> Result<Option<&'a str>, GitBasicError> {
    let Some(value) = obj.get(key) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let value = value
        .as_str()
        .ok_or_else(|| GitBasicError::new(error_code, error_code))?
        .trim();
    Ok((!value.is_empty()).then_some(value))
}

fn resolve_repository_root(
    workspace_root: &Path,
    repo: Option<&Value>,
) -> Result<PathBuf, GitBasicError> {
    let workspace = workspace_root
        .canonicalize()
        .map_err(|error| GitBasicError::new("workspace_canonicalize_failed", error.to_string()))?;
    let requested = match repo {
        None | Some(Value::Null) => ".",
        Some(Value::String(value)) if value.trim().is_empty() => ".",
        Some(Value::String(value)) => value.trim(),
        Some(_) => {
            return Err(GitBasicError::new(
                "git_repository_path_invalid",
                "git_basic.repository_path_invalid",
            ));
        }
    };
    let relative = normalize_repo_relative_path(requested)?;
    let candidate = workspace
        .join(relative)
        .canonicalize()
        .map_err(|error| GitBasicError::new("git_repository_path_invalid", error.to_string()))?;
    if !candidate.starts_with(&workspace) {
        return Err(GitBasicError::new(
            "git_repository_outside_workspace",
            "git_basic.repository_outside_workspace",
        ));
    }
    let output = Command::new("git")
        .current_dir(&candidate)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .map_err(|error| GitBasicError::new("git_spawn_failed", error.to_string()))?;
    if !output.status.success() {
        return Err(GitBasicError::new(
            "not_git_repository",
            tr("git_basic.msg.not_git_repo"),
        ));
    }
    let root = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    let root = root
        .canonicalize()
        .map_err(|error| GitBasicError::new("git_repository_path_invalid", error.to_string()))?;
    if !root.starts_with(&workspace) {
        return Err(GitBasicError::new(
            "git_repository_outside_workspace",
            "git_basic.repository_outside_workspace",
        ));
    }
    Ok(root)
}

fn normalize_repo_relative_path(value: &str) -> Result<String, GitBasicError> {
    if value.is_empty() || value.len() > MAX_REPO_PATH_BYTES {
        return Err(GitBasicError::new(
            "git_path_invalid",
            "git_basic.path_invalid",
        ));
    }
    let path = Path::new(value);
    if path.is_absolute() {
        return Err(GitBasicError::new(
            "git_path_outside_workspace",
            "git_basic.absolute_path_rejected",
        ));
    }
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => normalized.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(GitBasicError::new(
                    "git_path_outside_workspace",
                    "git_basic.path_traversal_rejected",
                ));
            }
        }
    }
    let normalized = normalized.to_string_lossy().replace('\\', "/");
    if normalized.is_empty() {
        Ok(".".to_string())
    } else {
        Ok(normalized)
    }
}

fn validated_pathspec_args(value: Option<&Value>) -> Result<Vec<String>, GitBasicError> {
    let path = match value {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::String(path)) => path.trim(),
        Some(_) => {
            return Err(GitBasicError::new(
                "git_path_invalid",
                "git_basic.path_invalid",
            ));
        }
    };
    if path.is_empty() {
        return Ok(Vec::new());
    }
    let path = normalize_repo_relative_path(path)?;
    Ok(vec!["--".to_string(), path])
}

fn resolve_revision(root: &Path, requested: &str) -> Result<String, GitBasicError> {
    let requested = requested.trim();
    if requested.is_empty()
        || requested.len() > MAX_REVISION_BYTES
        || requested.starts_with('-')
        || requested.chars().any(char::is_control)
    {
        return Err(GitBasicError::new(
            "git_revision_invalid",
            "git_basic.revision_invalid",
        ));
    }
    let revision_arg = format!("{requested}^{{object}}");
    let output = Command::new("git")
        .current_dir(root)
        .args(["rev-parse", "--verify", &revision_arg])
        .output()
        .map_err(|error| GitBasicError::new("git_spawn_failed", error.to_string()))?;
    let revision = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !output.status.success()
        || !matches!(revision.len(), 40 | 64)
        || !revision.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(
            GitBasicError::new("git_revision_not_found", "git_basic.revision_not_found")
                .with_extra(json!({"target": requested})),
        );
    }
    Ok(revision)
}

fn bound_text_lines(text: &str, page: PageSpec) -> (String, Value) {
    let lines = text.lines().collect::<Vec<_>>();
    let has_more = lines.len() > page.limit;
    let returned = lines.len().min(page.limit);
    let mut bounded = lines[..returned].join("\n");
    if returned > 0 && text.ends_with('\n') {
        bounded.push('\n');
    }
    (
        bounded,
        json!({
            "cursor": page.cursor,
            "limit": page.limit,
            "returned_count": returned,
            "total_count": Value::Null,
            "has_more": has_more,
            "next_cursor": has_more.then_some(page.cursor.saturating_add(returned)),
            "previous_cursor": (page.cursor > 0)
                .then_some(page.cursor.saturating_sub(page.limit)),
        }),
    )
}

fn append_repository_provenance(extra: &mut Value, root: &Path, output: &str) {
    let Some(fields) = extra.as_object_mut() else {
        return;
    };
    let observed_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    let head_revision = resolve_revision(root, "HEAD").ok();
    let digest = Sha256::digest(output.as_bytes());
    fields.insert("output_bytes".to_string(), json!(output.len()));
    fields.insert(
        "output_sha256".to_string(),
        json!(format!("sha256:{digest:x}")),
    );
    fields.insert("truncated".to_string(), json!(false));
    fields.insert(
        "provenance".to_string(),
        json!({
            "source": "git_cli",
            "repository_root": root,
            "head_revision": head_revision,
            "observed_at": observed_at,
            "operation_class": "read_only",
        }),
    );
}

fn apply_structured_page(action: &str, extra: &mut Value, page: PageSpec) {
    let Some(root) = extra.as_object_mut() else {
        return;
    };
    if root
        .remove("page_prebounded")
        .and_then(|value| value.as_bool())
        == Some(true)
    {
        let page_value = root.get("page").cloned().unwrap_or(Value::Null);
        let has_more = page_value
            .get("has_more")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        root.insert("truncated".to_string(), json!(has_more));
        if let Some(field_value) = root.get_mut("field_value").and_then(Value::as_object_mut) {
            field_value.remove("page_prebounded");
            field_value.insert("page".to_string(), page_value);
        }
        return;
    }

    let list_key = match action {
        "status" | "changed_files" => "changed_files",
        "branch" => "branches",
        "remote" => "remotes",
        _ => return,
    };
    let Some(all_items) = root.get(list_key).and_then(Value::as_array).cloned() else {
        return;
    };
    let total = all_items.len();
    let start = page.cursor.min(total);
    let end = start.saturating_add(page.limit).min(total);
    let selected = all_items[start..end].to_vec();
    let has_more = end < total;
    let page_value = json!({
        "cursor": page.cursor,
        "limit": page.limit,
        "returned_count": selected.len(),
        "total_count": total,
        "has_more": has_more,
        "next_cursor": has_more.then_some(end),
        "previous_cursor": (page.cursor > 0)
            .then_some(page.cursor.saturating_sub(page.limit)),
    });
    root.insert(list_key.to_string(), json!(selected));
    root.insert("page".to_string(), page_value.clone());
    root.insert("truncated".to_string(), json!(has_more));

    match action {
        "status" | "changed_files" => {
            let paths = root.get(list_key).cloned().unwrap_or_else(|| json!([]));
            root.insert("paths".to_string(), paths.clone());
            if let Some(field_value) = root.get_mut("field_value").and_then(Value::as_object_mut) {
                field_value.insert("paths".to_string(), paths);
                field_value.insert("page".to_string(), page_value);
            }
        }
        "branch" => {
            if let Some(field_value) = root.get_mut("field_value").and_then(Value::as_object_mut) {
                field_value.insert("page".to_string(), page_value);
            }
        }
        "remote" => {
            let remotes = root
                .get("remotes")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let remote_names = unique_remote_names(&remotes);
            let remote_urls = unique_remote_urls(&remotes);
            root.insert("remote_names".to_string(), json!(remote_names.clone()));
            root.insert("remote_urls".to_string(), json!(remote_urls.clone()));
            if let Some(field_value) = root.get_mut("field_value").and_then(Value::as_object_mut) {
                field_value.insert("remotes".to_string(), json!(remote_names.clone()));
                field_value.insert("remote_names".to_string(), json!(remote_names));
                field_value.insert("remote_urls".to_string(), json!(remote_urls));
                field_value.insert("page".to_string(), page_value);
            }
        }
        _ => {}
    }
}

fn git_success_extra(
    action: &str,
    raw_action: &str,
    subcommand: &str,
    exit_code: i32,
    command_text: &str,
    output: &str,
    input_meta: Option<&Map<String, Value>>,
) -> Value {
    let mut root = Map::new();
    root.insert("schema_version".to_string(), json!(1));
    root.insert("action".to_string(), json!(action));
    root.insert("raw_action".to_string(), json!(raw_action));
    root.insert("subcommand".to_string(), json!(subcommand));
    root.insert("exit_code".to_string(), json!(exit_code));
    root.insert("output".to_string(), json!(output));
    let mut field_value = Map::new();
    field_value.insert("exit_code".to_string(), json!(exit_code));
    field_value.insert("action".to_string(), json!(action));
    if let Some(input_meta) = input_meta {
        for (key, value) in input_meta {
            root.insert(key.clone(), value.clone());
            field_value.insert(key.clone(), value.clone());
        }
    }

    match action {
        "status" => append_git_status_extra(command_text, &mut root, &mut field_value),
        "current_branch" => append_current_branch_extra(command_text, &mut root, &mut field_value),
        "changed_files" => append_changed_files_extra(command_text, &mut root, &mut field_value),
        "log" => append_git_log_extra(command_text, &mut root, &mut field_value),
        "rev_parse" => append_rev_parse_extra(command_text, &mut root, &mut field_value),
        "branch" => append_branch_list_extra(command_text, &mut root, &mut field_value),
        "remote" => append_remote_list_extra(command_text, &mut root, &mut field_value),
        "show_file_at_rev" => {
            append_show_file_at_rev_extra(command_text, &mut root, &mut field_value)
        }
        _ => {}
    }

    root.insert("field_value".to_string(), Value::Object(field_value));
    Value::Object(root)
}

fn append_git_status_extra(
    text: &str,
    root: &mut Map<String, Value>,
    field_value: &mut Map<String, Value>,
) {
    let summary = parse_git_status_summary(text);
    if let Some(branch) = summary.branch.as_deref() {
        root.insert("branch".to_string(), json!(branch));
        root.insert("current_branch".to_string(), json!(branch));
        field_value.insert("branch".to_string(), json!(branch));
        field_value.insert("current_branch".to_string(), json!(branch));
    }
    if let Some(upstream) = summary.upstream.as_deref() {
        root.insert("upstream".to_string(), json!(upstream));
        field_value.insert("upstream".to_string(), json!(upstream));
    }
    if let Some(ahead) = summary.ahead {
        root.insert("ahead".to_string(), json!(ahead));
        field_value.insert("ahead".to_string(), json!(ahead));
    }
    if let Some(behind) = summary.behind {
        root.insert("behind".to_string(), json!(behind));
        field_value.insert("behind".to_string(), json!(behind));
    }
    root.insert("clean".to_string(), json!(summary.clean));
    root.insert(
        "worktree_state".to_string(),
        json!(if summary.clean { "clean" } else { "dirty" }),
    );
    root.insert("changed_count".to_string(), json!(summary.changed_count));
    root.insert("staged_count".to_string(), json!(summary.staged_count));
    root.insert("unstaged_count".to_string(), json!(summary.unstaged_count));
    root.insert(
        "untracked_count".to_string(),
        json!(summary.untracked_count),
    );
    root.insert(
        "changed_files".to_string(),
        json!(summary.changed_files.clone()),
    );
    root.insert("paths".to_string(), json!(summary.changed_files.clone()));
    field_value.insert("clean".to_string(), json!(summary.clean));
    field_value.insert(
        "worktree_state".to_string(),
        json!(if summary.clean { "clean" } else { "dirty" }),
    );
    field_value.insert("changed_count".to_string(), json!(summary.changed_count));
    field_value.insert("staged_count".to_string(), json!(summary.staged_count));
    field_value.insert("unstaged_count".to_string(), json!(summary.unstaged_count));
    field_value.insert(
        "untracked_count".to_string(),
        json!(summary.untracked_count),
    );
    field_value.insert("paths".to_string(), json!(summary.changed_files.clone()));
}

#[derive(Debug, Default, PartialEq, Eq)]
struct GitStatusSummary {
    branch: Option<String>,
    upstream: Option<String>,
    ahead: Option<u64>,
    behind: Option<u64>,
    clean: bool,
    changed_count: usize,
    staged_count: usize,
    unstaged_count: usize,
    untracked_count: usize,
    changed_files: Vec<String>,
}

fn parse_git_status_summary(text: &str) -> GitStatusSummary {
    let mut summary = GitStatusSummary::default();
    for line in text
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.is_empty())
    {
        if let Some(header) = line.strip_prefix("## ") {
            parse_git_status_branch_header(header, &mut summary);
            continue;
        }
        if line.len() < 3 {
            continue;
        }
        let code = &line[..2];
        let path = line[3..].trim();
        if path.is_empty() {
            continue;
        }
        summary.changed_count += 1;
        if code == "??" {
            summary.untracked_count += 1;
        } else {
            let bytes = code.as_bytes();
            if bytes.first().is_some_and(|ch| *ch != b' ') {
                summary.staged_count += 1;
            }
            if bytes.get(1).is_some_and(|ch| *ch != b' ') {
                summary.unstaged_count += 1;
            }
        }
        summary.changed_files.push(normalize_git_status_path(path));
    }
    summary.clean = summary.changed_count == 0;
    summary
}

fn parse_git_status_branch_header(header: &str, summary: &mut GitStatusSummary) {
    let branch_part = header
        .split_once(' ')
        .map(|(head, _)| head)
        .unwrap_or(header)
        .trim();
    if let Some((branch, upstream)) = branch_part.split_once("...") {
        if !branch.trim().is_empty() {
            summary.branch = Some(branch.trim().to_string());
        }
        if !upstream.trim().is_empty() {
            summary.upstream = Some(upstream.trim().to_string());
        }
    } else if !branch_part.is_empty() {
        summary.branch = Some(branch_part.to_string());
    }
    if let Some((_, bracket_tail)) = header.split_once('[') {
        if let Some((bracket, _)) = bracket_tail.split_once(']') {
            parse_git_status_ahead_behind(bracket, summary);
        }
    }
}

fn parse_git_status_ahead_behind(bracket: &str, summary: &mut GitStatusSummary) {
    let parts = bracket
        .split([',', ' '])
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    for pair in parts.windows(2) {
        match pair[0] {
            "ahead" => summary.ahead = pair[1].parse::<u64>().ok(),
            "behind" => summary.behind = pair[1].parse::<u64>().ok(),
            _ => {}
        }
    }
}

fn normalize_git_status_path(path: &str) -> String {
    path.split(" -> ")
        .last()
        .unwrap_or(path)
        .trim()
        .trim_matches('"')
        .to_string()
}

fn append_current_branch_extra(
    text: &str,
    root: &mut Map<String, Value>,
    field_value: &mut Map<String, Value>,
) {
    if let Some(branch) = first_non_empty_line(text) {
        root.insert("branch".to_string(), json!(branch));
        root.insert("current_branch".to_string(), json!(branch));
        field_value.insert("branch".to_string(), json!(branch));
        field_value.insert("current_branch".to_string(), json!(branch));
    }
}

fn append_changed_files_extra(
    text: &str,
    root: &mut Map<String, Value>,
    field_value: &mut Map<String, Value>,
) {
    let files = non_empty_lines(text);
    root.insert("changed_files".to_string(), json!(files.clone()));
    root.insert("paths".to_string(), json!(files.clone()));
    root.insert("changed_count".to_string(), json!(files.len()));
    field_value.insert("changed_count".to_string(), json!(files.len()));
    field_value.insert("paths".to_string(), json!(files));
}

fn append_git_log_extra(
    text: &str,
    root: &mut Map<String, Value>,
    field_value: &mut Map<String, Value>,
) {
    let commits = parse_git_log_commits(text);
    let subjects = commits
        .iter()
        .filter_map(|commit| commit.get("subject").and_then(Value::as_str))
        .map(str::to_string)
        .collect::<Vec<_>>();
    root.insert("commits".to_string(), json!(commits));
    root.insert("commit_count".to_string(), json!(subjects.len()));
    if let Some(subject) = subjects.first() {
        root.insert("subject".to_string(), json!(subject));
        field_value.insert("subject".to_string(), json!(subject));
    }
    root.insert("subjects".to_string(), json!(subjects));
    field_value.insert("commit_count".to_string(), json!(subjects.len()));
}

fn parse_git_log_commits(text: &str) -> Vec<Value> {
    non_empty_lines(text)
        .into_iter()
        .filter_map(|line| {
            let (sha, subject) = line.split_once(' ')?;
            Some(json!({
                "sha": sha,
                "subject": subject.trim(),
            }))
        })
        .collect()
}

fn append_rev_parse_extra(
    text: &str,
    root: &mut Map<String, Value>,
    field_value: &mut Map<String, Value>,
) {
    if let Some(revision) = first_non_empty_line(text) {
        root.insert("revision".to_string(), json!(revision));
        field_value.insert("revision".to_string(), json!(revision));
    }
}

fn append_branch_list_extra(
    text: &str,
    root: &mut Map<String, Value>,
    field_value: &mut Map<String, Value>,
) {
    let branches = parse_branch_list(text);
    let current_branch = branches
        .iter()
        .find(|branch| branch.get("current").and_then(Value::as_bool) == Some(true))
        .and_then(|branch| branch.get("name").and_then(Value::as_str))
        .map(str::to_string);
    root.insert("branches".to_string(), json!(branches));
    root.insert("branch_count".to_string(), json!(branches.len()));
    if let Some(branch) = current_branch {
        root.insert("current_branch".to_string(), json!(branch));
        field_value.insert("current_branch".to_string(), json!(branch));
    }
    field_value.insert("branch_count".to_string(), json!(branches.len()));
}

fn parse_branch_list(text: &str) -> Vec<Value> {
    text.lines()
        .map(str::trim_end)
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let current = line.trim_start().starts_with('*');
            let name = line.trim_start_matches(['*', ' ']).trim().to_string();
            json!({
                "name": name,
                "current": current,
            })
        })
        .collect()
}

fn append_remote_list_extra(
    text: &str,
    root: &mut Map<String, Value>,
    field_value: &mut Map<String, Value>,
) {
    let remotes = parse_remote_list(text);
    let remote_names = unique_remote_names(&remotes);
    let remote_urls = unique_remote_urls(&remotes);
    root.insert("remotes".to_string(), json!(remotes));
    root.insert("remote_names".to_string(), json!(remote_names.clone()));
    root.insert("remote_urls".to_string(), json!(remote_urls.clone()));
    root.insert("remote_count".to_string(), json!(remotes.len()));
    field_value.insert("remotes".to_string(), json!(remote_names.clone()));
    field_value.insert("remote_names".to_string(), json!(remote_names));
    field_value.insert("remote_urls".to_string(), json!(remote_urls));
    field_value.insert("remote_count".to_string(), json!(remotes.len()));
}

fn append_show_file_at_rev_extra(
    text: &str,
    root: &mut Map<String, Value>,
    field_value: &mut Map<String, Value>,
) {
    let content_excerpt = bounded_single_line_excerpt(text, 240);
    root.insert("content_excerpt".to_string(), json!(content_excerpt));
    root.insert(
        "content_line_count".to_string(),
        json!(text.lines().count()),
    );
    root.insert("content_bytes".to_string(), json!(text.len()));
    field_value.insert("content_excerpt".to_string(), json!(content_excerpt));
    field_value.insert(
        "content_line_count".to_string(),
        json!(text.lines().count()),
    );
    field_value.insert("content_bytes".to_string(), json!(text.len()));
}

fn bounded_single_line_excerpt(text: &str, limit: usize) -> String {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("")
        .chars()
        .take(limit)
        .collect()
}

fn parse_remote_list(text: &str) -> Vec<Value> {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let name = parts.next()?;
            let url = parts.next()?;
            let direction = parts
                .next()
                .map(|value| value.trim_matches(['(', ')']))
                .unwrap_or("");
            Some(json!({
                "name": name,
                "url": url,
                "direction": direction,
            }))
        })
        .collect()
}

fn unique_remote_names(remotes: &[Value]) -> Vec<String> {
    let mut names = Vec::new();
    for remote in remotes {
        let Some(name) = remote
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty())
        else {
            continue;
        };
        if !names.iter().any(|existing| existing == name) {
            names.push(name.to_string());
        }
    }
    names
}

fn unique_remote_urls(remotes: &[Value]) -> Vec<String> {
    let mut urls = Vec::new();
    for remote in remotes {
        let Some(url) = remote
            .get("url")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|url| !url.is_empty())
        else {
            continue;
        };
        if !urls.iter().any(|existing| existing == url) {
            urls.push(url.to_string());
        }
    }
    urls
}

fn first_non_empty_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_string)
}

fn non_empty_lines(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

fn normalize_action(raw: &str) -> String {
    let normalized = raw.trim().to_ascii_lowercase().replace('-', "_");
    match normalized.as_str() {
        "branches" | "list_branches" | "all_branches" => "branch",
        "current_branch_name" | "branch_current" | "get_current_branch" => "current_branch",
        "cached_diff" | "staged_diff" => "diff_cached",
        "changed_file" | "changed_file_names" => "changed_files",
        "revparse" | "head" => "rev_parse",
        _ => normalized.as_str(),
    }
    .to_string()
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
