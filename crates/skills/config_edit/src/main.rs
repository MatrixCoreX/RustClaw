use std::io::{self, BufRead, Write};
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use toml_edit::{value as toml_value_item, DocumentMut, Item};

const SKILL_NAME: &str = "config_edit";

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
    #[serde(skip_serializing_if = "Option::is_none")]
    error_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    platform: Option<String>,
}

#[derive(Debug, Clone)]
struct SkillError {
    kind: &'static str,
    message: String,
    extra: Option<Value>,
}

type SkillResult<T> = Result<T, SkillError>;

impl SkillError {
    fn new(kind: &'static str, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            extra: None,
        }
    }

    fn with_extra(mut self, extra: Value) -> Self {
        self.extra = Some(extra);
        self
    }

    fn invalid_input(message: impl Into<String>) -> Self {
        Self::new("invalid_input", message)
    }

    fn invalid_data(message: impl Into<String>) -> Self {
        Self::new("invalid_data", message)
    }

    fn unsupported_action(message: impl Into<String>) -> Self {
        Self::new("unsupported_action", message)
    }

    fn path_denied(message: impl Into<String>) -> Self {
        Self::new("path_denied", message)
    }

    fn io(operation: &'static str, path: &Path, err: io::Error) -> Self {
        let kind = match err.kind() {
            io::ErrorKind::NotFound => "not_found",
            io::ErrorKind::PermissionDenied => "permission_denied",
            io::ErrorKind::InvalidInput => "invalid_input",
            io::ErrorKind::InvalidData => "invalid_data",
            _ => "io_error",
        };
        Self::new(
            kind,
            format!("{operation} failed for {}: {err}", path.display()),
        )
        .with_extra(json!({ "operation": operation, "path": path.display().to_string() }))
    }
}

fn main() -> anyhow::Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    for line in stdin.lock().lines() {
        let line = line?;
        let parsed: Result<Req, _> = serde_json::from_str(&line);
        let resp = match parsed {
            Ok(req) => handle(req),
            Err(err) => Resp {
                request_id: "unknown".to_string(),
                status: "error".to_string(),
                text: String::new(),
                extra: Some(error_extra("invalid_input")),
                error_text: Some(format!("invalid input: {err}")),
                error_kind: Some("invalid_input".to_string()),
                platform: Some(std::env::consts::OS.to_string()),
            },
        };
        writeln!(stdout, "{}", serde_json::to_string(&resp)?)?;
        stdout.flush()?;
    }
    Ok(())
}

