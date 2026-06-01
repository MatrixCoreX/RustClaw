use std::collections::HashSet;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::Duration;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use claw_core::prompt_layers;
use regex::Regex;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

#[derive(Debug, Deserialize)]
struct Req {
    request_id: String,
    args: Value,
    /// Generic runner context from `clawd` / `skill-runner` (not `args._memory`).
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

#[derive(Debug, Clone, Deserialize, Default)]
struct RootConfig {
    #[serde(default)]
    llm: LlmConfig,
    #[serde(default)]
    image_vision: ImageSkillConfig,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct LlmConfig {
    #[serde(default)]
    selected_vendor: Option<String>,
    #[serde(default)]
    openai: Option<VendorConfig>,
    #[serde(default)]
    google: Option<VendorConfig>,
    #[serde(default)]
    anthropic: Option<VendorConfig>,
    #[serde(default)]
    grok: Option<VendorConfig>,
    #[serde(default)]
    deepseek: Option<VendorConfig>,
    #[serde(default)]
    qwen: Option<VendorConfig>,
    #[serde(default)]
    minimax: Option<VendorConfig>,
    #[serde(default)]
    mimo: Option<VendorConfig>,
}

#[derive(Debug, Clone, Deserialize)]
struct VendorConfig {
    base_url: String,
    #[serde(default)]
    api_key: String,
    model: String,
    #[serde(default)]
    timeout_seconds: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct ImageSkillConfig {
    #[serde(default)]
    default_vendor: Option<String>,
    #[serde(default)]
    default_model: Option<String>,
    #[serde(default)]
    models: Option<Vec<String>>,
    #[serde(default)]
    openai_models: Option<Vec<String>>,
    #[serde(default)]
    google_models: Option<Vec<String>>,
    #[serde(default)]
    anthropic_models: Option<Vec<String>>,
    #[serde(default)]
    grok_models: Option<Vec<String>>,
    #[serde(default)]
    deepseek_models: Option<Vec<String>>,
    #[serde(default)]
    qwen_models: Option<Vec<String>>,
    #[serde(default)]
    minimax_models: Option<Vec<String>>,
    #[serde(default)]
    mimo_models: Option<Vec<String>>,
    #[serde(default)]
    timeout_seconds: Option<u64>,
    #[serde(default)]
    max_images: Option<usize>,
    #[serde(default)]
    max_input_bytes: Option<usize>,
    #[serde(default)]
    adapter_mode: Option<String>,
    #[serde(default)]
    providers: ImageProviderOverrides,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct ImageProviderOverrides {
    #[serde(default)]
    openai: Option<VendorConfig>,
    #[serde(default)]
    google: Option<VendorConfig>,
    #[serde(default)]
    anthropic: Option<VendorConfig>,
    #[serde(default)]
    grok: Option<VendorConfig>,
    #[serde(default)]
    deepseek: Option<VendorConfig>,
    #[serde(default)]
    qwen: Option<VendorConfig>,
    #[serde(default)]
    minimax: Option<VendorConfig>,
    #[serde(default)]
    mimo: Option<VendorConfig>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum VendorKind {
    OpenAI,
    Google,
    Anthropic,
    Grok,
    DeepSeek,
    Qwen,
    MiniMax,
    Mimo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AdapterMode {
    Auto,
    Native,
    Compat,
}

#[derive(Debug, Clone)]
enum ImageSource {
    Path(PathBuf),
    Url(String),
    Base64(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ImageDescribeOut {
    summary: String,
    #[serde(default)]
    objects: Vec<String>,
    #[serde(default)]
    visible_text: Vec<String>,
    #[serde(default)]
    uncertainties: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ImageCompareOut {
    summary: String,
    #[serde(default)]
    similarities: Vec<String>,
    #[serde(default)]
    differences: Vec<String>,
    #[serde(default)]
    notable_changes: Vec<String>,
    #[serde(default)]
    uncertainties: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ImageScreenshotSummaryOut {
    purpose: String,
    #[serde(default)]
    critical_text: Vec<String>,
    #[serde(default)]
    warnings: Vec<String>,
    #[serde(default)]
    next_actions: Vec<String>,
    #[serde(default)]
    uncertainties: Vec<String>,
}

#[derive(Debug, Clone)]
enum StructuredNarrativeActionOutput {
    Describe(ImageDescribeOut),
    Compare(ImageCompareOut),
    ScreenshotSummary(ImageScreenshotSummaryOut),
}

fn main() -> anyhow::Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let cfg = load_root_config();
    let workspace_root = workspace_root();

    for line in stdin.lock().lines() {
        let line = line?;
        let parsed: Result<Req, _> = serde_json::from_str(&line);
        let resp = match parsed {
            Ok(req) => match execute(&cfg, &workspace_root, req.args, req.context.as_ref()) {
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
                    extra: None,
                    error_text: Some(err),
                },
            },
            Err(err) => Resp {
                request_id: "unknown".to_string(),
                status: "error".to_string(),
                text: String::new(),
                extra: None,
                error_text: Some(format!("invalid input: {err}")),
            },
        };
        writeln!(stdout, "{}", serde_json::to_string(&resp)?)?;
        stdout.flush()?;
    }
    Ok(())
}

fn execute(
    cfg: &RootConfig,
    workspace_root: &Path,
    args: Value,
    runner_context: Option<&Value>,
) -> Result<(String, Value), String> {
    let obj = args
        .as_object()
        .ok_or_else(|| "args must be object".to_string())?;

    let action = parse_action(obj)?;
    let images = parse_images(obj, workspace_root)?;
    let max_images = cfg.image_vision.max_images.unwrap_or(6).max(1);
    if images.is_empty() {
        return Err("at least one image is required".to_string());
    }
    if images.len() > max_images {
        return Err(format!(
            "too many images: {}, max={max_images}",
            images.len()
        ));
    }
    if action == "compare" && images.len() < 2 {
        return Err("compare requires at least two images".to_string());
    }

    let detail_level = obj
        .get("detail_level")
        .and_then(|v| v.as_str())
        .unwrap_or("normal");
    let timeout_seconds = obj
        .get("timeout_seconds")
        .and_then(|v| v.as_u64())
        .unwrap_or_else(|| cfg.image_vision.timeout_seconds.unwrap_or(90))
        .max(5)
        .min(300);
    let response_language = resolve_effective_response_language(
        cfg,
        workspace_root,
        obj,
        runner_context,
        timeout_seconds,
    );
    let schema = obj.get("schema").cloned();
    let user_instruction = obj
        .get("instruction")
        .or_else(|| obj.get("query"))
        .or_else(|| obj.get("text"))
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty());
    let max_input_bytes = cfg.image_vision.max_input_bytes.unwrap_or(10 * 1024 * 1024);

    let requested_vendor = obj.get("vendor").and_then(|v| v.as_str());
    let requested_model = obj.get("model").and_then(|v| v.as_str());
    let vendors = vendor_order(
        requested_vendor,
        cfg.image_vision.default_vendor.as_deref(),
        cfg.llm.selected_vendor.as_deref(),
    );
    if vendors.is_empty() {
        return Err("no vendor configured".to_string());
    }

    let mut attempt_errors = Vec::new();
    for vendor in vendors {
        let prompt = build_prompt(
            workspace_root,
            prompt_vendor_name_for_vendor(vendor),
            &action,
            detail_level,
            schema.as_ref(),
            response_language.as_deref(),
            user_instruction,
        );
        let config_default_model =
            select_model_override(&cfg.image_vision, vendor, requested_model);
        match call_vendor_vision(
            vendor,
            cfg,
            config_default_model,
            timeout_seconds,
            &prompt,
            &images,
            max_input_bytes,
        ) {
            Ok((text, model, model_kind)) => {
                let structured = parse_structured_narrative_action_output(action.as_str(), &text);
                let text = structured
                    .as_ref()
                    .map(|out| {
                        render_structured_narrative_action_output(out, response_language.as_deref())
                    })
                    .unwrap_or(text);
                let text = maybe_rewrite_image_vision_text_for_target_language(
                    cfg,
                    workspace_root,
                    action.as_str(),
                    response_language.as_deref(),
                    text,
                    timeout_seconds,
                );
                let text = strip_think_blocks(&text).trim().to_string();
                let mut extra = json!({
                    "provider": vendor_name(vendor),
                    "model": model,
                    "model_kind": model_kind,
                    "latency_ms": 0,
                    "outputs": [{"type":"text","preview": truncate(&text, 800)}],
                    "schema_validated": structured.is_some()
                });
                if let Some(structured) = structured {
                    extra["structured"] = structured.to_json_value();
                }
                return Ok((text, extra));
            }
            Err(err) => {
                attempt_errors.push(format!("{}: {}", vendor_name(vendor), truncate(&err, 300)));
            }
        }
    }

    Err(format!(
        "all providers failed: {}",
        attempt_errors.join("; ")
    ))
}

fn select_model_override<'a>(
    cfg: &'a ImageSkillConfig,
    vendor: VendorKind,
    requested_model: Option<&'a str>,
) -> Option<&'a str> {
    if let Some(v) = requested_model.map(str::trim).filter(|v| !v.is_empty()) {
        return Some(v);
    }
    if let Some(v) = first_model_from_list(vendor_models(cfg, vendor)) {
        return Some(v);
    }
    if cfg.default_vendor.as_deref().and_then(parse_vendor) == Some(vendor) {
        if let Some(v) = cfg
            .default_model
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            return Some(v);
        }
        return first_model_from_list(cfg.models.as_ref());
    }
    None
}

fn first_model_from_list(models: Option<&Vec<String>>) -> Option<&str> {
    models.and_then(|list| list.iter().map(|s| s.trim()).find(|v| !v.is_empty()))
}

fn vendor_models<'a>(cfg: &'a ImageSkillConfig, vendor: VendorKind) -> Option<&'a Vec<String>> {
    match vendor {
        VendorKind::OpenAI => cfg.openai_models.as_ref(),
        VendorKind::Google => cfg.google_models.as_ref(),
        VendorKind::Anthropic => cfg.anthropic_models.as_ref(),
        VendorKind::Grok => cfg.grok_models.as_ref(),
        VendorKind::DeepSeek => cfg.deepseek_models.as_ref(),
        VendorKind::Qwen => cfg.qwen_models.as_ref(),
        VendorKind::MiniMax => cfg.minimax_models.as_ref(),
        VendorKind::Mimo => cfg.mimo_models.as_ref(),
    }
}

fn parse_action(obj: &Map<String, Value>) -> Result<String, String> {
    let action = obj
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("describe")
        .trim()
        .to_ascii_lowercase();
    match action.as_str() {
        "analyze" => Ok("describe".to_string()),
        "describe" | "extract" | "compare" | "screenshot_summary" => Ok(action),
        _ => Err("unsupported action; use describe|extract|compare|screenshot_summary".to_string()),
    }
}

fn parse_images(
    obj: &Map<String, Value>,
    workspace_root: &Path,
) -> Result<Vec<ImageSource>, String> {
    let mut out = Vec::new();
    if let Some(arr) = obj.get("images").and_then(|v| v.as_array()) {
        for item in arr {
            out.push(parse_one_image(item, workspace_root)?);
        }
    } else if let Some(v) = obj.get("image") {
        out.push(parse_one_image(v, workspace_root)?);
    }
    Ok(out)
}

fn parse_one_image(v: &Value, workspace_root: &Path) -> Result<ImageSource, String> {
    if let Some(s) = v.as_str() {
        return parse_image_from_str(s, workspace_root);
    }
    let obj = v
        .as_object()
        .ok_or_else(|| "image entry must be string or object".to_string())?;
    if let Some(s) = obj.get("path").and_then(|v| v.as_str()) {
        let p = to_workspace_path(workspace_root, s)?;
        return Ok(ImageSource::Path(p));
    }
    if let Some(s) = obj.get("url").and_then(|v| v.as_str()) {
        if !s.starts_with("http://") && !s.starts_with("https://") {
            return Err("image.url must start with http:// or https://".to_string());
        }
        return Ok(ImageSource::Url(s.to_string()));
    }
    if let Some(s) = obj.get("base64").and_then(|v| v.as_str()) {
        return Ok(ImageSource::Base64(s.to_string()));
    }
    Err("image object requires path|url|base64".to_string())
}

fn parse_image_from_str(s: &str, workspace_root: &Path) -> Result<ImageSource, String> {
    let t = s.trim();
    if t.starts_with("http://") || t.starts_with("https://") {
        return Ok(ImageSource::Url(t.to_string()));
    }
    if t.starts_with("data:image/") {
        return Ok(ImageSource::Base64(t.to_string()));
    }
    Ok(ImageSource::Path(to_workspace_path(workspace_root, t)?))
}

fn to_workspace_path(workspace_root: &Path, input: &str) -> Result<PathBuf, String> {
    let p = Path::new(input);
    let joined = if p.is_absolute() {
        p.to_path_buf()
    } else {
        workspace_root.join(p)
    };
    if !joined.starts_with(workspace_root) {
        return Err("image path is outside workspace".to_string());
    }
    Ok(joined)
}

fn non_empty_str_from_value(v: Option<&Value>) -> Option<String> {
    v.and_then(|x| x.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// Matches `clawd` `preferred_response_language` ordering: last matching preference wins.
fn language_from_memory_preferences_map(prefs: &Map<String, Value>) -> Option<String> {
    let pairs: Vec<_> = prefs.iter().collect();
    for (k, v) in pairs.into_iter().rev() {
        let kt = k.trim();
        if kt == "response_language" || kt == "language" {
            if let Some(s) = v.as_str().map(|s| s.trim()).filter(|s| !s.is_empty()) {
                return Some(s.to_string());
            }
        }
    }
    None
}

fn language_from_json_value(v: &Value) -> Option<String> {
    v.get("language")
        .and_then(|x| x.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && s.to_ascii_lowercase() != "unknown")
}

fn language_infer_schema() -> &'static Value {
    LANGUAGE_INFER_SCHEMA.get_or_init(|| {
        serde_json::from_str::<Value>(LANGUAGE_INFER_SCHEMA_RAW)
            .expect("language_infer schema must be valid JSON")
    })
}

fn image_describe_schema() -> &'static Value {
    IMAGE_DESCRIBE_SCHEMA.get_or_init(|| {
        serde_json::from_str::<Value>(IMAGE_DESCRIBE_SCHEMA_RAW)
            .expect("image_describe schema must be valid JSON")
    })
}

fn image_compare_schema() -> &'static Value {
    IMAGE_COMPARE_SCHEMA.get_or_init(|| {
        serde_json::from_str::<Value>(IMAGE_COMPARE_SCHEMA_RAW)
            .expect("image_compare schema must be valid JSON")
    })
}

fn image_screenshot_summary_schema() -> &'static Value {
    IMAGE_SCREENSHOT_SUMMARY_SCHEMA.get_or_init(|| {
        serde_json::from_str::<Value>(IMAGE_SCREENSHOT_SUMMARY_SCHEMA_RAW)
            .expect("image_screenshot_summary schema must be valid JSON")
    })
}

