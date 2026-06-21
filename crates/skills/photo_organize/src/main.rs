use std::collections::{BTreeMap, HashMap};
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use exif::{In, Reader, Tag};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use toml::Value as TomlValue;
mod i18n;
mod organize_flow;

use i18n::*;
use organize_flow::*;

const IMAGE_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "heic", "heif", "tif", "tiff", "arw", "cr2", "cr3", "nef", "raf", "dng",
];
const PREVIEW_LIMIT_DEFAULT: usize = 12;
const CAMERA_GROUP_LIMIT: usize = 8;
const NON_EXIF_LIST_LIMIT: usize = 50;
const TEXT_PATH_DELIMS: &[char] = &[
    ' ', '\n', '\t', ',', ';', '，', '；', '。', '(', ')', '[', ']', '{', '}',
];
const PHOTO_CHILD_DIR_HINTS: &[&str] = &["DCIM", "Photos", "Pictures", "照片", "相机", "Camera"];

#[derive(Debug, Deserialize)]
struct Req {
    request_id: String,
    args: Value,
    #[serde(default)]
    context: Option<Value>,
    #[serde(default)]
    #[serde(rename = "user_id")]
    _user_id: i64,
    #[serde(default)]
    #[serde(rename = "chat_id")]
    _chat_id: i64,
}

#[derive(Debug, Serialize)]
struct Resp {
    request_id: String,
    status: String,
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    buttons: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    extra: Option<Value>,
    error_text: Option<String>,
}

#[derive(Debug)]
struct SkillOutput {
    text: String,
    buttons: Option<Value>,
    extra: Option<Value>,
}

#[derive(Debug, Deserialize, Default)]
struct RootConfig {
    #[serde(default)]
    photo_organize: PhotoOrganizeConfig,
}

#[derive(Debug, Deserialize, Default)]
struct PhotoOrganizeConfig {
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    i18n_path: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GroupField {
    Brand,
    Model,
    Lens,
    FocalLength,
    Year,
    YearMonth,
    Date,
}

impl GroupField {
    fn as_arg_str(self) -> &'static str {
        match self {
            Self::Brand => "brand",
            Self::Model => "model",
            Self::Lens => "lens",
            Self::FocalLength => "focal_length",
            Self::Year => "year",
            Self::YearMonth => "year_month",
            Self::Date => "date",
        }
    }

    fn i18n_key(self) -> &'static str {
        match self {
            Self::Brand => "photo_organize.msg.group_field.brand",
            Self::Model => "photo_organize.msg.group_field.model",
            Self::Lens => "photo_organize.msg.group_field.lens",
            Self::FocalLength => "photo_organize.msg.group_field.focal_length",
            Self::Year => "photo_organize.msg.group_field.year",
            Self::YearMonth => "photo_organize.msg.group_field.year_month",
            Self::Date => "photo_organize.msg.group_field.date",
        }
    }

    fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().replace('-', "_").as_str() {
            "brand" | "make" | "camera_make" => Some(Self::Brand),
            "model" | "camera_model" => Some(Self::Model),
            "lens" | "lens_model" => Some(Self::Lens),
            "focal" | "focal_length" | "focal_len" => Some(Self::FocalLength),
            "year" | "capture_year" | "yyyy" => Some(Self::Year),
            "month" | "year_month" | "capture_month" | "yyyy_mm" => Some(Self::YearMonth),
            "date" | "day" | "capture_date" | "year_month_day" | "yyyy_mm_dd" => Some(Self::Date),
            _ => None,
        }
    }

    fn defaults() -> Vec<Self> {
        vec![
            Self::Brand,
            Self::Model,
            Self::Lens,
            Self::FocalLength,
            Self::YearMonth,
        ]
    }
}

#[derive(Debug, Clone)]
struct OrganizeOptions {
    group_by: Vec<GroupField>,
    capture_year: Option<String>,
    capture_month: Option<String>,
    capture_date: Option<String>,
    selected_brands: Vec<String>,
    selected_models: Vec<String>,
    selected_lenses: Vec<String>,
}