fn handle(req: Req) -> Resp {
    let workspace_root = std::env::var("WORKSPACE_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let allow_path_outside_workspace = context_allows_path_outside_workspace(req.context.as_ref());
    match execute_action(&workspace_root, req.args, allow_path_outside_workspace) {
        Ok(extra) => Resp {
            request_id: req.request_id,
            status: "ok".to_string(),
            text: extra.to_string(),
            extra: Some(extra),
            error_text: None,
            error_kind: None,
            platform: None,
        },
        Err(err) => Resp {
            request_id: req.request_id,
            status: "error".to_string(),
            text: String::new(),
            extra: Some(error_extra_with_details(err.kind, err.extra)),
            error_text: Some(err.message),
            error_kind: Some(err.kind.to_string()),
            platform: Some(std::env::consts::OS.to_string()),
        },
    }
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

fn execute_action(
    workspace_root: &Path,
    args: Value,
    allow_path_outside_workspace: bool,
) -> SkillResult<Value> {
    let obj = args
        .as_object()
        .ok_or_else(|| SkillError::invalid_input("args must be object"))?;
    let action = obj
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or("plan_config_change")
        .trim()
        .to_ascii_lowercase();

    match action.as_str() {
        "plan_config_change" | "plan_change" => {
            plan_config_change(workspace_root, obj, allow_path_outside_workspace)
        }
        "apply_config_change" | "apply_change" | "write_field" | "set_field" => {
            apply_config_change(workspace_root, obj, allow_path_outside_workspace)
        }
        "validate_config" | "validate" => {
            validate_config(workspace_root, obj, allow_path_outside_workspace)
        }
        "guard_config" | "guard_rustclaw_config" => {
            guard_config(workspace_root, obj, allow_path_outside_workspace)
        }
        "read_back" => read_back(workspace_root, obj, allow_path_outside_workspace),
        "restart_if_requested" => restart_if_requested(obj),
        other => Err(SkillError::unsupported_action(format!(
            "unknown action: {other}; allowed: plan_config_change|apply_config_change|validate_config|guard_config|read_back|restart_if_requested"
        ))),
    }
}

fn plan_config_change(
    workspace_root: &Path,
    obj: &Map<String, Value>,
    allow_path_outside_workspace: bool,
) -> SkillResult<Value> {
    let target = config_target(workspace_root, obj, allow_path_outside_workspace)?;
    let field_path = required_str(obj, "field_path")?;
    let operation = operation_arg(obj);
    ensure_supported_operation(&operation)?;
    let requested_value = required_value(obj)?;
    let root = parse_root_value(&target.real_path, Some(target.format.as_str()))?;
    let old_value = lookup_json_path(&root, &split_field_path(field_path)).cloned();
    let new_value = coerce_value_like_existing(requested_value.clone(), old_value.as_ref());
    let sensitive = is_sensitive_field_path(field_path);
    let display_old = redact_if_sensitive(sensitive, old_value.clone().unwrap_or(Value::Null));
    let display_new = redact_if_sensitive(sensitive, new_value.clone());
    let would_change = old_value.as_ref() != Some(&new_value);
    let exists = old_value.is_some();

    Ok(json!({
        "action": "plan_config_change",
        "path": target.input_path,
        "resolved_path": target.real_path.display().to_string(),
        "format": target.format,
        "field_path": field_path,
        "operation": operation,
        "exists": exists,
        "old_value": display_old.clone(),
        "new_value": display_new.clone(),
        "would_change": would_change,
        "field_value": {
            "field_path": field_path,
            "old_value": display_old,
            "new_value": display_new,
            "exists": exists,
            "would_change": would_change,
        },
        "requires_confirmation": true,
        "restart_recommended": restart_recommended_for_path(&target.input_path),
    }))
}

fn apply_config_change(
    workspace_root: &Path,
    obj: &Map<String, Value>,
    allow_path_outside_workspace: bool,
) -> SkillResult<Value> {
    let target = config_target(workspace_root, obj, allow_path_outside_workspace)?;
    let field_path = required_str(obj, "field_path")?;
    let operation = operation_arg(obj);
    ensure_supported_operation(&operation)?;
    let requested_value = required_value(obj)?;
    let raw = std::fs::read_to_string(&target.real_path)
        .map_err(|err| SkillError::io("read_config", &target.real_path, err))?;
    let root_before = parse_root_value(&target.real_path, Some(target.format.as_str()))?;
    let old_value = lookup_json_path(&root_before, &split_field_path(field_path)).cloned();
    let new_value = coerce_value_like_existing(requested_value.clone(), old_value.as_ref());

    match target.format.as_str() {
        "toml" => {
            let mut doc = raw
                .parse::<DocumentMut>()
                .map_err(|err| SkillError::invalid_data(format!("parse toml failed: {err}")))?;
            set_toml_field(&mut doc, field_path, &new_value)?;
            std::fs::write(&target.real_path, doc.to_string())
                .map_err(|err| SkillError::io("write_config", &target.real_path, err))?;
        }
        "json" => {
            let mut root = root_before;
            set_json_path(&mut root, &split_field_path(field_path), new_value.clone())?;
            let text = serde_json::to_string_pretty(&root)
                .map_err(|err| SkillError::invalid_data(format!("serialize json failed: {err}")))?;
            std::fs::write(&target.real_path, format!("{text}\n"))
                .map_err(|err| SkillError::io("write_config", &target.real_path, err))?;
        }
        other => {
            return Err(SkillError::invalid_input(format!(
                "unsupported config format for mutation: {other}"
            )));
        }
    }

    let validation = validate_config_path(&target.real_path, Some(&target.format));
    if let Err(err) = validation {
        return Err(err.with_extra(json!({
            "action": "apply_config_change",
            "path": target.input_path,
            "resolved_path": target.real_path.display().to_string(),
            "field_path": field_path,
            "applied": true,
            "validation_failed": true,
        })));
    }

    let sensitive = is_sensitive_field_path(field_path);
    Ok(json!({
        "action": "apply_config_change",
        "applied": true,
        "path": target.input_path,
        "resolved_path": target.real_path.display().to_string(),
        "format": target.format,
        "field_path": field_path,
        "operation": operation,
        "old_value": redact_if_sensitive(sensitive, old_value.unwrap_or(Value::Null)),
        "new_value": redact_if_sensitive(sensitive, new_value),
        "validated": true,
        "restart_recommended": restart_recommended_for_path(&target.input_path),
    }))
}

fn validate_config(
    workspace_root: &Path,
    obj: &Map<String, Value>,
    allow_path_outside_workspace: bool,
) -> SkillResult<Value> {
    let target = config_target(workspace_root, obj, allow_path_outside_workspace)?;
    match validate_config_path(&target.real_path, Some(&target.format)) {
        Ok(root_type) => Ok(json!({
            "action": "validate_config",
            "path": target.input_path,
            "resolved_path": target.real_path.display().to_string(),
            "format": target.format,
            "valid": true,
            "root_type": root_type,
        })),
        Err(err) if matches!(err.kind, "invalid_data" | "invalid_input") => Ok(json!({
            "action": "validate_config",
            "path": target.input_path,
            "resolved_path": target.real_path.display().to_string(),
            "format": target.format,
            "valid": false,
            "error_kind": err.kind,
            "error_text": err.message,
        })),
        Err(err) => Err(err),
    }
}

fn guard_config(
    workspace_root: &Path,
    obj: &Map<String, Value>,
    allow_path_outside_workspace: bool,
) -> SkillResult<Value> {
    let target = config_target(workspace_root, obj, allow_path_outside_workspace)?;
    let root = parse_root_value(&target.real_path, Some(&target.format))?;
    let mut risks = Vec::new();

    for field_path in [
        "telegram.bot_token",
        "llm.openai.api_key",
        "llm.google.api_key",
        "llm.anthropic.api_key",
        "llm.grok.api_key",
        "llm.xai.api_key",
        "llm.deepseek.api_key",
        "llm.qwen.api_key",
        "llm.minimax.api_key",
        "llm.mimo.api_key",
    ] {
        if has_real_token(
            lookup_json_path(&root, &split_field_path(field_path)).and_then(Value::as_str),
        ) {
            risks.push(format!("{field_path} looks like a real secret"));
        }
    }
    if lookup_json_path(&root, &split_field_path("tools.allow_sudo"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        risks.push("tools.allow_sudo=true".to_string());
    }
    if lookup_json_path(
        &root,
        &split_field_path("tools.allow_path_outside_workspace"),
    )
    .and_then(Value::as_bool)
    .unwrap_or(false)
    {
        risks.push("tools.allow_path_outside_workspace=true".to_string());
    }
    if lookup_json_path(&root, &split_field_path("telegram.sendfile.full_access"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        risks.push("telegram.sendfile.full_access=true".to_string());
    }
    add_skills_registry_risks(&target.real_path, &root, &mut risks);
    let risk_count = risks.len();
    let valid = risk_count == 0;

    Ok(json!({
        "action": "guard_config",
        "path": target.input_path,
        "resolved_path": target.real_path.display().to_string(),
        "format": target.format,
        "valid": valid,
        "count": risk_count,
        "risk_count": risk_count,
        "candidates": risks.clone(),
        "risks": risks,
    }))
}

fn add_skills_registry_risks(path: &Path, root: &Value, risks: &mut Vec<String>) {
    if !path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("skills_registry.toml"))
    {
        return;
    }
    let Some(skills) = root.get("skills").and_then(Value::as_array) else {
        risks.push("skills registry has no skills array".to_string());
        return;
    };

    let mut skill_names = std::collections::HashSet::new();
    let mut aliases = std::collections::HashMap::<String, String>::new();
    for entry in skills {
        let Some(obj) = entry.as_object() else {
            risks.push("skills registry contains a non-object skill entry".to_string());
            continue;
        };
        let name = obj
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("(unnamed)");
        if !skill_names.insert(name.to_ascii_lowercase()) {
            risks.push(format!("duplicate skill name: {name}"));
        }

        let enabled = obj.get("enabled").and_then(Value::as_bool).unwrap_or(true);
        let planner_visible = obj
            .get("planner_visible")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let prompt_missing = obj
            .get("prompt_file")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_none();
        if enabled && planner_visible && prompt_missing {
            risks.push(format!(
                "enabled planner-visible skill {name} is missing prompt_file"
            ));
        }

        let high_risk = obj
            .get("risk_level")
            .and_then(Value::as_str)
            .is_some_and(|value| value.trim().eq_ignore_ascii_case("high"));
        let side_effect = obj
            .get("side_effect")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let explicitly_no_confirmation =
            obj.get("requires_confirmation").and_then(Value::as_bool) == Some(false);
        if enabled && (high_risk || side_effect) && explicitly_no_confirmation {
            risks.push(format!(
                "enabled high-risk or side-effect skill {name} explicitly disables confirmation"
            ));
        }

        if let Some(values) = obj.get("aliases").and_then(Value::as_array) {
            for alias in values
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                let key = alias.to_ascii_lowercase();
                if let Some(existing) = aliases.insert(key, name.to_string()) {
                    risks.push(format!("alias {alias} is shared by {existing} and {name}"));
                }
            }
        }
    }
}

fn read_back(
    workspace_root: &Path,
    obj: &Map<String, Value>,
    allow_path_outside_workspace: bool,
) -> SkillResult<Value> {
    let target = config_target(workspace_root, obj, allow_path_outside_workspace)?;
    let field_path = required_str(obj, "field_path")?;
    let root = parse_root_value(&target.real_path, Some(&target.format))?;
    let value = lookup_json_path(&root, &split_field_path(field_path)).cloned();
    let sensitive = is_sensitive_field_path(field_path);
    let display_value = redact_if_sensitive(sensitive, value.clone().unwrap_or(Value::Null));

    Ok(json!({
        "action": "read_back",
        "path": target.input_path,
        "resolved_path": target.real_path.display().to_string(),
        "format": target.format,
        "field_path": field_path,
        "exists": value.is_some(),
        "value_type": value.as_ref().map(json_value_type).unwrap_or("null"),
        "value": display_value,
        "value_text": json_value_to_text(&display_value),
    }))
}

fn restart_if_requested(obj: &Map<String, Value>) -> SkillResult<Value> {
    let restart = obj.get("restart").and_then(Value::as_bool).unwrap_or(false);
    let reason = obj
        .get("reason")
        .and_then(Value::as_str)
        .unwrap_or("config changed");
    Ok(json!({
        "action": "restart_if_requested",
        "restart_requested": restart,
        "restarted": false,
        "restart_supported": false,
        "restart_recommended": true,
        "reason": reason,
        "message": if restart {
            "Restart was requested, but config_edit does not execute restarts in this version. Use the approved restart workflow."
        } else {
            "Config changed; restart may be required for already running services to reload it."
        },
    }))
}

#[derive(Debug)]
struct ConfigTarget {
    input_path: String,
    real_path: PathBuf,
    format: String,
}

fn config_target(
    workspace_root: &Path,
    obj: &Map<String, Value>,
    allow_path_outside_workspace: bool,
) -> SkillResult<ConfigTarget> {
    let input_path = obj
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .unwrap_or("configs/config.toml")
        .to_string();
    let real_path = resolve_path(workspace_root, &input_path, allow_path_outside_workspace)?;
    let format = obj
        .get("format")
        .and_then(Value::as_str)
        .map(normalize_format)
        .unwrap_or_else(|| detect_format_from_path(&real_path));
    Ok(ConfigTarget {
        input_path,
        real_path,
        format,
    })
}

fn context_allows_path_outside_workspace(context: Option<&Value>) -> bool {
    context
        .and_then(|ctx| {
            ctx.get("permissions")
                .and_then(|permissions| permissions.get("allow_path_outside_workspace"))
                .or_else(|| ctx.get("allow_path_outside_workspace"))
        })
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn resolve_path(
    workspace_root: &Path,
    input: &str,
    allow_path_outside_workspace: bool,
) -> SkillResult<PathBuf> {
    let raw = Path::new(input);
    if allow_path_outside_workspace {
        return if raw.is_absolute() {
            Ok(raw.to_path_buf())
        } else {
            Ok(workspace_root.join(raw))
        };
    }

    let mut normalized = PathBuf::new();
    for comp in raw.components() {
        match comp {
            Component::ParentDir => {
                return Err(SkillError::path_denied("path with '..' is not allowed"));
            }
            Component::CurDir => {}
            other => normalized.push(other.as_os_str()),
        }
    }

    let candidate = if raw.is_absolute() {
        normalized
    } else {
        workspace_root.join(normalized)
    };
    let normalized_root = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.to_path_buf());
    let normalized_candidate = candidate
        .canonicalize()
        .unwrap_or_else(|_| candidate.clone());
    if !normalized_candidate.starts_with(normalized_root) {
        return Err(SkillError::path_denied("path is outside workspace"));
    }
    Ok(candidate)
}

fn required_str<'a>(obj: &'a Map<String, Value>, key: &str) -> SkillResult<&'a str> {
    obj.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| SkillError::invalid_input(format!("{key} is required")))
}