fn schema_requires_field(schema: &Value, name: &str) -> bool {
    schema
        .get("required")
        .and_then(|v| v.as_array())
        .map(|fields| fields.iter().any(|field| field.as_str() == Some(name)))
        .unwrap_or(false)
}

fn schema_declared_fields(schema: &Value) -> Option<&serde_json::Map<String, Value>> {
    schema.get("properties")?.as_object()
}

fn schema_allows_additional_properties(schema: &Value) -> bool {
    schema
        .get("additionalProperties")
        .and_then(|v| v.as_bool())
        .unwrap_or(true)
}

fn schema_string_is_valid(schema: &Value, name: &str, value: &str) -> bool {
    let property = match schema.get("properties").and_then(|v| v.get(name)) {
        Some(property) => property,
        None => return false,
    };
    if property.get("type").and_then(|v| v.as_str()) != Some("string") {
        return false;
    }
    let min_length = property
        .get("minLength")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;
    value.chars().count() >= min_length
}

fn parse_schema_validated_json_object(
    raw: &str,
    schema: &Value,
    label: &str,
) -> Result<Value, String> {
    let trimmed = raw.trim();
    let candidate = if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        value
    } else {
        let start = trimmed
            .find('{')
            .ok_or_else(|| format!("{label} is not valid JSON"))?;
        let end = trimmed
            .rfind('}')
            .ok_or_else(|| format!("{label} is not valid JSON"))?;
        if end <= start {
            return Err(format!("{label} is not valid JSON"));
        }
        serde_json::from_str::<Value>(&trimmed[start..=end])
            .map_err(|err| format!("{label} JSON parse failed: {err}"))?
    };
    validate_value_against_schema(&candidate, schema, "$")
        .map_err(|err| format!("{label} schema invalid: {err}"))?;
    Ok(candidate)
}

fn validate_value_against_schema(value: &Value, schema: &Value, path: &str) -> Result<(), String> {
    if let Some(kind) = schema.get("type").and_then(|v| v.as_str()) {
        match kind {
            "object" => {
                let object = value
                    .as_object()
                    .ok_or_else(|| format!("{path}: expected object"))?;
                let declared_fields = schema_declared_fields(schema);
                if !schema_allows_additional_properties(schema) {
                    let declared = declared_fields
                        .ok_or_else(|| format!("{path}: schema missing properties"))?;
                    if let Some(extra) = object.keys().find(|key| !declared.contains_key(*key)) {
                        return Err(format!("{path}.{extra}: unexpected field"));
                    }
                }
                if let Some(required) = schema.get("required").and_then(|v| v.as_array()) {
                    for field in required.iter().filter_map(|v| v.as_str()) {
                        if !object.contains_key(field) {
                            return Err(format!("{path}.{field}: missing required field"));
                        }
                    }
                }
                if let Some(properties) = declared_fields {
                    for (field, property_schema) in properties {
                        if let Some(field_value) = object.get(field) {
                            validate_value_against_schema(
                                field_value,
                                property_schema,
                                &format!("{path}.{field}"),
                            )?;
                        }
                    }
                }
            }
            "array" => {
                let items = value
                    .as_array()
                    .ok_or_else(|| format!("{path}: expected array"))?;
                if let Some(item_schema) = schema.get("items") {
                    for (idx, item) in items.iter().enumerate() {
                        validate_value_against_schema(
                            item,
                            item_schema,
                            &format!("{path}[{idx}]"),
                        )?;
                    }
                }
            }
            "string" => {
                let s = value
                    .as_str()
                    .ok_or_else(|| format!("{path}: expected string"))?;
                let min_length = schema
                    .get("minLength")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as usize;
                if s.chars().count() < min_length {
                    return Err(format!("{path}: shorter than minLength {min_length}"));
                }
            }
            other => return Err(format!("{path}: unsupported schema type {other}")),
        }
    }
    Ok(())
}

