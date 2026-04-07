use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use chrono::{Datelike, Local};
use exif::{In, Reader, Tag};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use toml::Value as TomlValue;

const IMAGE_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "heic", "heif", "tif", "tiff", "arw", "cr2", "cr3", "nef", "raf", "dng",
];
const PREVIEW_LIMIT_DEFAULT: usize = 12;
const CAMERA_GROUP_LIMIT: usize = 8;
const NON_EXIF_LIST_LIMIT: usize = 50;
const TEXT_PATH_DELIMS: &[char] = &[
    ' ', '\n', '\t', ',', ';', '，', '；', '。', '(', ')', '[', ']', '{', '}',
];

#[derive(Debug, Deserialize)]
struct Req {
    request_id: String,
    args: Value,
    #[serde(default)]
    context: Option<Value>,
    #[allow(dead_code)]
    #[serde(default)]
    user_id: i64,
    #[allow(dead_code)]
    #[serde(default)]
    chat_id: i64,
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

#[derive(Debug, Clone)]
struct TextCatalog {
    current: HashMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GroupField {
    Brand,
    Model,
    Lens,
    FocalLength,
    YearMonth,
}

impl GroupField {
    fn as_arg_str(self) -> &'static str {
        match self {
            Self::Brand => "brand",
            Self::Model => "model",
            Self::Lens => "lens",
            Self::FocalLength => "focal_length",
            Self::YearMonth => "year_month",
        }
    }

    fn i18n_key(self) -> &'static str {
        match self {
            Self::Brand => "photo_organize.msg.group_field.brand",
            Self::Model => "photo_organize.msg.group_field.model",
            Self::Lens => "photo_organize.msg.group_field.lens",
            Self::FocalLength => "photo_organize.msg.group_field.focal_length",
            Self::YearMonth => "photo_organize.msg.group_field.year_month",
        }
    }

    fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().replace('-', "_").as_str() {
            "brand" | "make" | "camera_make" => Some(Self::Brand),
            "model" | "camera_model" => Some(Self::Model),
            "lens" | "lens_model" => Some(Self::Lens),
            "focal" | "focal_length" | "focal_len" => Some(Self::FocalLength),
            "date" | "month" | "year_month" | "capture_month" => Some(Self::YearMonth),
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
    capture_month: Option<String>,
    selected_brands: Vec<String>,
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
    year_month: Option<String>,
    camera_label: String,
    lens_label: String,
    classification_rel: String,
    destination_dir: PathBuf,
    destination_rel: String,
    has_camera_metadata: bool,
}

#[derive(Debug, Default)]
struct InferredIntent {
    action: Option<String>,
    source_dir: Option<String>,
    output_dir: Option<String>,
    mode: Option<OrganizeMode>,
    group_by: Option<Vec<GroupField>>,
    capture_month: Option<String>,
    selected_brands: Option<Vec<String>>,
    include_subdirs: Option<bool>,
    preview_limit: Option<usize>,
    raw_text: Option<String>,
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

fn resolve_lang(req: &Req, cfg: &RootConfig) -> String {
    if let Some(obj) = req.args.as_object() {
        for key in ["locale", "language", "lang"] {
            if let Some(s) = obj.get(key).and_then(|v| v.as_str()) {
                let trimmed = s.trim();
                if !trimmed.is_empty() {
                    return normalize_lang_tag(trimmed);
                }
            }
        }
    }
    if let Some(ctx) = &req.context {
        if let Some(obj) = ctx.as_object() {
            for key in ["locale", "language", "lang"] {
                if let Some(s) = obj.get(key).and_then(|v| v.as_str()) {
                    let trimmed = s.trim();
                    if !trimmed.is_empty() {
                        return normalize_lang_tag(trimmed);
                    }
                }
            }
        }
    }
    cfg.photo_organize
        .language
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(normalize_lang_tag)
        .unwrap_or_else(|| "zh-CN".to_string())
}

fn normalize_lang_tag(raw: &str) -> String {
    let lower = raw.trim().to_ascii_lowercase().replace('_', "-");
    match lower.as_str() {
        "zh" | "zh-cn" => "zh-CN".to_string(),
        "en" | "en-us" => "en-US".to_string(),
        _ => raw.trim().to_string(),
    }
}

fn load_external_i18n(path: &Path) -> Option<HashMap<String, String>> {
    let raw = std::fs::read_to_string(path).ok()?;
    let value: TomlValue = toml::from_str(&raw).ok()?;
    let dict = value.get("dict")?.as_table()?;
    let mut out = HashMap::new();
    for (k, v) in dict {
        collect_i18n_entries(k, v, &mut out);
    }
    Some(out)
}

fn collect_i18n_entries(prefix: &str, value: &TomlValue, out: &mut HashMap<String, String>) {
    if let Some(text) = value.as_str() {
        out.insert(prefix.to_string(), text.to_string());
        return;
    }
    if let Some(table) = value.as_table() {
        for (k, v) in table {
            let next = format!("{prefix}.{k}");
            collect_i18n_entries(&next, v, out);
        }
    }
}

fn default_i18n_dict() -> HashMap<String, String> {
    let mut m = HashMap::new();
    m.insert(
        "photo_organize.err.invalid_input".to_string(),
        "Invalid input: {error}".to_string(),
    );
    m.insert(
        "photo_organize.err.args_object".to_string(),
        "args must be object or string".to_string(),
    );
    m.insert(
        "photo_organize.err.normalized_args_object".to_string(),
        "normalized args must be object".to_string(),
    );
    m.insert(
        "photo_organize.err.unsupported_action".to_string(),
        "Unsupported action `{action}`; allowed: prepare|organize".to_string(),
    );
    m.insert(
        "photo_organize.err.unsupported_mode".to_string(),
        "Unsupported mode `{mode}`; allowed: plan|copy|move".to_string(),
    );
    m.insert(
        "photo_organize.err.no_photos_found".to_string(),
        "No photo files were found under `{path}`.".to_string(),
    );
    m.insert(
        "photo_organize.err.no_photos_for_month".to_string(),
        "No photos taken in {capture_month} were found under `{path}`.".to_string(),
    );
    m.insert(
        "photo_organize.err.no_exif_operable_photos".to_string(),
        "Photo files were found under `{path}`, but none had readable EXIF metadata, so no operation was performed.".to_string(),
    );
    m.insert(
        "photo_organize.err.partial_apply".to_string(),
        "Photo organization partially completed: succeeded {success} files, failed {failed} files. First error: {first_error}".to_string(),
    );
    m.insert(
        "photo_organize.msg.completed".to_string(),
        "Photo organization completed: scanned {scanned_count} files, skipped {skipped_no_exif} files without readable EXIF, processed {processed} files, {action_word} {applied} files grouped by {group_by_desc}, skipped {skipped} files. Output directory: {output_dir}. Filter: {filter_desc}.".to_string(),
    );
    m.insert(
        "photo_organize.msg.action_word.copy".to_string(),
        "copied".to_string(),
    );
    m.insert(
        "photo_organize.msg.action_word.move".to_string(),
        "moved".to_string(),
    );
    m.insert(
        "photo_organize.msg.group_field.brand".to_string(),
        "brand".to_string(),
    );
    m.insert(
        "photo_organize.msg.group_field.model".to_string(),
        "model".to_string(),
    );
    m.insert(
        "photo_organize.msg.group_field.lens".to_string(),
        "lens".to_string(),
    );
    m.insert(
        "photo_organize.msg.group_field.focal_length".to_string(),
        "focal length".to_string(),
    );
    m.insert(
        "photo_organize.msg.group_field.year_month".to_string(),
        "year-month".to_string(),
    );
    m.insert(
        "photo_organize.msg.filter.none".to_string(),
        "none".to_string(),
    );
    m.insert(
        "photo_organize.msg.filter.capture_month".to_string(),
        "only photos shot in {capture_month}".to_string(),
    );
    m.insert(
        "photo_organize.msg.no_external_candidates".to_string(),
        "No obvious external drive or USB mount points were detected.".to_string(),
    );
    m.insert(
        "photo_organize.msg.macos_hint".to_string(),
        "Common macOS path example: `/Volumes/<disk-name>`.".to_string(),
    );
    m.insert(
        "photo_organize.msg.linux_hint".to_string(),
        "Common Linux path example: `/media/<user>/<disk-name>` or `/mnt/<disk-name>`.".to_string(),
    );
    m.insert(
        "photo_organize.msg.other_os_hint".to_string(),
        "This skill has explicit mount-path discovery for macOS and Linux. On the current OS, please provide the photo directory manually as an absolute path.".to_string(),
    );
    m.insert(
        "photo_organize.msg.directory_prompt".to_string(),
        "Please explicitly specify the photo directory to organize.\n\nDetected external drive / USB candidate paths:\n{candidates}\n\nCall `photo_organize` again with an explicit `source_dir`. By default it organizes by brand/model/lens/focal length/year-month. Start with `mode=\"plan\"` to preview before using `copy` or `move`.\nExample: {{\"action\":\"organize\",\"source_dir\":\"/Volumes/SDCARD/DCIM\",\"mode\":\"plan\"}}\nNatural language also works: \"Organize the photos in /Volumes/SDCARD/DCIM, preview first, do not move originals\"".to_string(),
    );
    m.insert(
        "photo_organize.err.resolve_current_dir".to_string(),
        "resolve current_dir failed: {error}".to_string(),
    );
    m.insert(
        "photo_organize.err.source_dir_inaccessible".to_string(),
        "source_dir does not exist or is not accessible: {path} ({error})".to_string(),
    );
    m.insert(
        "photo_organize.err.source_dir_metadata".to_string(),
        "failed to read source_dir metadata: {path} ({error})".to_string(),
    );
    m.insert(
        "photo_organize.err.source_dir_not_directory".to_string(),
        "source_dir is not a directory: {path}".to_string(),
    );
    m.insert(
        "photo_organize.err.read_dir_failed".to_string(),
        "failed to read directory: {path} ({error})".to_string(),
    );
    m.insert(
        "photo_organize.err.read_dir_entry_failed".to_string(),
        "failed to read directory entry: {path} ({error})".to_string(),
    );
    m.insert(
        "photo_organize.err.read_metadata_failed".to_string(),
        "failed to read file metadata: {path} ({error})".to_string(),
    );
    m.insert(
        "photo_organize.err.invalid_filename".to_string(),
        "failed to parse file name: {path}".to_string(),
    );
    m.insert(
        "photo_organize.msg.preview_empty".to_string(),
        "No preview items.".to_string(),
    );
    m.insert(
        "photo_organize.msg.preview_item".to_string(),
        "- {source} -> {destination}".to_string(),
    );
    m.insert(
        "photo_organize.msg.plan_summary".to_string(),
        "Preview generated for photo organization: scanned {photo_count} photos, {with_metadata} with readable EXIF metadata, skipped {without_metadata} without readable EXIF. Output directory: {output_dir}.\n\nThis run groups by {group_by_desc}. Filter: {filter_desc}.\n\nFirst {preview_count} preview items:\n{preview_lines}".to_string(),
    );
    m.insert(
        "photo_organize.err.create_dest_dir_failed".to_string(),
        "failed to create destination directory: {path} ({error})".to_string(),
    );
    m.insert(
        "photo_organize.err.copy_failed".to_string(),
        "failed to copy file: {source} -> {dest} ({error})".to_string(),
    );
    m.insert(
        "photo_organize.err.remove_original_failed".to_string(),
        "failed to remove original file: {source} ({error})".to_string(),
    );
    m
}

fn load_catalog(workspace_root: &Path, cfg: &PhotoOrganizeConfig, lang: &str) -> TextCatalog {
    let mut current = default_i18n_dict();
    let path = cfg
        .i18n_path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|p| {
            if Path::new(p).is_absolute() {
                PathBuf::from(p)
            } else {
                workspace_root.join(p)
            }
        })
        .unwrap_or_else(|| workspace_root.join(format!("configs/i18n/photo_organize.{lang}.toml")));
    if let Some(overrides) = load_external_i18n(&path) {
        current.extend(overrides);
    } else if lang != "en-US" {
        let fallback = workspace_root.join("configs/i18n/photo_organize.en-US.toml");
        if let Some(overrides) = load_external_i18n(&fallback) {
            for (k, v) in overrides {
                current.entry(k).or_insert(v);
            }
        }
    }
    TextCatalog { current }
}

fn tr(cat: &TextCatalog, key: &str) -> String {
    cat.current
        .get(key)
        .cloned()
        .unwrap_or_else(|| key.to_string())
}

fn tr_with(cat: &TextCatalog, key: &str, vars: &[(&str, String)]) -> String {
    let mut out = tr(cat, key);
    for (name, value) in vars {
        out = out.replace(&format!("{{{name}}}"), value);
    }
    out
}

fn group_by_display(cat: &TextCatalog, fields: &[GroupField]) -> String {
    fields
        .iter()
        .map(|field| tr(cat, field.i18n_key()))
        .collect::<Vec<_>>()
        .join(" / ")
}

fn filter_display(cat: &TextCatalog, options: &OrganizeOptions) -> String {
    let mut parts = Vec::new();
    if let Some(capture_month) = &options.capture_month {
        parts.push(tr_with(
            cat,
            "photo_organize.msg.filter.capture_month",
            &[("capture_month", capture_month.clone())],
        ));
    }
    if !options.selected_brands.is_empty() {
        parts.push(tr_with(
            cat,
            "photo_organize.msg.filter.selected_brands",
            &[("brands", options.selected_brands.join(" / "))],
        ));
    }
    if parts.is_empty() {
        tr(cat, "photo_organize.msg.filter.none")
    } else {
        parts.join("；")
    }
}

fn build_no_matches_error(
    source_dir: &Path,
    options: &OrganizeOptions,
    scanned_photo_count: usize,
    skipped_no_exif: usize,
    non_exif_files: &[String],
    cat: &TextCatalog,
) -> String {
    if scanned_photo_count > 0 && skipped_no_exif == scanned_photo_count {
        let base = tr_with(
            cat,
            "photo_organize.err.no_exif_operable_photos",
            &[("path", source_dir.display().to_string())],
        );
        return if let Some(non_exif_text) = non_exif_list_text(cat, non_exif_files) {
            format!("{base}\n\n{non_exif_text}")
        } else {
            base
        };
    }
    if !options.selected_brands.is_empty() {
        return tr_with(
            cat,
            "photo_organize.err.no_photos_for_brands",
            &[
                ("path", source_dir.display().to_string()),
                ("brands", options.selected_brands.join(" / ")),
            ],
        );
    }
    if let Some(capture_month) = &options.capture_month {
        return tr_with(
            cat,
            "photo_organize.err.no_photos_for_month",
            &[
                ("path", source_dir.display().to_string()),
                ("capture_month", capture_month.clone()),
            ],
        );
    }
    tr_with(
        cat,
        "photo_organize.err.no_photos_found",
        &[("path", source_dir.display().to_string())],
    )
}

fn non_exif_list_text(cat: &TextCatalog, non_exif_files: &[String]) -> Option<String> {
    if non_exif_files.is_empty() {
        return None;
    }
    let preview = non_exif_files
        .iter()
        .take(NON_EXIF_LIST_LIMIT)
        .map(|path| {
            tr_with(
                cat,
                "photo_organize.msg.non_exif_item",
                &[("path", path.clone())],
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    Some(tr_with(
        cat,
        "photo_organize.msg.non_exif_list",
        &[
            ("count", non_exif_files.len().to_string()),
            ("items", preview),
        ],
    ))
}

fn execute(args: &Value, cat: &TextCatalog) -> Result<SkillOutput, String> {
    let normalized = normalize_args(args, cat)?;
    let obj = normalized
        .as_object()
        .ok_or_else(|| tr(cat, "photo_organize.err.normalized_args_object"))?;
    let action = obj
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or("organize")
        .trim()
        .to_ascii_lowercase();
    match action.as_str() {
        "prepare" | "select_source" => Ok(build_directory_prompt(cat)),
        "organize" | "run" => handle_organize(obj, cat),
        other => Err(tr_with(
            cat,
            "photo_organize.err.unsupported_action",
            &[("action", other.to_string())],
        )),
    }
}

fn handle_organize(obj: &Map<String, Value>, cat: &TextCatalog) -> Result<SkillOutput, String> {
    let source_dir_raw = match pick_string(
        obj,
        &[
            "source_dir",
            "source",
            "dir",
            "directory",
            "path",
            "photo_dir",
        ],
    ) {
        Some(raw) if !raw.trim().is_empty() => raw.trim(),
        _ => return Ok(build_directory_prompt(cat)),
    };
    let mode = OrganizeMode::parse(
        obj.get("mode")
            .or_else(|| obj.get("organize_mode"))
            .and_then(Value::as_str),
        cat,
    )?;
    let include_subdirs = obj
        .get("include_subdirs")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let preview_limit = obj
        .get("preview_limit")
        .and_then(Value::as_u64)
        .unwrap_or(PREVIEW_LIMIT_DEFAULT as u64)
        .clamp(1, 50) as usize;
    let options = resolve_organize_options(obj);

    let source_dir = resolve_existing_dir(source_dir_raw, cat)?;
    let output_dir = resolve_output_dir(obj, &source_dir)?;
    let photo_files = collect_photo_files(&source_dir, include_subdirs, &output_dir, cat)?;
    if photo_files.is_empty() {
        return Err(tr_with(
            cat,
            "photo_organize.err.no_photos_found",
            &[("path", source_dir.display().to_string())],
        ));
    }

    let mut build_result = build_photo_plans(&source_dir, &output_dir, photo_files, &options, cat)?;
    build_result
        .plans
        .sort_by(|left, right| left.source_rel.cmp(&right.source_rel));
    if build_result.plans.is_empty() {
        return Err(build_no_matches_error(
            &source_dir,
            &options,
            build_result.scanned_photo_count,
            build_result.skipped_no_exif,
            &build_result.non_exif_files,
            cat,
        ));
    }

    if mode == OrganizeMode::Plan {
        return Ok(build_plan_output(
            &source_dir,
            &output_dir,
            include_subdirs,
            &build_result.plans,
            build_result.scanned_photo_count,
            build_result.skipped_no_exif,
            &build_result.non_exif_files,
            preview_limit,
            &options,
            cat,
        ));
    }

    let summary = apply_plan(&build_result.plans, mode, cat)?;
    if !summary.failures.is_empty() {
        let first_failure = summary.failures.first().cloned().unwrap_or_default();
        return Err(tr_with(
            cat,
            "photo_organize.err.partial_apply",
            &[
                ("success", (summary.copied + summary.moved).to_string()),
                ("failed", summary.failures.len().to_string()),
                ("first_error", first_failure),
            ],
        ));
    }

    let preview = build_result
        .plans
        .iter()
        .take(preview_limit)
        .map(|plan| {
            json!({
                "source": plan.source_rel,
                "destination": plan.destination_rel,
                "camera": plan.camera_label,
                "lens": plan.lens_label,
                "classification_path": plan.classification_rel,
            })
        })
        .collect::<Vec<_>>();

    let action_word = if mode == OrganizeMode::Move {
        tr(cat, "photo_organize.msg.action_word.move")
    } else {
        tr(cat, "photo_organize.msg.action_word.copy")
    };
    let text = tr_with(
        cat,
        "photo_organize.msg.completed",
        &[
            (
                "scanned_count",
                build_result.scanned_photo_count.to_string(),
            ),
            ("processed", summary.processed.to_string()),
            ("skipped_no_exif", build_result.skipped_no_exif.to_string()),
            ("action_word", action_word),
            ("applied", (summary.copied + summary.moved).to_string()),
            ("skipped", summary.skipped.to_string()),
            ("output_dir", output_dir.display().to_string()),
            ("group_by_desc", group_by_display(cat, &options.group_by)),
            ("filter_desc", filter_display(cat, &options)),
        ],
    );
    let text = if let Some(non_exif_text) = non_exif_list_text(cat, &build_result.non_exif_files) {
        format!("{text}\n\n{non_exif_text}")
    } else {
        text
    };
    Ok(SkillOutput {
        text,
        buttons: None,
        extra: Some(json!({
            "action": "organize",
            "mode": mode.as_str(),
            "source_dir": source_dir.display().to_string(),
            "output_dir": output_dir.display().to_string(),
            "scanned_photo_count": build_result.scanned_photo_count,
            "processed": summary.processed,
            "copied": summary.copied,
            "moved": summary.moved,
            "skipped": summary.skipped,
            "skipped_no_exif": build_result.skipped_no_exif,
            "group_by": options
                .group_by
                .iter()
                .map(|field| field.as_arg_str())
                .collect::<Vec<_>>(),
            "capture_month": options.capture_month,
            "selected_brands": options.selected_brands,
            "non_exif_files": build_result.non_exif_files,
            "preview": preview,
        })),
    })
}

fn build_directory_prompt(cat: &TextCatalog) -> SkillOutput {
    let candidates = discover_external_photo_candidates();
    let lines = if candidates.is_empty() {
        platform_hint_lines(cat)
    } else {
        candidates
            .iter()
            .enumerate()
            .map(|(idx, path)| format!("{}. {}", idx + 1, path))
            .collect::<Vec<_>>()
    };
    let text = tr_with(
        cat,
        "photo_organize.msg.directory_prompt",
        &[("candidates", lines.join("\n"))],
    );
    let buttons = if candidates.is_empty() {
        None
    } else {
        Some(json!(candidates
            .iter()
            .take(6)
            .map(|path| {
                json!({
                    "text": path,
                    "value": json!({
                        "action": "organize",
                        "source_dir": path,
                        "mode": "plan"
                    }).to_string()
                })
            })
            .collect::<Vec<_>>()))
    };
    SkillOutput {
        text,
        buttons,
        extra: Some(json!({
            "action": "prepare",
            "needs_directory": true,
            "external_candidates": candidates,
            "recommended_mode": "plan",
        })),
    }
}

fn platform_hint_lines(cat: &TextCatalog) -> Vec<String> {
    match current_platform() {
        HostPlatform::MacOS => vec![
            tr(cat, "photo_organize.msg.no_external_candidates"),
            tr(cat, "photo_organize.msg.macos_hint"),
        ],
        HostPlatform::Linux => vec![
            tr(cat, "photo_organize.msg.no_external_candidates"),
            tr(cat, "photo_organize.msg.linux_hint"),
        ],
        HostPlatform::Other => vec![
            tr(cat, "photo_organize.msg.no_external_candidates"),
            tr(cat, "photo_organize.msg.other_os_hint"),
        ],
    }
}

fn resolve_existing_dir(raw: &str, cat: &TextCatalog) -> Result<PathBuf, String> {
    let path = PathBuf::from(raw);
    let path = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .map_err(|err| {
                tr_with(
                    cat,
                    "photo_organize.err.resolve_current_dir",
                    &[("error", err.to_string())],
                )
            })?
            .join(path)
    };
    let canonical = fs::canonicalize(&path).map_err(|err| {
        tr_with(
            cat,
            "photo_organize.err.source_dir_inaccessible",
            &[
                ("path", path.display().to_string()),
                ("error", err.to_string()),
            ],
        )
    })?;
    let meta = fs::metadata(&canonical).map_err(|err| {
        tr_with(
            cat,
            "photo_organize.err.source_dir_metadata",
            &[
                ("path", canonical.display().to_string()),
                ("error", err.to_string()),
            ],
        )
    })?;
    if !meta.is_dir() {
        return Err(tr_with(
            cat,
            "photo_organize.err.source_dir_not_directory",
            &[("path", canonical.display().to_string())],
        ));
    }
    Ok(canonical)
}

fn resolve_output_dir(obj: &Map<String, Value>, source_dir: &Path) -> Result<PathBuf, String> {
    let output = match obj.get("output_dir").and_then(Value::as_str) {
        Some(raw) if !raw.trim().is_empty() => {
            let candidate = PathBuf::from(raw.trim());
            if candidate.is_absolute() {
                candidate
            } else {
                source_dir.join(candidate)
            }
        }
        _ => source_dir.join("_organized_by_camera"),
    };
    Ok(output)
}

fn resolve_organize_options(obj: &Map<String, Value>) -> OrganizeOptions {
    let group_by = parse_group_by_value(obj.get("group_by"))
        .filter(|fields| !fields.is_empty())
        .unwrap_or_else(GroupField::defaults);
    let capture_month = obj
        .get("capture_month")
        .or_else(|| obj.get("month"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(normalize_capture_month);
    let selected_brands = parse_string_list(
        obj.get("selected_brands")
            .or_else(|| obj.get("brands"))
            .or_else(|| obj.get("camera_brands")),
    )
    .into_iter()
    .filter_map(|brand| canonical_brand_name(&brand))
    .collect::<Vec<_>>();
    OrganizeOptions {
        group_by,
        capture_month,
        selected_brands,
    }
}

fn parse_group_by_value(value: Option<&Value>) -> Option<Vec<GroupField>> {
    let value = value?;
    let mut out = Vec::new();
    match value {
        Value::String(text) => {
            for token in text.split([',', '|', '/', '>', ' ']) {
                if let Some(field) = GroupField::parse(token) {
                    push_unique_group_field(&mut out, field);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                if let Some(raw) = item.as_str() {
                    if let Some(field) = GroupField::parse(raw) {
                        push_unique_group_field(&mut out, field);
                    }
                }
            }
        }
        _ => {}
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

fn push_unique_group_field(out: &mut Vec<GroupField>, field: GroupField) {
    if !out.contains(&field) {
        out.push(field);
    }
}

fn normalize_capture_month(raw: &str) -> String {
    raw.trim().replace('/', "-").replace('.', "-")
}

fn parse_string_list(value: Option<&Value>) -> Vec<String> {
    let Some(value) = value else {
        return Vec::new();
    };
    let mut out = Vec::new();
    match value {
        Value::String(text) => {
            for token in text.split([',', '|', '/', '、', '，', ' ']) {
                let trimmed = token.trim();
                if !trimmed.is_empty() {
                    out.push(trimmed.to_string());
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                if let Some(text) = item.as_str() {
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        out.push(trimmed.to_string());
                    }
                }
            }
        }
        _ => {}
    }
    out
}

fn canonical_brand_name(raw: &str) -> Option<String> {
    let lowered = raw.trim().to_ascii_lowercase();
    match lowered.as_str() {
        "canon" | "佳能" => Some("Canon".to_string()),
        "sony" | "索尼" => Some("Sony".to_string()),
        "nikon" | "尼康" => Some("Nikon".to_string()),
        "fujifilm" | "fuji" | "富士" => Some("Fujifilm".to_string()),
        "panasonic" | "lumix" | "松下" => Some("Panasonic".to_string()),
        "leica" | "徕卡" => Some("Leica".to_string()),
        _ if raw.trim().is_empty() => None,
        _ => Some(raw.trim().to_string()),
    }
}

fn brand_matches(camera_make: &str, selected_brands: &[String]) -> bool {
    if selected_brands.is_empty() {
        return true;
    }
    let make_lower = camera_make.to_ascii_lowercase();
    selected_brands.iter().any(|brand| {
        let brand_lower = brand.to_ascii_lowercase();
        make_lower.contains(&brand_lower)
            || match brand_lower.as_str() {
                "canon" => make_lower.contains("canon") || make_lower.contains("佳能"),
                "sony" => make_lower.contains("sony") || make_lower.contains("索尼"),
                "nikon" => make_lower.contains("nikon") || make_lower.contains("尼康"),
                "fujifilm" => {
                    make_lower.contains("fujifilm")
                        || make_lower.contains("fuji")
                        || make_lower.contains("富士")
                }
                "panasonic" => {
                    make_lower.contains("panasonic")
                        || make_lower.contains("lumix")
                        || make_lower.contains("松下")
                }
                "leica" => make_lower.contains("leica") || make_lower.contains("徕卡"),
                _ => false,
            }
    })
}

fn collect_photo_files(
    source_dir: &Path,
    include_subdirs: bool,
    output_dir: &Path,
    cat: &TextCatalog,
) -> Result<Vec<PathBuf>, String> {
    let mut out = Vec::new();
    collect_photo_files_inner(source_dir, include_subdirs, output_dir, &mut out, cat)?;
    out.sort();
    Ok(out)
}

fn collect_photo_files_inner(
    dir: &Path,
    include_subdirs: bool,
    output_dir: &Path,
    out: &mut Vec<PathBuf>,
    cat: &TextCatalog,
) -> Result<(), String> {
    let entries = fs::read_dir(dir).map_err(|err| {
        tr_with(
            cat,
            "photo_organize.err.read_dir_failed",
            &[
                ("path", dir.display().to_string()),
                ("error", err.to_string()),
            ],
        )
    })?;
    for entry in entries {
        let entry = entry.map_err(|err| {
            tr_with(
                cat,
                "photo_organize.err.read_dir_entry_failed",
                &[
                    ("path", dir.display().to_string()),
                    ("error", err.to_string()),
                ],
            )
        })?;
        let path = entry.path();
        let meta = entry.metadata().map_err(|err| {
            tr_with(
                cat,
                "photo_organize.err.read_metadata_failed",
                &[
                    ("path", path.display().to_string()),
                    ("error", err.to_string()),
                ],
            )
        })?;
        if meta.is_dir() {
            if path == output_dir {
                continue;
            }
            if include_subdirs {
                collect_photo_files_inner(&path, include_subdirs, output_dir, out, cat)?;
            }
            continue;
        }
        if meta.is_file() && is_photo_path(&path) {
            out.push(path);
        }
    }
    Ok(())
}

fn build_photo_plans(
    source_dir: &Path,
    output_dir: &Path,
    photo_files: Vec<PathBuf>,
    options: &OrganizeOptions,
    cat: &TextCatalog,
) -> Result<BuildPhotoPlansResult, String> {
    let scanned_photo_count = photo_files.len();
    let mut skipped_no_exif = 0usize;
    let mut non_exif_files = Vec::new();
    let mut plans = Vec::with_capacity(photo_files.len());
    for path in photo_files {
        let metadata = read_camera_metadata(&path);
        if metadata.is_none() {
            skipped_no_exif += 1;
            non_exif_files.push(relative_or_absolute(&path, source_dir));
            continue;
        }
        let make = metadata
            .as_ref()
            .and_then(|meta| meta.get("make"))
            .and_then(Value::as_str)
            .map(str::to_string);
        let model = metadata
            .as_ref()
            .and_then(|meta| meta.get("model"))
            .and_then(Value::as_str)
            .map(str::to_string);
        let lens_model = metadata
            .as_ref()
            .and_then(|meta| meta.get("lens_model"))
            .and_then(Value::as_str)
            .map(str::to_string);
        let focal_length = metadata
            .as_ref()
            .and_then(|meta| meta.get("focal_length"))
            .and_then(Value::as_str)
            .map(str::to_string);
        let year_month = metadata
            .as_ref()
            .and_then(|meta| meta.get("year_month"))
            .and_then(Value::as_str)
            .map(str::to_string);
        if let Some(make_text) = make.as_deref() {
            if !brand_matches(make_text, &options.selected_brands) {
                continue;
            }
        } else if !options.selected_brands.is_empty() {
            continue;
        }
        if let Some(capture_month) = &options.capture_month {
            if year_month.as_deref() != Some(capture_month.as_str()) {
                continue;
            }
        }

        let make_dir = make
            .as_deref()
            .map(sanitize_component)
            .unwrap_or_else(|| "unknown_camera".to_string());
        let model_dir = if make.is_none() && model.is_none() {
            "unknown_model".to_string()
        } else {
            model
                .as_deref()
                .map(sanitize_component)
                .unwrap_or_else(|| "unknown_model".to_string())
        };
        let date_dir = year_month
            .as_deref()
            .map(sanitize_component)
            .unwrap_or_else(|| "undated".to_string());
        let lens_dir = lens_model
            .as_deref()
            .map(sanitize_component)
            .unwrap_or_else(|| "unknown_lens".to_string());
        let focal_dir = focal_length
            .as_deref()
            .map(normalize_focal_length_label)
            .unwrap_or_else(|| "unknown_focal".to_string());

        let mut destination_dir = output_dir.to_path_buf();
        let mut classification_parts = Vec::new();
        for field in &options.group_by {
            let value = match field {
                GroupField::Brand => &make_dir,
                GroupField::Model => &model_dir,
                GroupField::Lens => &lens_dir,
                GroupField::FocalLength => &focal_dir,
                GroupField::YearMonth => &date_dir,
            };
            destination_dir = destination_dir.join(value);
            classification_parts.push(value.clone());
        }
        let destination_rel = relative_or_absolute(&destination_dir, output_dir);
        let classification_rel = classification_parts.join("/");
        let camera_label = match (make.as_deref(), model.as_deref()) {
            (Some(mk), Some(md)) if mk.eq_ignore_ascii_case(md) => mk.to_string(),
            (Some(mk), Some(md)) => format!("{mk} / {md}"),
            (Some(mk), None) => mk.to_string(),
            (None, Some(md)) => md.to_string(),
            (None, None) => "unknown_camera".to_string(),
        };
        let lens_label = match (lens_model.as_deref(), focal_length.as_deref()) {
            (Some(lens), Some(focal)) => format!("{lens} / {focal}"),
            (Some(lens), None) => lens.to_string(),
            (None, Some(focal)) => focal.to_string(),
            (None, None) => "unknown_lens".to_string(),
        };
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                tr_with(
                    cat,
                    "photo_organize.err.invalid_filename",
                    &[("path", path.display().to_string())],
                )
            })?
            .to_string();
        plans.push(PhotoPlan {
            source_rel: relative_or_absolute(&path, source_dir),
            source: path,
            file_name,
            camera_make: make,
            camera_model: model,
            lens_model,
            focal_length,
            year_month,
            camera_label,
            lens_label,
            classification_rel,
            destination_dir,
            destination_rel,
            has_camera_metadata: metadata.is_some(),
        });
    }
    Ok(BuildPhotoPlansResult {
        plans,
        scanned_photo_count,
        skipped_no_exif,
        non_exif_files,
    })
}

fn build_plan_output(
    source_dir: &Path,
    output_dir: &Path,
    include_subdirs: bool,
    plans: &[PhotoPlan],
    scanned_photo_count: usize,
    skipped_no_exif: usize,
    non_exif_files: &[String],
    preview_limit: usize,
    options: &OrganizeOptions,
    cat: &TextCatalog,
) -> SkillOutput {
    let with_metadata = plans.iter().filter(|plan| plan.has_camera_metadata).count();
    let mut camera_groups = BTreeMap::<String, usize>::new();
    let mut lens_groups = BTreeMap::<String, usize>::new();
    for plan in plans {
        *camera_groups.entry(plan.camera_label.clone()).or_insert(0) += 1;
        *lens_groups.entry(plan.lens_label.clone()).or_insert(0) += 1;
    }
    let mut top_groups = camera_groups.into_iter().collect::<Vec<_>>();
    top_groups.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    let top_groups = top_groups
        .into_iter()
        .take(CAMERA_GROUP_LIMIT)
        .map(|(camera, count)| json!({ "camera": camera, "count": count }))
        .collect::<Vec<_>>();
    let mut top_lens_groups = lens_groups.into_iter().collect::<Vec<_>>();
    top_lens_groups.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    let top_lens_groups = top_lens_groups
        .into_iter()
        .take(CAMERA_GROUP_LIMIT)
        .map(|(lens, count)| json!({ "lens": lens, "count": count }))
        .collect::<Vec<_>>();
    let preview = plans
        .iter()
        .take(preview_limit)
        .map(|plan| {
            json!({
                "source": plan.source_rel,
                "destination": format!("{}/{}", plan.destination_rel, plan.file_name),
                "camera": plan.camera_label,
                "make": plan.camera_make,
                "model": plan.camera_model,
                "lens": plan.lens_model,
                "focal_length": plan.focal_length,
                "year_month": plan.year_month,
                "classification_path": plan.classification_rel,
            })
        })
        .collect::<Vec<_>>();
    let preview_lines = if preview.is_empty() {
        tr(cat, "photo_organize.msg.preview_empty")
    } else {
        preview
            .iter()
            .map(|item| {
                tr_with(
                    cat,
                    "photo_organize.msg.preview_item",
                    &[
                        (
                            "source",
                            item.get("source")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_string(),
                        ),
                        (
                            "destination",
                            item.get("destination")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_string(),
                        ),
                    ],
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let text = tr_with(
        cat,
        "photo_organize.msg.plan_summary",
        &[
            ("photo_count", scanned_photo_count.to_string()),
            ("with_metadata", with_metadata.to_string()),
            ("without_metadata", skipped_no_exif.to_string()),
            ("output_dir", output_dir.display().to_string()),
            ("preview_count", preview.len().to_string()),
            ("preview_lines", preview_lines),
            ("group_by_desc", group_by_display(cat, &options.group_by)),
            ("filter_desc", filter_display(cat, options)),
        ],
    );
    let text = if let Some(non_exif_text) = non_exif_list_text(cat, non_exif_files) {
        format!("{text}\n\n{non_exif_text}")
    } else {
        text
    };
    SkillOutput {
        text,
        buttons: None,
        extra: Some(json!({
            "action": "organize",
            "mode": "plan",
            "source_dir": source_dir.display().to_string(),
            "output_dir": output_dir.display().to_string(),
            "include_subdirs": include_subdirs,
            "photo_count": scanned_photo_count,
            "with_camera_metadata": with_metadata,
            "without_camera_metadata": skipped_no_exif,
            "skipped_no_exif": skipped_no_exif,
            "group_by": options
                .group_by
                .iter()
                .map(|field| field.as_arg_str())
                .collect::<Vec<_>>(),
            "capture_month": options.capture_month,
            "selected_brands": options.selected_brands,
            "non_exif_files": non_exif_files,
            "top_camera_groups": top_groups,
            "top_lens_groups": top_lens_groups,
            "preview": preview,
        })),
    }
}

fn apply_plan(
    plans: &[PhotoPlan],
    mode: OrganizeMode,
    cat: &TextCatalog,
) -> Result<ApplySummary, String> {
    let mut summary = ApplySummary {
        processed: plans.len(),
        ..ApplySummary::default()
    };
    for plan in plans {
        if !plan.destination_dir.exists() {
            fs::create_dir_all(&plan.destination_dir).map_err(|err| {
                tr_with(
                    cat,
                    "photo_organize.err.create_dest_dir_failed",
                    &[
                        ("path", plan.destination_dir.display().to_string()),
                        ("error", err.to_string()),
                    ],
                )
            })?;
        }
        let dest_path = allocate_destination_path(&plan.destination_dir, &plan.file_name);
        if dest_path == plan.source {
            summary.skipped += 1;
            continue;
        }
        let result = match mode {
            OrganizeMode::Plan => Ok(()),
            OrganizeMode::Copy => copy_file(&plan.source, &dest_path, cat),
            OrganizeMode::Move => move_file(&plan.source, &dest_path, cat),
        };
        match result {
            Ok(()) => match mode {
                OrganizeMode::Copy => summary.copied += 1,
                OrganizeMode::Move => summary.moved += 1,
                OrganizeMode::Plan => {}
            },
            Err(err) => summary.failures.push(format!(
                "{} -> {} ({err})",
                plan.source.display(),
                dest_path.display()
            )),
        }
    }
    Ok(summary)
}

fn copy_file(source: &Path, dest: &Path, cat: &TextCatalog) -> Result<(), String> {
    fs::copy(source, dest).map_err(|err| {
        tr_with(
            cat,
            "photo_organize.err.copy_failed",
            &[
                ("source", source.display().to_string()),
                ("dest", dest.display().to_string()),
                ("error", err.to_string()),
            ],
        )
    })?;
    Ok(())
}

fn move_file(source: &Path, dest: &Path, cat: &TextCatalog) -> Result<(), String> {
    match fs::rename(source, dest) {
        Ok(()) => Ok(()),
        Err(_) => {
            copy_file(source, dest, cat)?;
            fs::remove_file(source).map_err(|err| {
                tr_with(
                    cat,
                    "photo_organize.err.remove_original_failed",
                    &[
                        ("source", source.display().to_string()),
                        ("error", err.to_string()),
                    ],
                )
            })
        }
    }
}

fn allocate_destination_path(destination_dir: &Path, file_name: &str) -> PathBuf {
    let candidate = destination_dir.join(file_name);
    if !candidate.exists() {
        return candidate;
    }

    let original = Path::new(file_name);
    let stem = original
        .file_stem()
        .and_then(|v| v.to_str())
        .unwrap_or("photo");
    let ext = original.extension().and_then(|v| v.to_str()).unwrap_or("");
    for idx in 1..10_000 {
        let next_name = if ext.is_empty() {
            format!("{stem}_{idx}")
        } else {
            format!("{stem}_{idx}.{ext}")
        };
        let next_path = destination_dir.join(next_name);
        if !next_path.exists() {
            return next_path;
        }
    }
    destination_dir.join(format!("{stem}_overflow"))
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
    let year_month = capture_time.as_deref().and_then(parse_year_month);
    Some(json!({
        "make": make,
        "model": model,
        "lens_model": lens_model,
        "focal_length": focal_length,
        "captured_at": capture_time,
        "year_month": year_month,
    }))
}

fn exif_string(exif: &exif::Exif, tag: Tag) -> Option<String> {
    exif.get_field(tag, In::PRIMARY)
        .map(|field| field.display_value().with_unit(exif).to_string())
        .map(|raw| raw.trim_matches('"').trim().to_string())
        .filter(|value| !value.is_empty())
}

fn parse_year_month(raw: &str) -> Option<String> {
    let date_part = raw.split_whitespace().next()?;
    let mut parts = date_part.split(':');
    let year = parts.next()?;
    let month = parts.next()?;
    if year.len() == 4 && month.len() == 2 {
        Some(format!("{year}-{month}"))
    } else {
        None
    }
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

    if !obj.contains_key("action") {
        if let Some(action) = inferred.action.take() {
            obj.insert("action".to_string(), Value::String(action));
        }
    }
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
    if !obj.contains_key("mode") {
        if let Some(mode) = inferred.mode.take() {
            obj.insert("mode".to_string(), Value::String(mode.as_str().to_string()));
        }
    }
    if !obj.contains_key("group_by") {
        if let Some(group_by) = inferred.group_by.take() {
            obj.insert(
                "group_by".to_string(),
                Value::Array(
                    group_by
                        .into_iter()
                        .map(|field| Value::String(field.as_arg_str().to_string()))
                        .collect(),
                ),
            );
        }
    }
    if !obj.contains_key("capture_month") {
        if let Some(capture_month) = inferred.capture_month.take() {
            obj.insert("capture_month".to_string(), Value::String(capture_month));
        }
    }
    if !obj.contains_key("selected_brands") && !obj.contains_key("brands") {
        if let Some(selected_brands) = inferred.selected_brands.take() {
            obj.insert(
                "selected_brands".to_string(),
                Value::Array(selected_brands.into_iter().map(Value::String).collect()),
            );
        }
    }
    if !obj.contains_key("include_subdirs") {
        if let Some(include_subdirs) = inferred.include_subdirs.take() {
            obj.insert("include_subdirs".to_string(), Value::Bool(include_subdirs));
        }
    }
    if !obj.contains_key("preview_limit") {
        if let Some(preview_limit) = inferred.preview_limit.take() {
            obj.insert(
                "preview_limit".to_string(),
                Value::from(preview_limit as u64),
            );
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
    inferred.raw_text = Some(trimmed.to_string());
    let text_lower = trimmed.to_lowercase();

    if text_lower.contains("外接硬盘")
        || text_lower.contains("u盘")
        || text_lower.contains("usb")
        || text_lower.contains("候选路径")
        || text_lower.contains("先问")
    {
        inferred.action = Some("prepare".to_string());
    }

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

    if text_lower.contains("不递归")
        || text_lower.contains("不要子目录")
        || text_lower.contains("不要子文件夹")
        || text_lower.contains("只看当前目录")
        || text_lower.contains("只整理当前目录")
    {
        inferred.include_subdirs = Some(false);
    } else if text_lower.contains("递归")
        || text_lower.contains("子目录")
        || text_lower.contains("子文件夹")
        || text_lower.contains("所有子目录")
    {
        inferred.include_subdirs = Some(true);
    }

    if text_lower.contains("移动")
        || text_lower.contains("挪到")
        || text_lower.contains("搬到")
        || text_lower.contains("move ")
        || text_lower.ends_with("move")
    {
        inferred.mode = Some(OrganizeMode::Move);
    }
    if text_lower.contains("复制")
        || text_lower.contains("拷贝")
        || text_lower.contains("备份")
        || text_lower.contains("copy ")
        || text_lower.ends_with("copy")
    {
        inferred.mode = Some(OrganizeMode::Copy);
    }
    if text_lower.contains("预览")
        || text_lower.contains("先看")
        || text_lower.contains("先预览")
        || text_lower.contains("先不要动")
        || text_lower.contains("不要移动")
        || text_lower.contains("仅预览")
        || text_lower.contains("dry run")
        || text_lower.contains("dry-run")
        || text_lower.contains("preview")
    {
        inferred.mode = Some(OrganizeMode::Plan);
    }

    if inferred.preview_limit.is_none() {
        inferred.preview_limit = extract_preview_limit(trimmed);
        if let Some(limit) = inferred.preview_limit {
            inferred
                .notes
                .push(format!("from_text_preview_limit={limit}"));
        }
    }

    if inferred.group_by.is_none() {
        inferred.group_by = infer_group_by_from_text(trimmed, &text_lower);
        if let Some(group_by) = &inferred.group_by {
            inferred.notes.push(format!(
                "from_text_group_by={}",
                group_by
                    .iter()
                    .map(|field| field.as_arg_str())
                    .collect::<Vec<_>>()
                    .join(",")
            ));
        }
    }

    if inferred.capture_month.is_none() {
        inferred.capture_month = infer_capture_month_from_text(trimmed, &text_lower);
        if let Some(capture_month) = &inferred.capture_month {
            inferred
                .notes
                .push(format!("from_text_capture_month={capture_month}"));
        }
    }

    if inferred.selected_brands.is_none() {
        inferred.selected_brands = infer_selected_brands_from_text(trimmed, &text_lower);
        if let Some(selected_brands) = &inferred.selected_brands {
            inferred.notes.push(format!(
                "from_text_selected_brands={}",
                selected_brands.join(",")
            ));
        }
    }

    inferred
}

fn infer_group_by_from_text(text: &str, text_lower: &str) -> Option<Vec<GroupField>> {
    let mut ordered = Vec::new();
    let keywords: &[(GroupField, &[&str])] = &[
        (GroupField::Brand, &["品牌", "牌子", "brand", "make"]),
        (GroupField::Model, &["机型", "型号", "model"]),
        (GroupField::Lens, &["镜头", "lens"]),
        (GroupField::FocalLength, &["焦段", "焦距", "focal"]),
        (GroupField::YearMonth, &["年月", "月份", "month", "date"]),
    ];

    let mut positions = Vec::new();
    for (field, field_keywords) in keywords {
        let mut best: Option<usize> = None;
        for keyword in *field_keywords {
            let pos = if keyword.is_ascii() {
                text_lower.find(keyword)
            } else {
                text.find(keyword)
            };
            if let Some(idx) = pos {
                best = Some(best.map(|current| current.min(idx)).unwrap_or(idx));
            }
        }
        if let Some(pos) = best {
            positions.push((pos, *field));
        }
    }
    positions.sort_by_key(|(pos, _)| *pos);
    for (_, field) in positions {
        push_unique_group_field(&mut ordered, field);
    }

    if ordered.is_empty() {
        let brands = [
            "佳能", "索尼", "尼康", "富士", "松下", "徕卡", "canon", "sony", "nikon", "fujifilm",
        ];
        let matched_brands = brands
            .iter()
            .filter(|name| text_lower.contains(&name.to_lowercase()))
            .count();
        if matched_brands >= 2
            && (text.contains("分开")
                || text_lower.contains("separate")
                || text_lower.contains("split"))
        {
            ordered.push(GroupField::Brand);
        }
    }

    if ordered.is_empty() {
        None
    } else {
        Some(ordered)
    }
}

fn infer_capture_month_from_text(text: &str, text_lower: &str) -> Option<String> {
    if text.contains("这个月") || text.contains("本月") || text_lower.contains("this month") {
        let now = Local::now();
        return Some(format!("{:04}-{:02}", now.year(), now.month()));
    }

    let chars = text.chars().collect::<Vec<_>>();
    for idx in 0..chars.len() {
        if idx + 7 <= chars.len() {
            let candidate = chars[idx..idx + 7].iter().collect::<String>();
            if is_year_month_candidate(&candidate) {
                return Some(candidate.replace('/', "-").replace('.', "-"));
            }
        }
    }
    None
}

fn infer_selected_brands_from_text(text: &str, text_lower: &str) -> Option<Vec<String>> {
    let wants_filter = text.contains("只整理")
        || text.contains("仅整理")
        || text.contains("其他品牌不动")
        || text.contains("其他不要动")
        || text_lower.contains("only organize")
        || text_lower.contains("only sort")
        || text_lower.contains("leave other brands");
    if !wants_filter {
        return None;
    }

    let candidates = [
        ("Canon", &["佳能", "canon"][..]),
        ("Sony", &["索尼", "sony"][..]),
        ("Nikon", &["尼康", "nikon"][..]),
        ("Fujifilm", &["富士", "fujifilm", "fuji"][..]),
        ("Panasonic", &["松下", "panasonic", "lumix"][..]),
        ("Leica", &["徕卡", "leica"][..]),
    ];
    let mut matched = Vec::new();
    for (canonical, aliases) in candidates {
        if aliases.iter().any(|alias| {
            if alias.is_ascii() {
                text_lower.contains(alias)
            } else {
                text.contains(alias)
            }
        }) {
            matched.push(canonical.to_string());
        }
    }
    if matched.is_empty() {
        None
    } else {
        Some(matched)
    }
}

fn is_year_month_candidate(raw: &str) -> bool {
    let normalized = raw.replace('/', "-").replace('.', "-");
    let mut parts = normalized.split('-');
    let Some(year) = parts.next() else {
        return false;
    };
    let Some(month) = parts.next() else {
        return false;
    };
    parts.next().is_none()
        && year.len() == 4
        && month.len() == 2
        && year.chars().all(|ch| ch.is_ascii_digit())
        && month.chars().all(|ch| ch.is_ascii_digit())
}

fn extract_preview_limit(text: &str) -> Option<usize> {
    for marker in ["前", "预览", "显示"] {
        if let Some(idx) = text.find(marker) {
            let tail = &text[idx + marker.len()..];
            let digits = tail
                .chars()
                .skip_while(|ch| !ch.is_ascii_digit())
                .take_while(|ch| ch.is_ascii_digit())
                .collect::<String>();
            if let Ok(value) = digits.parse::<usize>() {
                if (1..=50).contains(&value) {
                    return Some(value);
                }
            }
        }
    }
    None
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
    let mut roots = BTreeSet::new();
    for path in discover_external_roots() {
        roots.insert(path.display().to_string());
        for child in discover_photo_children(&path) {
            roots.insert(child.display().to_string());
        }
    }
    roots.into_iter().collect()
}

fn discover_photo_children(root: &Path) -> Vec<PathBuf> {
    const HINTS: &[&str] = &["DCIM", "Photos", "Pictures", "照片", "相机", "Camera"];
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
        if HINTS.iter().any(|hint| name.eq_ignore_ascii_case(hint)) {
            out.push(path);
        }
    }
    out.sort();
    out
}

fn discover_external_roots() -> Vec<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        discover_roots_in("/Volumes")
    }
    #[cfg(target_os = "linux")]
    {
        let mut roots = Vec::new();
        roots.extend(discover_roots_in("/media"));
        roots.extend(discover_roots_in("/mnt"));
        if let Ok(user) = std::env::var("USER") {
            roots.extend(discover_roots_in(&format!("/run/media/{user}")));
        }
        roots.sort();
        roots.dedup();
        roots
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        Vec::new()
    }
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
    let root_path = Path::new(root);
    let Ok(entries) = fs::read_dir(root_path) else {
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