fn required_value(obj: &Map<String, Value>) -> SkillResult<&Value> {
    obj.get("value")
        .ok_or_else(|| SkillError::invalid_input("value is required"))
}

fn operation_arg(obj: &Map<String, Value>) -> String {
    obj.get("operation")
        .and_then(Value::as_str)
        .unwrap_or("set")
        .trim()
        .to_ascii_lowercase()
}

fn ensure_supported_operation(operation: &str) -> SkillResult<()> {
    if operation == "set" {
        Ok(())
    } else {
        Err(SkillError::invalid_input(format!(
            "unsupported operation: {operation}; allowed: set"
        )))
    }
}

fn parse_root_value(path: &Path, format: Option<&str>) -> SkillResult<Value> {
    let raw =
        std::fs::read_to_string(path).map_err(|err| SkillError::io("read_config", path, err))?;
    match format
        .map(normalize_format)
        .unwrap_or_else(|| detect_format_from_path(path))
        .as_str()
    {
        "toml" => {
            let parsed: toml::Value = toml::from_str(&raw)
                .map_err(|err| SkillError::invalid_data(format!("parse toml failed: {err}")))?;
            serde_json::to_value(parsed)
                .map_err(|err| SkillError::invalid_data(format!("convert toml failed: {err}")))
        }
        "json" => serde_json::from_str(&raw)
            .map_err(|err| SkillError::invalid_data(format!("parse json failed: {err}"))),
        other => Err(SkillError::invalid_input(format!(
            "unsupported config format: {other}"
        ))),
    }
}