fn parse_structured_narrative_action_output(
    action: &str,
    raw: &str,
) -> Option<StructuredNarrativeActionOutput> {
    let (schema, label) = match action {
        "describe" => (image_describe_schema(), "image describe output"),
        "compare" => (image_compare_schema(), "image compare output"),
        "screenshot_summary" => (
            image_screenshot_summary_schema(),
            "image screenshot summary output",
        ),
        _ => return None,
    };
    let candidate = parse_schema_validated_json_object(raw, schema, label).ok()?;
    match action {
        "describe" => serde_json::from_value::<ImageDescribeOut>(candidate)
            .ok()
            .map(StructuredNarrativeActionOutput::Describe),
        "compare" => serde_json::from_value::<ImageCompareOut>(candidate)
            .ok()
            .map(StructuredNarrativeActionOutput::Compare),
        "screenshot_summary" => serde_json::from_value::<ImageScreenshotSummaryOut>(candidate)
            .ok()
            .map(StructuredNarrativeActionOutput::ScreenshotSummary),
        _ => None,
    }
}

fn contains_cjk(text: &str) -> bool {
    text.chars()
        .any(|ch| ('\u{4E00}'..='\u{9FFF}').contains(&ch))
}

fn should_use_zh_labels(response_language: Option<&str>, primary_text: &str) -> bool {
    response_language
        .map(|lang| lang.trim().to_ascii_lowercase().starts_with("zh"))
        .unwrap_or_else(|| contains_cjk(primary_text))
}

fn list_separator(use_zh: bool) -> &'static str {
    if use_zh {
        "、"
    } else {
        ", "
    }
}

fn join_non_empty_items(items: &[String], separator: &str) -> Option<String> {
    let filtered = items
        .iter()
        .map(|item| item.trim())
        .filter(|item| !item.is_empty())
        .collect::<Vec<_>>();
    if filtered.is_empty() {
        None
    } else {
        Some(filtered.join(separator))
    }
}

fn push_labeled_list(lines: &mut Vec<String>, label: &str, items: &[String], separator: &str) {
    if let Some(joined) = join_non_empty_items(items, separator) {
        lines.push(format!("{label}{joined}"));
    }
}

fn render_structured_narrative_action_output(
    output: &StructuredNarrativeActionOutput,
    response_language: Option<&str>,
) -> String {
    let primary_text = match output {
        StructuredNarrativeActionOutput::Describe(out) => out.summary.as_str(),
        StructuredNarrativeActionOutput::Compare(out) => out.summary.as_str(),
        StructuredNarrativeActionOutput::ScreenshotSummary(out) => out.purpose.as_str(),
    };
    let use_zh = should_use_zh_labels(response_language, primary_text);
    let separator = list_separator(use_zh);
    let mut lines = Vec::new();
    match output {
        StructuredNarrativeActionOutput::Describe(out) => {
            lines.push(out.summary.trim().to_string());
            push_labeled_list(
                &mut lines,
                if use_zh { "对象：" } else { "Objects: " },
                &out.objects,
                separator,
            );
            push_labeled_list(
                &mut lines,
                if use_zh {
                    "可见文字："
                } else {
                    "Visible text: "
                },
                &out.visible_text,
                separator,
            );
            push_labeled_list(
                &mut lines,
                if use_zh {
                    "不确定点："
                } else {
                    "Uncertainties: "
                },
                &out.uncertainties,
                separator,
            );
        }
        StructuredNarrativeActionOutput::Compare(out) => {
            lines.push(out.summary.trim().to_string());
            push_labeled_list(
                &mut lines,
                if use_zh {
                    "相同点："
                } else {
                    "Similarities: "
                },
                &out.similarities,
                separator,
            );
            push_labeled_list(
                &mut lines,
                if use_zh { "差异：" } else { "Differences: " },
                &out.differences,
                separator,
            );
            push_labeled_list(
                &mut lines,
                if use_zh {
                    "显著变化："
                } else {
                    "Notable changes: "
                },
                &out.notable_changes,
                separator,
            );
            push_labeled_list(
                &mut lines,
                if use_zh {
                    "不确定点："
                } else {
                    "Uncertainties: "
                },
                &out.uncertainties,
                separator,
            );
        }
        StructuredNarrativeActionOutput::ScreenshotSummary(out) => {
            lines.push(format!(
                "{}{}",
                if use_zh { "用途：" } else { "Purpose: " },
                out.purpose.trim()
            ));
            push_labeled_list(
                &mut lines,
                if use_zh {
                    "关键信息："
                } else {
                    "Critical text: "
                },
                &out.critical_text,
                separator,
            );
            push_labeled_list(
                &mut lines,
                if use_zh { "警告：" } else { "Warnings: " },
                &out.warnings,
                separator,
            );
            push_labeled_list(
                &mut lines,
                if use_zh {
                    "下一步："
                } else {
                    "Next actions: "
                },
                &out.next_actions,
                separator,
            );
            push_labeled_list(
                &mut lines,
                if use_zh {
                    "不确定点："
                } else {
                    "Uncertainties: "
                },
                &out.uncertainties,
                separator,
            );
        }
    }
    lines.join("\n")
}

fn parse_language_choice_from_json_value(v: &Value) -> Option<String> {
    let schema = language_infer_schema();
    let object = v.as_object()?;
    if !schema_allows_additional_properties(schema) {
        let declared_fields = schema_declared_fields(schema)?;
        if object.keys().any(|key| !declared_fields.contains_key(key)) {
            return None;
        }
    }
    if schema_requires_field(schema, "language") && !object.contains_key("language") {
        return None;
    }
    let language = object.get("language")?.as_str()?.trim();
    if !schema_string_is_valid(schema, "language", language) {
        return None;
    }
    language_from_json_value(v)
}

fn parse_language_choice_from_llm(raw: &str) -> Option<String> {
    let t = raw.trim();
    if let Ok(v) = serde_json::from_str::<Value>(t) {
        return parse_language_choice_from_json_value(&v);
    }
    let start = t.find('{')?;
    let end = t.rfind('}')?;
    if end <= start {
        return None;
    }
    let slice = &t[start..=end];
    let v: Value = serde_json::from_str(slice).ok()?;
    parse_language_choice_from_json_value(&v)
}

fn preferred_prompt_vendor(cfg: &RootConfig) -> &str {
    cfg.llm
        .selected_vendor
        .as_deref()
        .or(cfg.image_vision.default_vendor.as_deref())
        .unwrap_or("default")
}

fn load_language_infer_prompt_template(workspace_root: &Path, prompt_vendor: &str) -> String {
    prompt_layers::load_prompt_template_for_vendor(
        workspace_root,
        prompt_vendor,
        "prompts/language_infer_prompt.md",
        DEFAULT_LANGUAGE_INFER_PROMPT_TEMPLATE,
    )
    .0
}

/// When explicit language args are absent, derive target language from generic runner `context`,
/// injected `args._memory`, and (last resort) an OpenAI-compatible chat pass over `_memory.context`.
fn resolve_effective_response_language(
    cfg: &RootConfig,
    workspace_root: &Path,
    args_obj: &Map<String, Value>,
    runner_context: Option<&Value>,
    task_timeout_seconds: u64,
) -> Option<String> {
    if let Some(s) = non_empty_str_from_value(args_obj.get("response_language")) {
        return Some(s);
    }
    if let Some(s) = non_empty_str_from_value(args_obj.get("language")) {
        return Some(s);
    }
    if let Some(ctx) = runner_context.and_then(|c| c.as_object()) {
        if let Some(s) = non_empty_str_from_value(ctx.get("response_language"))
            .or_else(|| non_empty_str_from_value(ctx.get("language")))
        {
            return Some(s);
        }
    }
    let Some(mem_obj) = args_obj.get("_memory").and_then(|m| m.as_object()) else {
        return None;
    };
    if let Some(s) = non_empty_str_from_value(mem_obj.get("lang_hint")) {
        return Some(s);
    }
    if let Some(prefs) = mem_obj.get("preferences").and_then(|p| p.as_object()) {
        if let Some(s) = language_from_memory_preferences_map(prefs) {
            return Some(s);
        }
    }
    let snippets = mem_obj
        .get("context")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim();
    if snippets.is_empty() || snippets == "<none>" {
        return None;
    }
    let infer_timeout = task_timeout_seconds.clamp(10, 90).min(30);
    infer_language_from_memory_snippets_llm(cfg, workspace_root, snippets, infer_timeout)
}