impl OrganizeOptions {
    fn has_filters(&self) -> bool {
        self.capture_year.is_some()
            || self.capture_month.is_some()
            || self.capture_date.is_some()
            || !self.selected_brands.is_empty()
            || !self.selected_models.is_empty()
            || !self.selected_lenses.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OrganizeMode {
    Plan,
    Copy,
    Move,
}

impl OrganizeMode {
    fn parse(raw: Option<&str>, cat: &TextCatalog) -> Result<Self, String> {
        match raw.unwrap_or("plan").trim().to_ascii_lowercase().as_str() {
            "" | "plan" | "preview" | "dry_run" => Ok(Self::Plan),
            "copy" => Ok(Self::Copy),
            "move" => Ok(Self::Move),
            other => Err(tr_with(
                cat,
                "photo_organize.err.unsupported_mode",
                &[("mode", other.to_string())],
            )),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Plan => "plan",
            Self::Copy => "copy",
            Self::Move => "move",
        }
    }
}

#[derive(Debug)]
struct PhotoPlan {
    source: PathBuf,
    source_rel: String,
    file_name: String,
    camera_make: Option<String>,
    camera_model: Option<String>,
    lens_model: Option<String>,
    focal_length: Option<String>,
    capture_year: Option<String>,
    year_month: Option<String>,
    capture_date: Option<String>,
    camera_label: String,
    lens_label: String,
    classification_rel: String,
    destination_dir: PathBuf,
    destination_rel: String,
    has_camera_metadata: bool,
}

#[derive(Debug, Default)]
struct InferredIntent {
    source_dir: Option<String>,
    output_dir: Option<String>,
    notes: Vec<String>,
}

#[derive(Debug, Default)]
struct ApplySummary {
    processed: usize,
    copied: usize,
    moved: usize,
    skipped: usize,
    failures: Vec<String>,
}

#[derive(Debug)]
struct BuildPhotoPlansResult {
    plans: Vec<PhotoPlan>,
    scanned_photo_count: usize,
    skipped_no_exif: usize,
    non_exif_files: Vec<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HostPlatform {
    MacOS,
    Linux,
    Other,
}

fn main() -> anyhow::Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let workspace_root = std::env::var("WORKSPACE_ROOT")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let cfg = load_root_config(&workspace_root);

    for line in stdin.lock().lines() {
        let line = line?;
        let parsed: Result<Req, _> = serde_json::from_str(&line);
        let resp = match parsed {
            Ok(req) => {
                let lang = resolve_lang(&req, &cfg);
                let cat = load_catalog(&workspace_root, &cfg.photo_organize, &lang);
                match execute(&req.args, &cat) {
                    Ok(out) => Resp {
                        request_id: req.request_id,
                        status: "ok".to_string(),
                        text: out.text,
                        buttons: out.buttons,
                        extra: out.extra,
                        error_text: None,
                    },
                    Err(err) => Resp {
                        request_id: req.request_id,
                        status: "error".to_string(),
                        text: String::new(),
                        buttons: None,
                        extra: None,
                        error_text: Some(err),
                    },
                }
            }
            Err(err) => Resp {
                request_id: "unknown".to_string(),
                status: "error".to_string(),
                text: String::new(),
                buttons: None,
                extra: None,
                error_text: Some(tr_with(
                    &load_catalog(
                        &workspace_root,
                        &cfg.photo_organize,
                        cfg.photo_organize
                            .language
                            .as_deref()
                            .map(normalize_lang_tag)
                            .unwrap_or_else(|| "zh-CN".to_string())
                            .as_str(),
                    ),
                    "photo_organize.err.invalid_input",
                    &[("error", err.to_string())],
                )),
            },
        };
        writeln!(stdout, "{}", serde_json::to_string(&resp)?)?;
        stdout.flush()?;
    }

    Ok(())
}

fn load_root_config(workspace_root: &Path) -> RootConfig {
    let path = workspace_root.join("configs/photo_organize.toml");
    let raw = match std::fs::read_to_string(path) {
        Ok(v) => v,
        Err(_) => return RootConfig::default(),
    };
    toml::from_str::<RootConfig>(&raw).unwrap_or_default()
}

fn read_camera_metadata(path: &Path) -> Option<Value> {
    let file = File::open(path).ok()?;
    let mut reader = BufReader::new(file);
    let exif = Reader::new().read_from_container(&mut reader).ok()?;
    let make = exif_string(&exif, Tag::Make);
    let model = exif_string(&exif, Tag::Model);
    let lens_model = exif_string(&exif, Tag::LensModel)
        .or_else(|| exif_string(&exif, Tag::LensMake))
        .or_else(|| exif_string(&exif, Tag::LensSerialNumber));
    let focal_length = exif_string(&exif, Tag::FocalLengthIn35mmFilm)
        .map(|value| normalize_focal_value(&value, true))
        .or_else(|| {
            exif_string(&exif, Tag::FocalLength).map(|value| normalize_focal_value(&value, false))
        });
    let capture_time = exif_string(&exif, Tag::DateTimeOriginal)
        .or_else(|| exif_string(&exif, Tag::DateTimeDigitized))
        .or_else(|| exif_string(&exif, Tag::DateTime));
    let (capture_year, year_month, capture_date) = capture_time
        .as_deref()
        .map(parse_capture_date_parts)
        .unwrap_or_default();
    Some(json!({
        "make": make,
        "model": model,
        "lens_model": lens_model,
        "focal_length": focal_length,
        "captured_at": capture_time,
        "capture_year": capture_year,
        "year_month": year_month,
        "capture_date": capture_date,
    }))
}

fn exif_string(exif: &exif::Exif, tag: Tag) -> Option<String> {
    exif.get_field(tag, In::PRIMARY)
        .map(|field| field.display_value().with_unit(exif).to_string())
        .map(|raw| raw.trim_matches('"').trim().to_string())
        .filter(|value| !value.is_empty())
}

fn parse_capture_date_parts(raw: &str) -> (Option<String>, Option<String>, Option<String>) {
    let Some(date_part) = raw.split_whitespace().next() else {
        return (None, None, None);
    };
    let parts = date_part.split([':', '-', '/', '.']).collect::<Vec<_>>();
    let year = parts.first().copied().unwrap_or_default();
    let month = parts.get(1).copied().unwrap_or_default();
    let day = parts.get(2).copied().unwrap_or_default();
    if year.len() != 4
        || !year.chars().all(|ch| ch.is_ascii_digit())
        || month.len() != 2
        || !month.chars().all(|ch| ch.is_ascii_digit())
    {
        return (None, None, None);
    }
    let capture_year = Some(year.to_string());
    let year_month = Some(format!("{year}-{month}"));
    let capture_date = if day.len() == 2 && day.chars().all(|ch| ch.is_ascii_digit()) {
        Some(format!("{year}-{month}-{day}"))
    } else {
        None
    };
    (capture_year, year_month, capture_date)
}

fn normalize_focal_value(raw: &str, is_35mm_equivalent: bool) -> String {
    let compact = raw
        .replace(" ", "")
        .replace("mm", "")
        .replace("MM", "")
        .replace(".0", "");
    let compact = compact.trim().trim_matches('"');
    if compact.is_empty() {
        return if is_35mm_equivalent {
            "unknown_focal_35mm".to_string()
        } else {
            "unknown_focal".to_string()
        };
    }
    if is_35mm_equivalent {
        format!("{compact}mm_eq")
    } else {
        format!("{compact}mm")
    }
}

fn is_photo_path(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| {
            let ext = ext.to_ascii_lowercase();
            IMAGE_EXTENSIONS.iter().any(|allowed| *allowed == ext)
        })
        .unwrap_or(false)
}

fn sanitize_component(raw: &str) -> String {
    let sanitized = raw
        .trim()
        .chars()
        .map(|ch| match ch {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect::<String>()
        .trim_matches('.')
        .trim()
        .to_string();
    if sanitized.is_empty() {
        "unknown".to_string()
    } else {
        sanitized
    }
}

fn normalize_focal_length_label(raw: &str) -> String {
    sanitize_component(raw).replace(" ", "").replace("__", "_")
}

fn normalize_args(args: &Value, cat: &TextCatalog) -> Result<Value, String> {
    let mut obj = match args {
        Value::Object(map) => map.clone(),
        Value::String(text) => {
            let mut map = Map::new();
            map.insert("text".to_string(), Value::String(text.clone()));
            map
        }
        _ => return Err(tr(cat, "photo_organize.err.args_object")),
    };

    let mut inferred = infer_from_natural_language(&obj);

    if !obj.contains_key("source_dir") {
        if let Some(source_dir) = inferred.source_dir.take() {
            obj.insert("source_dir".to_string(), Value::String(source_dir));
        }
    }
    if !obj.contains_key("output_dir") {
        if let Some(output_dir) = inferred.output_dir.take() {
            obj.insert("output_dir".to_string(), Value::String(output_dir));
        }
    }
    if !obj.contains_key("action") {
        obj.insert("action".to_string(), Value::String("organize".to_string()));
    }
    if !inferred.notes.is_empty() {
        obj.insert(
            "_natural_language_notes".to_string(),
            Value::Array(inferred.notes.into_iter().map(Value::String).collect()),
        );
    }
    Ok(Value::Object(obj))
}

fn infer_from_natural_language(obj: &Map<String, Value>) -> InferredIntent {
    let mut inferred = InferredIntent::default();
    let Some(text) = pick_string(obj, &["text", "prompt", "input", "instruction", "query"]) else {
        return inferred;
    };
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return inferred;
    }

    // Keep skill-local fallback parsing limited to concrete path binding. Semantic
    // choices such as mode/grouping/date/brand filters belong in LLM-produced
    // structured args, not hard-coded natural-language keyword tables.
    let explicit_paths = extract_path_like_tokens(trimmed);
    if let Some(first_path) = explicit_paths.first() {
        inferred.source_dir = Some(first_path.clone());
        inferred
            .notes
            .push(format!("from_text_source_dir={first_path}"));
        if explicit_paths.len() >= 2 {
            let second_path = explicit_paths[1].clone();
            inferred.output_dir = Some(second_path.clone());
            inferred
                .notes
                .push(format!("from_text_output_dir={second_path}"));
        }
    } else if let Some(candidate) = resolve_candidate_root_from_text(trimmed) {
        inferred.source_dir = Some(candidate.clone());
        inferred
            .notes
            .push(format!("from_candidate_source_dir={candidate}"));
    }

    inferred
}

fn extract_path_like_tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for token in text.split(TEXT_PATH_DELIMS) {
        let token = token
            .trim_matches('"')
            .trim_matches('\'')
            .trim_matches('`')
            .trim();
        if token.starts_with('/') || token.starts_with("./") || token.starts_with("~/") {
            out.push(token.to_string());
        }
    }
    out.dedup();
    out
}

fn resolve_candidate_root_from_text(text: &str) -> Option<String> {
    let lowered = text.to_lowercase();
    let mut matches = Vec::new();
    for candidate in discover_external_photo_candidates() {
        let base_name = Path::new(&candidate)
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_lowercase();
        if !base_name.is_empty() && lowered.contains(&base_name) {
            matches.push(candidate);
        }
    }
    matches.sort();
    matches.dedup();
    if matches.len() == 1 {
        matches.into_iter().next()
    } else {
        None
    }
}

fn pick_string<'a>(obj: &'a Map<String, Value>, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| obj.get(*key).and_then(Value::as_str))
        .filter(|value| !value.trim().is_empty())
}