fn validate_config_path(path: &Path, format: Option<&str>) -> SkillResult<&'static str> {
    let root = parse_root_value(path, format)?;
    Ok(json_value_type(&root))
}

fn detect_format_from_path(path: &Path) -> String {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "json" => "json".to_string(),
        _ => "toml".to_string(),
    }
}

fn normalize_format(format: &str) -> String {
    match format.trim().to_ascii_lowercase().as_str() {
        "json" => "json".to_string(),
        _ => "toml".to_string(),
    }
}

fn split_field_path(field_path: &str) -> Vec<&str> {
    field_path
        .split('.')
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .collect()
}

fn lookup_json_path<'a>(root: &'a Value, segments: &[&str]) -> Option<&'a Value> {
    let mut current = root;
    for segment in segments {
        match current {
            Value::Object(map) => current = map.get(*segment)?,
            Value::Array(values) => {
                let idx = segment.parse::<usize>().ok()?;
                current = values.get(idx)?;
            }
            _ => return None,
        }
    }
    Some(current)
}

fn set_json_path(root: &mut Value, segments: &[&str], value: Value) -> SkillResult<()> {
    if segments.is_empty() {
        return Err(SkillError::invalid_input("field_path is empty"));
    }
    let mut current = root;
    for segment in &segments[..segments.len() - 1] {
        if !current.is_object() {
            *current = Value::Object(Map::new());
        }
        let map = current
            .as_object_mut()
            .ok_or_else(|| SkillError::invalid_data("expected object while setting field"))?;
        current = map
            .entry((*segment).to_string())
            .or_insert_with(|| Value::Object(Map::new()));
    }
    let last = segments
        .last()
        .ok_or_else(|| SkillError::invalid_input("field_path is empty"))?;
    if !current.is_object() {
        *current = Value::Object(Map::new());
    }
    current
        .as_object_mut()
        .ok_or_else(|| SkillError::invalid_data("expected object while setting field"))?
        .insert((*last).to_string(), value);
    Ok(())
}