fn infer_language_from_memory_snippets_llm(
    cfg: &RootConfig,
    workspace_root: &Path,
    memory_snippets: &str,
    infer_timeout_secs: u64,
) -> Option<String> {
    let template =
        load_language_infer_prompt_template(workspace_root, preferred_prompt_vendor(cfg));
    let prompt = template.replace("__MEMORY_SNIPPETS__", memory_snippets);
    let t = infer_timeout_secs.clamp(5, 45).min(25);
    for vk in vendor_order(
        None,
        cfg.image_vision.default_vendor.as_deref(),
        cfg.llm.selected_vendor.as_deref(),
    ) {
        let Ok((vname, vcfg)) = resolve_vendor_config(cfg, vk) else {
            continue;
        };
        if check_api_key(vname, &vcfg.api_key).is_err() {
            continue;
        }
        let model = vcfg.model.trim();
        if model.is_empty() {
            continue;
        }
        if let Ok(out) =
            openai_compat_chat_rewrite(&vcfg, model, &prompt, t, vk == VendorKind::Mimo)
        {
            if let Some(lang) = parse_language_choice_from_llm(&out) {
                return Some(lang);
            }
        }
    }
    None
}

fn build_prompt(
    workspace_root: &Path,
    prompt_vendor: &str,
    action: &str,
    detail_level: &str,
    schema: Option<&Value>,
    response_language: Option<&str>,
    user_instruction: Option<&str>,
) -> String {
    let template = load_image_vision_prompt_template(workspace_root, prompt_vendor);
    let mut task_instruction =
        action_instruction(workspace_root, prompt_vendor, action, detail_level, schema);
    if let Some(extra) = user_instruction {
        task_instruction.push_str("\n\nAdditional user instruction:\n");
        task_instruction.push_str(extra);
    }
    let schema_hint = schema
        .map(|s| s.to_string())
        .unwrap_or_else(|| "none".to_string());
    let language_hint = response_language
        .map(|s| {
            load_prompt_fragment(
                workspace_root,
                prompt_vendor,
                "prompts/image_vision_language_hint_with_target.md",
                DEFAULT_IMAGE_VISION_LANGUAGE_HINT_WITH_TARGET_TEMPLATE,
            )
            .replace("__RESPONSE_LANGUAGE__", s)
        })
        .unwrap_or_else(|| {
            load_prompt_fragment(
                workspace_root,
                prompt_vendor,
                "prompts/image_vision_language_hint_default.md",
                DEFAULT_IMAGE_VISION_LANGUAGE_HINT_DEFAULT_TEMPLATE,
            )
        });
    template
        .replace("__ACTION__", action)
        .replace("__DETAIL_LEVEL__", detail_level)
        .replace("__TASK_INSTRUCTION__", &task_instruction)
        .replace("__SCHEMA_HINT__", &schema_hint)
        .replace("__LANGUAGE_HINT__", &language_hint)
}

fn load_image_output_rewrite_prompt_template(workspace_root: &Path, prompt_vendor: &str) -> String {
    prompt_layers::load_prompt_template_for_vendor(
        workspace_root,
        prompt_vendor,
        "prompts/image_output_rewrite_prompt.md",
        include_str!("../../../../prompts/layers/overlays/image_output_rewrite_prompt.md"),
    )
    .0
}

fn openai_compat_chat_rewrite(
    vcfg: &VendorConfig,
    model: &str,
    user_prompt: &str,
    timeout_secs: u64,
    include_api_key_header: bool,
) -> Result<String, String> {
    let client = Client::builder()
        .timeout(Duration::from_secs(timeout_secs.clamp(5, 120)))
        .build()
        .map_err(|e| format!("http client: {e}"))?;
    let url = format!("{}/chat/completions", trim_trailing_slash(&vcfg.base_url));
    let body = json!({
        "model": model,
        "messages": [{"role": "user", "content": user_prompt}],
        "temperature": 0.0
    });
    let mut request = client.post(&url).bearer_auth(vcfg.api_key.trim());
    if include_api_key_header {
        request = request.header("api-key", vcfg.api_key.trim());
    }
    let resp = request
        .json(&body)
        .send()
        .map_err(|e| format!("chat completion request failed: {e}"))?;
    if !resp.status().is_success() {
        let t = resp.text().unwrap_or_default();
        return Err(format!("chat completion HTTP error: {t}"));
    }
    let v: Value = resp
        .json()
        .map_err(|e| format!("parse chat response: {e}"))?;
    Ok(v.get("choices")
        .and_then(|c| c.as_array())
        .and_then(|a| a.first())
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string())
}

/// Same-turn alignment pass formerly done in `clawd`: rewrite vision text to `response_language`
/// for narrative actions. On failure, returns `vision_output` unchanged.
fn maybe_rewrite_image_vision_text_for_target_language(
    cfg: &RootConfig,
    workspace_root: &Path,
    action: &str,
    target_language: Option<&str>,
    vision_output: String,
    task_timeout_seconds: u64,
) -> String {
    let Some(lang) = target_language.map(str::trim).filter(|s| !s.is_empty()) else {
        return vision_output;
    };
    if !matches!(action, "describe" | "compare" | "screenshot_summary") {
        return vision_output;
    }
    if vision_output.trim().is_empty() {
        return vision_output;
    }
    let template =
        load_image_output_rewrite_prompt_template(workspace_root, preferred_prompt_vendor(cfg));
    let prompt = template
        .replace("__TARGET_LANGUAGE__", lang)
        .replace("__ORIGINAL_OUTPUT__", &vision_output);
    let rewrite_timeout = task_timeout_seconds.clamp(10, 90).min(45);
    for vk in vendor_order(
        None,
        cfg.image_vision.default_vendor.as_deref(),
        cfg.llm.selected_vendor.as_deref(),
    ) {
        let Ok((vname, vcfg)) = resolve_vendor_config(cfg, vk) else {
            continue;
        };
        if check_api_key(vname, &vcfg.api_key).is_err() {
            continue;
        }
        let model = vcfg.model.trim();
        if model.is_empty() {
            continue;
        }
        if let Ok(out) = openai_compat_chat_rewrite(
            &vcfg,
            model,
            &prompt,
            rewrite_timeout,
            vk == VendorKind::Mimo,
        ) {
            let t = out.trim();
            if !t.is_empty() {
                return strip_think_blocks(t).trim().to_string();
            }
        }
    }
    vision_output
}

fn strip_think_blocks(text: &str) -> String {
    static THINK_RE: OnceLock<Regex> = OnceLock::new();
    THINK_RE
        .get_or_init(|| Regex::new(r"(?is)<think>.*?</think>").expect("think regex compiles"))
        .replace_all(text, "")
        .to_string()
}

fn action_instruction(
    workspace_root: &Path,
    prompt_vendor: &str,
    action: &str,
    detail_level: &str,
    schema: Option<&Value>,
) -> String {
    match action {
        "describe" => load_prompt_fragment(
            workspace_root,
            prompt_vendor,
            "prompts/image_vision_action_describe.md",
            DEFAULT_IMAGE_VISION_ACTION_DESCRIBE_TEMPLATE,
        )
        .replace("__DETAIL_LEVEL__", detail_level),
        "compare" => load_prompt_fragment(
            workspace_root,
            prompt_vendor,
            "prompts/image_vision_action_compare.md",
            DEFAULT_IMAGE_VISION_ACTION_COMPARE_TEMPLATE,
        ),
        "screenshot_summary" => load_prompt_fragment(
            workspace_root,
            prompt_vendor,
            "prompts/image_vision_action_screenshot_summary.md",
            DEFAULT_IMAGE_VISION_ACTION_SCREENSHOT_SUMMARY_TEMPLATE,
        ),
        "extract" => {
            if let Some(s) = schema {
                load_prompt_fragment(
                    workspace_root,
                    prompt_vendor,
                    "prompts/image_vision_action_extract_with_schema.md",
                    DEFAULT_IMAGE_VISION_ACTION_EXTRACT_WITH_SCHEMA_TEMPLATE,
                )
                .replace("__SCHEMA__", &s.to_string())
            } else {
                load_prompt_fragment(
                    workspace_root,
                    prompt_vendor,
                    "prompts/image_vision_action_extract_default.md",
                    DEFAULT_IMAGE_VISION_ACTION_EXTRACT_DEFAULT_TEMPLATE,
                )
            }
        }
        _ => load_prompt_fragment(
            workspace_root,
            prompt_vendor,
            "prompts/image_vision_action_fallback.md",
            DEFAULT_IMAGE_VISION_ACTION_FALLBACK_TEMPLATE,
        ),
    }
}