fn relative_or_absolute(path: &Path, base: &Path) -> String {
    path.strip_prefix(base)
        .map(|value| value.display().to_string())
        .unwrap_or_else(|_| path.display().to_string())
}

fn discover_external_photo_candidates() -> Vec<String> {
    let mut roots = Vec::new();
    for path in discover_external_roots() {
        push_unique_string(&mut roots, path.display().to_string());
        for child in discover_photo_children(&path) {
            push_unique_string(&mut roots, child.display().to_string());
        }
    }
    roots
}

fn discover_photo_children(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        if PHOTO_CHILD_DIR_HINTS
            .iter()
            .any(|hint| name.eq_ignore_ascii_case(hint))
        {
            out.push(path);
        }
    }
    out.sort();
    out
}

fn discover_external_roots() -> Vec<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        discover_macos_volume_roots()
    }
    #[cfg(target_os = "linux")]
    {
        let mut roots = Vec::new();
        for path in discover_linux_mountinfo_roots() {
            push_unique_path(&mut roots, path);
        }
        for path in discover_linux_common_mount_roots() {
            push_unique_path(&mut roots, path);
        }
        roots
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        Vec::new()
    }
}

#[cfg(target_os = "macos")]
fn discover_macos_volume_roots() -> Vec<PathBuf> {
    discover_roots_in("/Volumes")
        .into_iter()
        .filter(|path| {
            fs::canonicalize(path)
                .map(|canonical| canonical != Path::new("/"))
                .unwrap_or(true)
        })
        .collect()
}