fn set_toml_field(doc: &mut DocumentMut, field_path: &str, value: &Value) -> SkillResult<()> {
    let segments = split_field_path(field_path);
    if segments.is_empty() {
        return Err(SkillError::invalid_input("field_path is empty"));
    }
    let item = toml_item_from_json(value)?;
    set_toml_item(doc.as_item_mut(), &segments, item)
}

fn set_toml_item(parent: &mut Item, segments: &[&str], value: Item) -> SkillResult<()> {
    if parent.is_none() {
        *parent = Item::Table(toml_edit::Table::new());
    }
    if segments.len() == 1 {
        let table_like = parent
            .as_table_like_mut()
            .ok_or_else(|| SkillError::invalid_data("field_path parent is not a table"))?;
        table_like.insert(segments[0], value);
        return Ok(());
    }
    let segment = segments[0];
    let table_like = parent
        .as_table_like_mut()
        .ok_or_else(|| SkillError::invalid_data("field_path parent is not a table"))?;
    let needs_create = table_like.get(segment).is_none_or(Item::is_none);
    if needs_create {
        table_like.insert(segment, Item::Table(toml_edit::Table::new()));
    }
    let child = table_like
        .get_mut(segment)
        .ok_or_else(|| SkillError::invalid_data("failed to create TOML parent table"))?;
    if !child.is_table_like() {
        return Err(SkillError::invalid_data(format!(
            "field_path parent `{segment}` is not a table"
        )));
    }
    set_toml_item(child, &segments[1..], value)
}