fn prompt_vendor_name_for_vendor(vendor: VendorKind) -> &'static str {
    match vendor {
        VendorKind::OpenAI => "openai",
        VendorKind::Google => "google",
        VendorKind::Anthropic => "claude",
        VendorKind::Grok => "grok",
        VendorKind::DeepSeek => "deepseek",
        VendorKind::Qwen => "qwen",
        VendorKind::MiniMax => "minimax",
        VendorKind::Mimo => "mimo",
    }
}

fn load_prompt_fragment(
    workspace_root: &Path,
    vendor: &str,
    relative_path: &str,
    default_template: &str,
) -> String {
    prompt_layers::load_prompt_template_for_vendor(
        workspace_root,
        vendor,
        relative_path,
        default_template,
    )
    .0
}

fn load_image_vision_prompt_template(workspace_root: &Path, vendor: &str) -> String {
    load_prompt_fragment(
        workspace_root,
        vendor,
        "prompts/image_vision_prompt.md",
        DEFAULT_IMAGE_VISION_PROMPT_TEMPLATE,
    )
}

const DEFAULT_IMAGE_VISION_PROMPT_TEMPLATE: &str =
    include_str!("../../../../prompts/layers/overlays/image_vision_prompt.md");
const DEFAULT_IMAGE_VISION_LANGUAGE_HINT_WITH_TARGET_TEMPLATE: &str =
    include_str!("../../../../prompts/layers/overlays/image_vision_language_hint_with_target.md");
const DEFAULT_IMAGE_VISION_LANGUAGE_HINT_DEFAULT_TEMPLATE: &str =
    include_str!("../../../../prompts/layers/overlays/image_vision_language_hint_default.md");
const DEFAULT_IMAGE_VISION_ACTION_DESCRIBE_TEMPLATE: &str =
    include_str!("../../../../prompts/layers/overlays/image_vision_action_describe.md");
const DEFAULT_IMAGE_VISION_ACTION_COMPARE_TEMPLATE: &str =
    include_str!("../../../../prompts/layers/overlays/image_vision_action_compare.md");
const DEFAULT_IMAGE_VISION_ACTION_SCREENSHOT_SUMMARY_TEMPLATE: &str =
    include_str!("../../../../prompts/layers/overlays/image_vision_action_screenshot_summary.md");
const DEFAULT_IMAGE_VISION_ACTION_EXTRACT_WITH_SCHEMA_TEMPLATE: &str =
    include_str!("../../../../prompts/layers/overlays/image_vision_action_extract_with_schema.md");
const DEFAULT_IMAGE_VISION_ACTION_EXTRACT_DEFAULT_TEMPLATE: &str =
    include_str!("../../../../prompts/layers/overlays/image_vision_action_extract_default.md");
const DEFAULT_IMAGE_VISION_ACTION_FALLBACK_TEMPLATE: &str =
    include_str!("../../../../prompts/layers/overlays/image_vision_action_fallback.md");
const DEFAULT_LANGUAGE_INFER_PROMPT_TEMPLATE: &str =
    include_str!("../../../../prompts/layers/overlays/language_infer_prompt.md");
const LANGUAGE_INFER_SCHEMA_RAW: &str =
    include_str!("../../../../prompts/schemas/language_infer.schema.json");
const IMAGE_DESCRIBE_SCHEMA_RAW: &str =
    include_str!("../../../../prompts/schemas/image_vision_describe.schema.json");
const IMAGE_COMPARE_SCHEMA_RAW: &str =
    include_str!("../../../../prompts/schemas/image_vision_compare.schema.json");
const IMAGE_SCREENSHOT_SUMMARY_SCHEMA_RAW: &str =
    include_str!("../../../../prompts/schemas/image_vision_screenshot_summary.schema.json");

static LANGUAGE_INFER_SCHEMA: OnceLock<Value> = OnceLock::new();
static IMAGE_DESCRIBE_SCHEMA: OnceLock<Value> = OnceLock::new();
static IMAGE_COMPARE_SCHEMA: OnceLock<Value> = OnceLock::new();
static IMAGE_SCREENSHOT_SUMMARY_SCHEMA: OnceLock<Value> = OnceLock::new();

impl StructuredNarrativeActionOutput {
    fn to_json_value(&self) -> Value {
        match self {
            StructuredNarrativeActionOutput::Describe(out) => {
                serde_json::to_value(out).unwrap_or(Value::Null)
            }
            StructuredNarrativeActionOutput::Compare(out) => {
                serde_json::to_value(out).unwrap_or(Value::Null)
            }
            StructuredNarrativeActionOutput::ScreenshotSummary(out) => {
                serde_json::to_value(out).unwrap_or(Value::Null)
            }
        }
    }
}

fn call_vendor_vision(
    vendor: VendorKind,
    cfg: &RootConfig,
    requested_model: Option<&str>,
    timeout_seconds: u64,
    prompt: &str,
    images: &[ImageSource],
    max_input_bytes: usize,
) -> Result<(String, String, &'static str), String> {
    let mode = resolve_adapter_mode(&cfg.image_vision);
    let (vendor_name, vcfg) = resolve_vendor_config(cfg, vendor)?;
    check_api_key(vendor_name, &vcfg.api_key)?;
    match vendor {
        VendorKind::OpenAI => {
            let model = requested_model.unwrap_or(&vcfg.model).to_string();
            let client = Client::builder()
                .timeout(Duration::from_secs(
                    timeout_seconds.max(vcfg.timeout_seconds.unwrap_or(30)),
                ))
                .build()
                .map_err(|err| format!("build openai client failed: {err}"))?;
            let text = openai_vision(&client, &vcfg, &model, prompt, images, max_input_bytes)?;
            Ok((text, model, "native"))
        }
        VendorKind::Google => {
            let model = requested_model.unwrap_or(&vcfg.model).to_string();
            let client = Client::builder()
                .timeout(Duration::from_secs(
                    timeout_seconds.max(vcfg.timeout_seconds.unwrap_or(30)),
                ))
                .build()
                .map_err(|err| format!("build google client failed: {err}"))?;
            let text = google_vision(&client, &vcfg, &model, prompt, images, max_input_bytes)?;
            Ok((text, model, "native"))
        }
        VendorKind::Anthropic => {
            let model = requested_model.unwrap_or(&vcfg.model).to_string();
            let client = Client::builder()
                .timeout(Duration::from_secs(
                    timeout_seconds.max(vcfg.timeout_seconds.unwrap_or(30)),
                ))
                .build()
                .map_err(|err| format!("build anthropic client failed: {err}"))?;
            let text = anthropic_vision(&client, &vcfg, &model, prompt, images, max_input_bytes)?;
            Ok((text, model, "native"))
        }
        VendorKind::Grok | VendorKind::DeepSeek => {
            if mode == AdapterMode::Native {
                return Err(format!(
                    "{vendor_name} native vision adapter is not implemented; use image_vision.adapter_mode=compat"
                ));
            }
            let model = requested_model.unwrap_or(&vcfg.model).to_string();
            let client = Client::builder()
                .timeout(Duration::from_secs(
                    timeout_seconds.max(vcfg.timeout_seconds.unwrap_or(30)),
                ))
                .build()
                .map_err(|err| format!("build {vendor_name} client failed: {err}"))?;
            let text = openai_compat_vision(
                &client,
                &vcfg,
                &model,
                prompt,
                images,
                max_input_bytes,
                vendor_name,
                false,
            )?;
            Ok((text, model, "compat"))
        }
        VendorKind::Mimo => {
            if mode == AdapterMode::Native {
                return Err(
                    "mimo native vision adapter is not implemented; use image_vision.adapter_mode=compat"
                        .to_string(),
                );
            }
            let model = requested_model.unwrap_or(&vcfg.model).to_string();
            let client = Client::builder()
                .timeout(Duration::from_secs(
                    timeout_seconds.max(vcfg.timeout_seconds.unwrap_or(30)),
                ))
                .build()
                .map_err(|err| format!("build mimo client failed: {err}"))?;
            let text = mimo_vision(&client, &vcfg, &model, prompt, images, max_input_bytes)?;
            Ok((text, model, "compat"))
        }
        VendorKind::MiniMax => {
            if mode == AdapterMode::Native {
                return Err(
                    "minimax native vision adapter is not implemented; use image_vision.adapter_mode=compat"
                        .to_string(),
                );
            }
            let model = requested_model.unwrap_or(&vcfg.model).to_string();
            let client = Client::builder()
                .timeout(Duration::from_secs(
                    timeout_seconds.max(vcfg.timeout_seconds.unwrap_or(30)),
                ))
                .build()
                .map_err(|err| format!("build minimax client failed: {err}"))?;
            if let Ok(text) = minimax_mcp_vision(&vcfg, prompt, images, timeout_seconds) {
                return Ok((text, model, "mcp"));
            }
            let text = minimax_vision(&client, &vcfg, &model, prompt, images, max_input_bytes)?;
            Ok((text, model, "compat"))
        }
        VendorKind::Qwen => {
            let model = requested_model.unwrap_or(&vcfg.model).to_string();
            let client = Client::builder()
                .timeout(Duration::from_secs(
                    timeout_seconds.max(vcfg.timeout_seconds.unwrap_or(30)),
                ))
                .build()
                .map_err(|err| format!("build qwen client failed: {err}"))?;
            if mode == AdapterMode::Native {
                return Err(
                    "qwen native vision adapter is not implemented; use image_vision.adapter_mode=compat"
                        .to_string(),
                );
            }
            let text = openai_compat_vision(
                &client,
                &vcfg,
                &model,
                prompt,
                images,
                max_input_bytes,
                vendor_name,
                false,
            )?;
            Ok((text, model, "compat"))
        }
    }
}