#[cfg(target_os = "linux")]
fn discover_linux_mountinfo_roots() -> Vec<PathBuf> {
    let Ok(raw) = fs::read_to_string("/proc/self/mountinfo") else {
        return Vec::new();
    };
    linux_external_roots_from_mountinfo(&raw)
}

#[cfg(target_os = "linux")]
fn linux_external_roots_from_mountinfo(raw: &str) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for line in raw.lines() {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        let Some(raw_mount_point) = fields.get(4) else {
            continue;
        };
        let mount_point = PathBuf::from(decode_mountinfo_path(raw_mount_point));
        if is_linux_external_mount_path(&mount_point) {
            push_unique_path(&mut roots, mount_point);
        }
    }
    roots
}

#[cfg(target_os = "linux")]
fn discover_linux_common_mount_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for path in discover_media_style_roots("/media") {
        push_unique_path(&mut roots, path);
    }
    for path in discover_media_style_roots("/run/media") {
        push_unique_path(&mut roots, path);
    }
    for path in discover_roots_in("/mnt") {
        push_unique_path(&mut roots, path);
    }
    roots
}

#[cfg(target_os = "linux")]
fn discover_media_style_roots(base: &str) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for path in discover_roots_in(base) {
        let child_dirs = discover_roots_in_path(&path);
        if !child_dirs.is_empty() && !has_photo_child_hint(&child_dirs) {
            for child in child_dirs {
                push_unique_path(&mut roots, child);
            }
        }
        if !is_likely_user_media_container(&path) {
            push_unique_path(&mut roots, path);
        }
    }
    roots
}