fn toml_item_from_json(value: &Value) -> SkillResult<Item> {
    match value {
        Value::Null => Err(SkillError::invalid_input("null cannot be written to TOML")),
        Value::Bool(v) => Ok(toml_value_item(*v)),
        Value::Number(n) => {
            if let Some(v) = n.as_i64() {
                Ok(toml_value_item(v))
            } else if let Some(v) = n.as_u64() {
                if let Ok(v) = i64::try_from(v) {
                    Ok(toml_value_item(v))
                } else {
                    Err(SkillError::invalid_input("TOML integer is too large"))
                }
            } else if let Some(v) = n.as_f64() {
                Ok(toml_value_item(v))
            } else {
                Err(SkillError::invalid_input("unsupported number"))
            }
        }
        Value::String(v) => Ok(toml_value_item(v.clone())),
        Value::Array(values) => {
            let toml_value: toml::Value = serde_json::from_value(Value::Array(values.clone()))
                .map_err(|err| SkillError::invalid_input(format!("invalid TOML array: {err}")))?;
            let text = toml::to_string(&toml_value).map_err(|err| {
                SkillError::invalid_input(format!("serialize TOML array failed: {err}"))
            })?;
            let wrapped = format!("value = {text}");
            let doc = wrapped.parse::<DocumentMut>().map_err(|err| {
                SkillError::invalid_input(format!("parse TOML array failed: {err}"))
            })?;
            Ok(doc["value"].clone())
        }
        Value::Object(map) => {
            let toml_value: toml::Value = serde_json::from_value(Value::Object(map.clone()))
                .map_err(|err| SkillError::invalid_input(format!("invalid TOML object: {err}")))?;
            let text = toml::to_string(&toml_value).map_err(|err| {
                SkillError::invalid_input(format!("serialize TOML object failed: {err}"))
            })?;
            let wrapped = format!(
                "value = {{ {} }}",
                text.replace('\n', ", ").trim_end_matches(", ")
            );
            let doc = wrapped.parse::<DocumentMut>().map_err(|err| {
                SkillError::invalid_input(format!("parse TOML object failed: {err}"))
            })?;
            Ok(doc["value"].clone())
        }
    }
}

fn coerce_value_like_existing(new_value: Value, old_value: Option<&Value>) -> Value {
    let Some(old_value) = old_value else {
        return new_value;
    };
    let Value::String(text) = &new_value else {
        return new_value;
    };
    let trimmed = text.trim();
    match old_value {
        Value::Bool(_) => match trimmed.to_ascii_lowercase().as_str() {
            "true" => Value::Bool(true),
            "false" => Value::Bool(false),
            _ => new_value,
        },
        Value::Number(old) if old.is_i64() || old.is_u64() => trimmed
            .parse::<i64>()
            .ok()
            .map(|v| json!(v))
            .unwrap_or(new_value),
        Value::Number(old) if old.is_f64() => trimmed
            .parse::<f64>()
            .ok()
            .map(|v| json!(v))
            .unwrap_or(new_value),
        _ => new_value,
    }
}

fn json_value_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn json_value_to_text(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Bool(v) => v.to_string(),
        Value::Number(v) => v.to_string(),
        _ => value.to_string(),
    }
}

fn is_sensitive_field_path(field_path: &str) -> bool {
    field_path
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .any(|token| {
            matches!(
                token.to_ascii_lowercase().as_str(),
                "key" | "api_key" | "token" | "secret" | "password" | "credential"
            )
        })
}

fn redact_if_sensitive(sensitive: bool, value: Value) -> Value {
    if sensitive && !value.is_null() {
        Value::String("<redacted>".to_string())
    } else {
        value
    }
}

fn has_real_token(v: Option<&str>) -> bool {
    let Some(s) = v else { return false };
    let t = s.trim();
    if t.is_empty() {
        return false;
    }
    !t.starts_with("REPLACE_ME_") && t != "<redacted>"
}

fn restart_recommended_for_path(path: &str) -> bool {
    path.ends_with(".toml") || path.ends_with(".json")
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