fn resolve_adapter_mode(cfg: &ImageSkillConfig) -> AdapterMode {
    match cfg
        .adapter_mode
        .as_deref()
        .map(str::trim)
        .unwrap_or("auto")
        .to_ascii_lowercase()
        .as_str()
    {
        "native" => AdapterMode::Native,
        "compat" | "compatible" => AdapterMode::Compat,
        _ => AdapterMode::Auto,
    }
}

fn openai_vision(
    client: &Client,
    cfg: &VendorConfig,
    model: &str,
    prompt: &str,
    images: &[ImageSource],
    max_input_bytes: usize,
) -> Result<String, String> {
    openai_compat_vision(
        client,
        cfg,
        model,
        prompt,
        images,
        max_input_bytes,
        "openai",
        false,
    )
}

fn minimax_vision(
    client: &Client,
    cfg: &VendorConfig,
    model: &str,
    prompt: &str,
    images: &[ImageSource],
    max_input_bytes: usize,
) -> Result<String, String> {
    let mut content = String::from(prompt);
    for (idx, image) in images.iter().enumerate() {
        let encoded = image_base64_payload(image, max_input_bytes)?;
        content.push_str("\n\nimage ");
        content.push_str(&(idx + 1).to_string());
        content.push_str(":\n[图片base64:");
        content.push_str(&encoded);
        content.push(']');
    }
    let body = json!({
        "model": model,
        "messages": [{"role":"user","content":content}],
        "temperature": 0.2
    });
    let url = format!("{}/chat/completions", trim_trailing_slash(&cfg.base_url));
    let resp = client
        .post(url)
        .bearer_auth(&cfg.api_key)
        .json(&body)
        .send()
        .map_err(|err| format!("minimax request failed: {err}"))?;
    let status = resp.status().as_u16();
    let v: Value = resp
        .json()
        .map_err(|err| format!("parse minimax response failed: {err}"))?;
    if status >= 300 {
        return Err(format!(
            "minimax error status={status}: {}",
            provider_error_excerpt(&v, 400)
        ));
    }
    if let Some(s) = v
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|v| v.as_str())
    {
        return Ok(s.to_string());
    }
    Err(format!(
        "minimax response missing text: {}",
        provider_error_excerpt(&v, 400)
    ))
}

fn minimax_mcp_vision(
    cfg: &VendorConfig,
    prompt: &str,
    images: &[ImageSource],
    timeout_seconds: u64,
) -> Result<String, String> {
    if images.len() != 1 {
        return Err("minimax mcp image understanding supports one image per call".to_string());
    }
    let (image_arg, cleanup_path) = image_source_for_minimax_mcp(&images[0])?;
    let mut cmd = Command::new("npx");
    cmd.arg("-y")
        .arg("@jayjanii/pi-minimax-mcp")
        .arg("understand")
        .arg(&image_arg)
        .arg("--prompt")
        .arg(prompt)
        .env("MINIMAX_API_KEY", &cfg.api_key)
        .env("MINIMAX_API_HOST", minimax_mcp_api_host(&cfg.base_url))
        .env(
            "MINIMAX_MCP_STARTUP_TIMEOUT_MS",
            std::env::var("MINIMAX_MCP_STARTUP_TIMEOUT_MS").unwrap_or_else(|_| "60000".to_string()),
        )
        .env(
            "MINIMAX_MCP_TIMEOUT_MS",
            std::env::var("MINIMAX_MCP_TIMEOUT_MS")
                .unwrap_or_else(|_| (timeout_seconds.max(60) * 1000).to_string()),
        );
    if let Some(path) = path_with_local_uvx() {
        cmd.env("PATH", path);
    }
    if std::env::var_os("MINIMAX_MCP_UV_PATH").is_none() {
        if let Some(uvx) = default_uvx_path() {
            cmd.env("MINIMAX_MCP_UV_PATH", uvx);
        }
    }
    let output = cmd
        .output()
        .map_err(|err| format!("minimax mcp launch failed: {err}"));
    if let Some(path) = cleanup_path {
        let _ = std::fs::remove_file(path);
    }
    let output = output?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if output.status.success() && !stdout.is_empty() {
        return Ok(stdout);
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(format!(
        "minimax mcp failed status={}: {}{}",
        output
            .status
            .code()
            .map(|v| v.to_string())
            .unwrap_or_else(|| "signal".to_string()),
        redact_sensitive_inline(&truncate(&stderr, 600)),
        if stdout.is_empty() {
            String::new()
        } else {
            format!(
                " stdout={}",
                redact_sensitive_inline(&truncate(&stdout, 300))
            )
        }
    ))
}

fn image_source_for_minimax_mcp(image: &ImageSource) -> Result<(String, Option<PathBuf>), String> {
    match image {
        ImageSource::Url(s) => Ok((s.to_string(), None)),
        ImageSource::Path(p) => Ok((p.to_string_lossy().to_string(), None)),
        ImageSource::Base64(s) => {
            let path = std::env::temp_dir().join(format!(
                "rustclaw-image-vision-{}-{}.png",
                std::process::id(),
                monotonic_millis()
            ));
            let data = STANDARD
                .decode(strip_base64_data_url(s))
                .map_err(|err| format!("decode base64 image failed: {err}"))?;
            std::fs::write(&path, data).map_err(|err| format!("write temp image failed: {err}"))?;
            Ok((path.to_string_lossy().to_string(), Some(path)))
        }
    }
}

fn minimax_mcp_api_host(base_url: &str) -> String {
    let trimmed = trim_trailing_slash(base_url);
    trimmed
        .strip_suffix("/v1")
        .unwrap_or(trimmed.as_str())
        .to_string()
}

fn path_with_local_uvx() -> Option<String> {
    let current = std::env::var("PATH").unwrap_or_default();
    let home = std::env::var("HOME").ok()?;
    let local_bin = format!("{home}/.local/bin");
    if current.split(':').any(|part| part == local_bin) {
        Some(current)
    } else if Path::new(&local_bin).is_dir() {
        Some(format!("{local_bin}:{current}"))
    } else {
        Some(current)
    }
}

fn default_uvx_path() -> Option<String> {
    let home = std::env::var("HOME").ok()?;
    let uvx = Path::new(&home).join(".local/bin/uvx");
    uvx.exists().then(|| uvx.to_string_lossy().to_string())
}

fn monotonic_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|v| v.as_millis())
        .unwrap_or(0)
}

fn mimo_vision(
    client: &Client,
    cfg: &VendorConfig,
    model: &str,
    prompt: &str,
    images: &[ImageSource],
    max_input_bytes: usize,
) -> Result<String, String> {
    openai_compat_vision(
        client,
        cfg,
        model,
        prompt,
        images,
        max_input_bytes,
        "mimo",
        true,
    )
}

fn image_base64_payload(image: &ImageSource, max_input_bytes: usize) -> Result<String, String> {
    match image {
        ImageSource::Url(s) => Ok(s.to_string()),
        ImageSource::Path(p) => {
            let bytes = std::fs::read(p).map_err(|err| format!("read image failed: {err}"))?;
            if bytes.len() > max_input_bytes {
                return Err(format!("image too large: {} bytes", bytes.len()));
            }
            Ok(STANDARD.encode(bytes))
        }
        ImageSource::Base64(s) => Ok(strip_base64_data_url(s).to_string()),
    }
}

fn openai_compat_vision(
    client: &Client,
    cfg: &VendorConfig,
    model: &str,
    prompt: &str,
    images: &[ImageSource],
    max_input_bytes: usize,
    error_label: &str,
    include_api_key_header: bool,
) -> Result<String, String> {
    let mut content = vec![json!({"type":"text","text":prompt})];
    for image in images {
        let url = match image {
            ImageSource::Url(s) => s.to_string(),
            ImageSource::Path(p) => {
                let bytes = std::fs::read(p).map_err(|err| format!("read image failed: {err}"))?;
                if bytes.len() > max_input_bytes {
                    return Err(format!("image too large: {} bytes", bytes.len()));
                }
                let mime = guess_mime_from_path(p);
                format!("data:{mime};base64,{}", STANDARD.encode(bytes))
            }
            ImageSource::Base64(s) => normalize_base64_image(s),
        };
        content.push(json!({"type":"image_url","image_url":{"url":url}}));
    }
    let body = json!({
        "model": model,
        "messages": [{"role":"user","content":content}],
        "temperature": 0.2
    });
    let url = format!("{}/chat/completions", trim_trailing_slash(&cfg.base_url));
    let mut request = client.post(url).bearer_auth(&cfg.api_key);
    if include_api_key_header {
        request = request.header("api-key", &cfg.api_key);
    }
    let resp = request
        .json(&body)
        .send()
        .map_err(|err| format!("{error_label} request failed: {err}"))?;
    let status = resp.status().as_u16();
    let v: Value = resp
        .json()
        .map_err(|err| format!("parse openai response failed: {err}"))?;
    if status >= 300 {
        return Err(format!(
            "{error_label} error status={status}: {}",
            provider_error_excerpt(&v, 400)
        ));
    }
    if let Some(s) = v
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|v| v.as_str())
    {
        return Ok(s.to_string());
    }
    Err(format!(
        "{error_label} response missing text: {}",
        provider_error_excerpt(&v, 400)
    ))
}