#[cfg(target_os = "linux")]
fn is_likely_user_media_container(path: &Path) -> bool {
    let parent = path.parent().and_then(|value| value.to_str());
    if !matches!(parent, Some("/media") | Some("/run/media")) {
        return false;
    }
    let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
        return false;
    };
    [std::env::var("USER").ok(), std::env::var("LOGNAME").ok()]
        .into_iter()
        .flatten()
        .any(|user| user == name)
        || Path::new("/home").join(name).is_dir()
}

#[cfg(target_os = "linux")]
fn has_photo_child_hint(paths: &[PathBuf]) -> bool {
    paths.iter().any(|path| {
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        PHOTO_CHILD_DIR_HINTS
            .iter()
            .any(|hint| name.eq_ignore_ascii_case(hint))
    })
}

#[cfg(target_os = "linux")]
fn is_linux_external_mount_path(path: &Path) -> bool {
    let text = path.to_string_lossy();
    if text == "/media" || text == "/run/media" || text == "/mnt" {
        return false;
    }
    text.starts_with("/media/") || text.starts_with("/mnt/") || text.starts_with("/run/media/")
}

#[cfg(target_os = "linux")]
fn decode_mountinfo_path(raw: &str) -> String {
    raw.replace("\\040", " ")
        .replace("\\011", "\t")
        .replace("\\012", "\n")
        .replace("\\134", "\\")
}

fn current_platform() -> HostPlatform {
    #[cfg(target_os = "macos")]
    {
        HostPlatform::MacOS
    }
    #[cfg(target_os = "linux")]
    {
        HostPlatform::Linux
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        HostPlatform::Other
    }
}

fn discover_roots_in(root: &str) -> Vec<PathBuf> {
    discover_roots_in_path(Path::new(root))
}

fn discover_roots_in_path(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.push(path);
        }
    }
    out.sort();
    out
}

fn push_unique_path(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if !paths.iter().any(|existing| existing == &path) {
        paths.push(path);
    }
}

fn push_unique_string(items: &mut Vec<String>, item: String) {
    if !items.iter().any(|existing| existing == &item) {
        items.push(item);
    }
}

#[cfg(all(test, target_os = "linux"))]
#[path = "main_tests.rs"]
mod tests;