fn google_vision(
    client: &Client,
    cfg: &VendorConfig,
    model: &str,
    prompt: &str,
    images: &[ImageSource],
    max_input_bytes: usize,
) -> Result<String, String> {
    let mut parts = vec![json!({"text":prompt})];
    for image in images {
        match image {
            ImageSource::Path(p) => {
                let bytes = std::fs::read(p).map_err(|err| format!("read image failed: {err}"))?;
                if bytes.len() > max_input_bytes {
                    return Err(format!("image too large: {} bytes", bytes.len()));
                }
                let mime = guess_mime_from_path(p);
                parts.push(json!({"inline_data":{"mime_type":mime,"data":STANDARD.encode(bytes)}}));
            }
            ImageSource::Base64(s) => {
                let (mime, data) = split_image_data(s);
                parts.push(json!({"inline_data":{"mime_type":mime,"data":data}}));
            }
            ImageSource::Url(u) => {
                parts.push(json!({"text": format!("Image URL: {u}")}));
            }
        }
    }
    let body = json!({"contents":[{"parts":parts}]});
    let url = format!(
        "{}/models/{}:generateContent?key={}",
        trim_trailing_slash(&cfg.base_url),
        model,
        cfg.api_key
    );
    let resp = client
        .post(url)
        .json(&body)
        .send()
        .map_err(|err| format!("google request failed: {err}"))?;
    let status = resp.status().as_u16();
    let v: Value = resp
        .json()
        .map_err(|err| format!("parse google response failed: {err}"))?;
    if status >= 300 {
        return Err(format!(
            "google error status={status}: {}",
            provider_error_excerpt(&v, 400)
        ));
    }
    let mut out = String::new();
    if let Some(parts) = v
        .get("candidates")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("content"))
        .and_then(|c| c.get("parts"))
        .and_then(|p| p.as_array())
    {
        for part in parts {
            if let Some(t) = part.get("text").and_then(|v| v.as_str()) {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(t);
            }
        }
    }
    if out.is_empty() {
        return Err(format!(
            "google response missing text: {}",
            provider_error_excerpt(&v, 400)
        ));
    }
    Ok(out)
}

fn anthropic_vision(
    client: &Client,
    cfg: &VendorConfig,
    model: &str,
    prompt: &str,
    images: &[ImageSource],
    max_input_bytes: usize,
) -> Result<String, String> {
    let mut content = vec![json!({"type":"text","text":prompt})];
    for image in images {
        match image {
            ImageSource::Path(p) => {
                let bytes = std::fs::read(p).map_err(|err| format!("read image failed: {err}"))?;
                if bytes.len() > max_input_bytes {
                    return Err(format!("image too large: {} bytes", bytes.len()));
                }
                let mime = guess_mime_from_path(p);
                content.push(json!({
                    "type":"image",
                    "source":{"type":"base64","media_type":mime,"data":STANDARD.encode(bytes)}
                }));
            }
            ImageSource::Base64(s) => {
                let (mime, data) = split_image_data(s);
                content.push(json!({
                    "type":"image",
                    "source":{"type":"base64","media_type":mime,"data":data}
                }));
            }
            ImageSource::Url(u) => {
                content.push(json!({"type":"text","text":format!("Image URL reference: {u}")}));
            }
        }
    }
    let body = json!({
        "model": model,
        "max_tokens": 1024,
        "messages": [{"role":"user","content":content}]
    });
    let url = format!("{}/messages", trim_trailing_slash(&cfg.base_url));
    let resp = client
        .post(url)
        .header("x-api-key", &cfg.api_key)
        .header("anthropic-version", "2023-06-01")
        .json(&body)
        .send()
        .map_err(|err| format!("anthropic request failed: {err}"))?;
    let status = resp.status().as_u16();
    let v: Value = resp
        .json()
        .map_err(|err| format!("parse anthropic response failed: {err}"))?;
    if status >= 300 {
        return Err(format!(
            "anthropic error status={status}: {}",
            provider_error_excerpt(&v, 400)
        ));
    }
    let mut out = String::new();
    if let Some(arr) = v.get("content").and_then(|c| c.as_array()) {
        for item in arr {
            if let Some(t) = item.get("text").and_then(|v| v.as_str()) {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(t);
            }
        }
    }
    if out.is_empty() {
        return Err(format!(
            "anthropic response missing text: {}",
            provider_error_excerpt(&v, 400)
        ));
    }
    Ok(out)
}

fn load_root_config() -> RootConfig {
    let root = workspace_root();
    let core_cfg = match std::fs::read_to_string(root.join("configs/config.toml"))
        .ok()
        .and_then(|s| toml::from_str::<toml::Value>(&s).ok())
    {
        Some(v) => v,
        None => toml::Value::Table(toml::map::Map::new()),
    };
    let image_cfg = match std::fs::read_to_string(root.join("configs/image.toml"))
        .ok()
        .and_then(|s| toml::from_str::<toml::Value>(&s).ok())
    {
        Some(v) => v,
        None => toml::Value::Table(toml::map::Map::new()),
    };

    let mut cfg = RootConfig::default();
    if let Some(v) = core_cfg.get("llm").cloned() {
        if let Ok(parsed) = v.try_into::<LlmConfig>() {
            cfg.llm = parsed;
        }
    }
    if let Some(v) = image_cfg.get("image_vision").cloned() {
        if let Ok(parsed) = v.try_into::<ImageSkillConfig>() {
            cfg.image_vision = parsed;
        }
    }
    apply_env_overrides(&mut cfg);
    cfg
}

fn env_non_empty(key: &str) -> Option<String> {
    claw_core::secrets::env_non_empty_resolved_or_none(key)
}

fn apply_vendor_api_key_env(target: &mut Option<VendorConfig>, key: &str) {
    if let (Some(value), Some(cfg)) = (env_non_empty(key), target.as_mut()) {
        cfg.api_key = value;
    }
}

fn apply_env_overrides(cfg: &mut RootConfig) {
    apply_vendor_api_key_env(&mut cfg.llm.openai, "OPENAI_API_KEY");
    apply_vendor_api_key_env(&mut cfg.llm.google, "GOOGLE_API_KEY");
    apply_vendor_api_key_env(&mut cfg.llm.anthropic, "ANTHROPIC_API_KEY");
    apply_vendor_api_key_env(&mut cfg.llm.grok, "GROK_API_KEY");
    apply_vendor_api_key_env(&mut cfg.llm.deepseek, "DEEPSEEK_API_KEY");
    apply_vendor_api_key_env(&mut cfg.llm.qwen, "QWEN_API_KEY");
    apply_vendor_api_key_env(&mut cfg.llm.minimax, "MINIMAX_API_KEY");
    apply_vendor_api_key_env(&mut cfg.llm.mimo, "MIMO_API_KEY");

    apply_selected_openai_compat_env(cfg);

    apply_vendor_api_key_env(
        &mut cfg.image_vision.providers.openai,
        "IMAGE_VISION_OPENAI_API_KEY",
    );
    apply_vendor_api_key_env(
        &mut cfg.image_vision.providers.google,
        "IMAGE_VISION_GOOGLE_API_KEY",
    );
    apply_vendor_api_key_env(
        &mut cfg.image_vision.providers.anthropic,
        "IMAGE_VISION_ANTHROPIC_API_KEY",
    );
    apply_vendor_api_key_env(
        &mut cfg.image_vision.providers.grok,
        "IMAGE_VISION_GROK_API_KEY",
    );
    apply_vendor_api_key_env(
        &mut cfg.image_vision.providers.deepseek,
        "IMAGE_VISION_DEEPSEEK_API_KEY",
    );
    apply_vendor_api_key_env(
        &mut cfg.image_vision.providers.qwen,
        "IMAGE_VISION_QWEN_API_KEY",
    );
    apply_vendor_api_key_env(
        &mut cfg.image_vision.providers.minimax,
        "IMAGE_VISION_MINIMAX_API_KEY",
    );
    apply_vendor_api_key_env(
        &mut cfg.image_vision.providers.mimo,
        "IMAGE_VISION_MIMO_API_KEY",
    );
}

fn apply_selected_openai_compat_env(cfg: &mut RootConfig) {
    let Some(vendor) = cfg.llm.selected_vendor.as_deref().and_then(parse_vendor) else {
        return;
    };
    let base_url = env_non_empty("OPENAI_BASE_URL");
    let model = env_non_empty("OPENAI_MODEL");
    let api_key = env_non_empty("OPENAI_API_KEY");
    let Some(target) = llm_vendor_config_mut(&mut cfg.llm, vendor) else {
        return;
    };
    if let Some(value) = base_url {
        target.base_url = value;
    }
    if let Some(value) = model {
        target.model = value;
    }
    if let Some(value) = api_key {
        target.api_key = value;
    }
}

fn llm_vendor_config_mut(cfg: &mut LlmConfig, vendor: VendorKind) -> Option<&mut VendorConfig> {
    match vendor {
        VendorKind::OpenAI => cfg.openai.as_mut(),
        VendorKind::Google => cfg.google.as_mut(),
        VendorKind::Anthropic => cfg.anthropic.as_mut(),
        VendorKind::Grok => cfg.grok.as_mut(),
        VendorKind::DeepSeek => cfg.deepseek.as_mut(),
        VendorKind::Qwen => cfg.qwen.as_mut(),
        VendorKind::MiniMax => cfg.minimax.as_mut(),
        VendorKind::Mimo => cfg.mimo.as_mut(),
    }
}

fn workspace_root() -> PathBuf {
    std::env::var("WORKSPACE_ROOT")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

fn vendor_order(
    requested: Option<&str>,
    section_default: Option<&str>,
    selected_vendor: Option<&str>,
) -> Vec<VendorKind> {
    if let Some(req) = requested.and_then(parse_vendor) {
        return vec![req];
    }
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for name in [section_default, selected_vendor].into_iter().flatten() {
        if let Some(v) = parse_vendor(name) {
            if seen.insert(v) {
                out.push(v);
            }
        }
    }
    for v in [
        VendorKind::OpenAI,
        VendorKind::Google,
        VendorKind::Anthropic,
        VendorKind::Grok,
        VendorKind::DeepSeek,
        VendorKind::Qwen,
        VendorKind::MiniMax,
        VendorKind::Mimo,
    ] {
        if seen.insert(v) {
            out.push(v);
        }
    }
    out
}

fn parse_vendor(name: &str) -> Option<VendorKind> {
    match name.trim().to_ascii_lowercase().as_str() {
        "openai" => Some(VendorKind::OpenAI),
        "google" | "gemini" => Some(VendorKind::Google),
        "anthropic" | "claude" => Some(VendorKind::Anthropic),
        "grok" | "xai" => Some(VendorKind::Grok),
        "deepseek" => Some(VendorKind::DeepSeek),
        "qwen" => Some(VendorKind::Qwen),
        "minimax" => Some(VendorKind::MiniMax),
        "mimo" | "xiaomi" => Some(VendorKind::Mimo),
        _ => None,
    }
}

fn vendor_name(v: VendorKind) -> &'static str {
    match v {
        VendorKind::OpenAI => "openai",
        VendorKind::Google => "google",
        VendorKind::Anthropic => "anthropic",
        VendorKind::Grok => "grok",
        VendorKind::DeepSeek => "deepseek",
        VendorKind::Qwen => "qwen",
        VendorKind::MiniMax => "minimax",
        VendorKind::Mimo => "mimo",
    }
}

fn provider_config_with_shared_key(
    provider: Option<&VendorConfig>,
    shared: Option<&VendorConfig>,
) -> Option<VendorConfig> {
    match (provider, shared) {
        (Some(provider), Some(shared)) => {
            let mut merged = provider.clone();
            if check_api_key("", &merged.api_key).is_err()
                && check_api_key("", &shared.api_key).is_ok()
            {
                merged.api_key = shared.api_key.clone();
            }
            Some(merged)
        }
        (Some(provider), None) => Some(provider.clone()),
        (None, Some(shared)) => Some(shared.clone()),
        (None, None) => None,
    }
}

fn resolve_vendor_config(
    cfg: &RootConfig,
    vendor: VendorKind,
) -> Result<(&'static str, VendorConfig), String> {
    let section = &cfg.image_vision.providers;
    match vendor {
        VendorKind::OpenAI => {
            provider_config_with_shared_key(section.openai.as_ref(), cfg.llm.openai.as_ref())
                .map(|v| ("openai", v))
                .ok_or_else(|| "openai config missing".to_string())
        }
        VendorKind::Google => {
            provider_config_with_shared_key(section.google.as_ref(), cfg.llm.google.as_ref())
                .map(|v| ("google", v))
                .ok_or_else(|| "google config missing".to_string())
        }
        VendorKind::Anthropic => {
            provider_config_with_shared_key(section.anthropic.as_ref(), cfg.llm.anthropic.as_ref())
                .map(|v| ("anthropic", v))
                .ok_or_else(|| "anthropic config missing".to_string())
        }
        VendorKind::Grok => {
            provider_config_with_shared_key(section.grok.as_ref(), cfg.llm.grok.as_ref())
                .map(|v| ("grok", v))
                .ok_or_else(|| "grok config missing".to_string())
        }
        VendorKind::DeepSeek => {
            provider_config_with_shared_key(section.deepseek.as_ref(), cfg.llm.deepseek.as_ref())
                .map(|v| ("deepseek", v))
                .ok_or_else(|| "deepseek config missing".to_string())
        }
        VendorKind::Qwen => {
            provider_config_with_shared_key(section.qwen.as_ref(), cfg.llm.qwen.as_ref())
                .map(|v| ("qwen", v))
                .ok_or_else(|| "qwen config missing".to_string())
        }
        VendorKind::MiniMax => {
            provider_config_with_shared_key(section.minimax.as_ref(), cfg.llm.minimax.as_ref())
                .map(|v| ("minimax", v))
                .ok_or_else(|| "minimax config missing".to_string())
        }
        VendorKind::Mimo => {
            provider_config_with_shared_key(section.mimo.as_ref(), cfg.llm.mimo.as_ref())
                .map(|v| ("mimo", v))
                .ok_or_else(|| "mimo config missing".to_string())
        }
    }
}

fn check_api_key(vendor: &str, key: &str) -> Result<(), String> {
    let t = key.trim();
    if t.is_empty() || t.starts_with("REPLACE_ME_") {
        return Err(format!("{vendor} api key is not configured"));
    }
    Ok(())
}

fn guess_mime_from_path(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        _ => "image/png",
    }
}

fn normalize_base64_image(raw: &str) -> String {
    let t = raw.trim();
    if t.starts_with("data:image/") {
        t.to_string()
    } else {
        format!("data:image/png;base64,{t}")
    }
}

fn split_image_data(raw: &str) -> (String, String) {
    let t = raw.trim();
    if let Some(body) = t.strip_prefix("data:") {
        let mut parts = body.splitn(2, ',');
        let meta = parts.next().unwrap_or("image/png;base64");
        let data = parts.next().unwrap_or("").to_string();
        let mime = meta
            .split(';')
            .next()
            .unwrap_or("image/png")
            .to_string()
            .trim()
            .to_string();
        return (mime, data);
    }
    ("image/png".to_string(), t.to_string())
}

fn strip_base64_data_url(raw: &str) -> &str {
    let t = raw.trim();
    if let Some((_, data)) = t.split_once(',') {
        data
    } else {
        t
    }
}

fn trim_trailing_slash(v: &str) -> String {
    v.trim_end_matches('/').to_string()
}

fn provider_error_excerpt(value: &Value, max: usize) -> String {
    truncate(&redact_sensitive_inline(&value.to_string()), max)
}

fn redact_sensitive_inline(text: &str) -> String {
    static SECRET_ASSIGNMENT_RE: OnceLock<Regex> = OnceLock::new();
    static BEARER_RE: OnceLock<Regex> = OnceLock::new();
    static OPENAI_STYLE_KEY_RE: OnceLock<Regex> = OnceLock::new();

    let out = SECRET_ASSIGNMENT_RE
        .get_or_init(|| {
            Regex::new(
                r#"(?i)("?(?:api[_-]?key|api[_-]?secret|access[_-]?token|refresh[_-]?token|authorization|client[_-]?secret|secret|token)"?\s*(?::|=)\s*"?)[A-Za-z0-9_./+=*\-]{8,}"?"#,
            )
            .expect("secret assignment redaction regex compiles")
        })
        .replace_all(text, "${1}[REDACTED]")
        .to_string();
    let out = BEARER_RE
        .get_or_init(|| {
            Regex::new(r#"(?i)(bearer\s+)[A-Za-z0-9_./+=*\-]{8,}"#)
                .expect("bearer redaction regex compiles")
        })
        .replace_all(&out, "${1}[REDACTED]")
        .to_string();
    OPENAI_STYLE_KEY_RE
        .get_or_init(|| {
            Regex::new(r#"sk-[A-Za-z0-9_*\-]{6,}"#)
                .expect("openai-style key redaction regex compiles")
        })
        .replace_all(&out, "[REDACTED_API_KEY]")
        .to_string()
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    s.chars().take(max).collect::<String>() + "..."
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
