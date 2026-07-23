use super::*;

pub(super) fn execute(
    args: &Value,
    cat: &TextCatalog,
    cfg: &PhotoOrganizeConfig,
) -> Result<SkillOutput, String> {
    let normalized = normalize_args(args, cat, cfg)?;
    let obj = normalized
        .as_object()
        .ok_or_else(|| tr(cat, "photo_organize.err.normalized_args_object"))?;
    let action = obj
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or("organize")
        .trim()
        .to_ascii_lowercase();
    if let Some(default_mode) = default_mode_for_action_alias(&action) {
        return handle_organize_with_default_mode(obj, cat, cfg, default_mode);
    }
    match action.as_str() {
        "prepare" | "select_source" => Ok(build_directory_prompt(cat, cfg)),
        "organize" | "run" => handle_organize(obj, cat, cfg),
        other => Err(tr_with(
            cat,
            "photo_organize.err.unsupported_action",
            &[("action", other.to_string())],
        )),
    }
}

pub(super) fn default_mode_for_action_alias(action: &str) -> Option<OrganizeMode> {
    match action {
        "plan" | "preview" | "dry_run" => Some(OrganizeMode::Plan),
        "copy" => Some(OrganizeMode::Copy),
        "move" => Some(OrganizeMode::Move),
        _ => None,
    }
}

pub(super) fn has_mode_arg(obj: &Map<String, Value>) -> bool {
    obj.get("mode")
        .or_else(|| obj.get("organize_mode"))
        .and_then(Value::as_str)
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
}

pub(super) fn handle_organize_with_default_mode(
    obj: &Map<String, Value>,
    cat: &TextCatalog,
    cfg: &PhotoOrganizeConfig,
    default_mode: OrganizeMode,
) -> Result<SkillOutput, String> {
    if has_mode_arg(obj) {
        return handle_organize(obj, cat, cfg);
    }
    let mut normalized = obj.clone();
    normalized.insert(
        "mode".to_string(),
        Value::String(default_mode.as_str().to_string()),
    );
    handle_organize(&normalized, cat, cfg)
}

pub(super) fn handle_organize(
    obj: &Map<String, Value>,
    cat: &TextCatalog,
    cfg: &PhotoOrganizeConfig,
) -> Result<SkillOutput, String> {
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
        Some(raw) if !raw.trim().is_empty() => raw.trim().to_string(),
        _ => match auto_source_dir_from_external_roots(cfg) {
            Some(path) => path.display().to_string(),
            None => return Ok(build_directory_prompt(cat, cfg)),
        },
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
    let options = resolve_organize_options(obj, cfg);

    let source_dir = resolve_existing_dir(&source_dir_raw, cat)?;
    let output_dir = resolve_output_dir(obj, &source_dir)?;
    let photo_files = collect_photo_files(&source_dir, include_subdirs, &output_dir, cat)?;
    if photo_files.is_empty() {
        return Err(tr_with(
            cat,
            "photo_organize.err.no_photos_found",
            &[("path", source_dir.display().to_string())],
        ));
    }

    let mut build_result =
        build_photo_plans(&source_dir, &output_dir, photo_files, &options, cat, cfg)?;
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
                "capture_year": plan.capture_year,
                "capture_month": plan.year_month,
                "capture_date": plan.capture_date,
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
            "capture_year": options.capture_year,
            "capture_month": options.capture_month,
            "capture_date": options.capture_date,
            "selected_brands": options.selected_brands,
            "selected_models": options.selected_models,
            "selected_lenses": options.selected_lenses,
            "non_exif_files": build_result.non_exif_files,
            "preview": preview,
        })),
    })
}

pub(super) fn build_directory_prompt(cat: &TextCatalog, cfg: &PhotoOrganizeConfig) -> SkillOutput {
    let candidates = discover_external_photo_candidates(cfg);
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
            "requires_user_input": true,
            "missing_argument": "source_dir",
            "needs_directory": true,
            "external_candidates": candidates,
            "recommended_mode": "plan",
        })),
    }
}

pub(super) fn auto_source_dir_from_external_roots(cfg: &PhotoOrganizeConfig) -> Option<PathBuf> {
    preferred_auto_source_root(discover_external_roots(cfg))
}

pub(super) fn preferred_auto_source_root(roots: Vec<PathBuf>) -> Option<PathBuf> {
    if roots.len() == 1 {
        roots.into_iter().next()
    } else {
        None
    }
}

pub(super) fn platform_hint_lines(cat: &TextCatalog) -> Vec<String> {
    match current_platform() {
        #[cfg(target_os = "macos")]
        HostPlatform::MacOS => vec![
            tr(cat, "photo_organize.msg.no_external_candidates"),
            tr(cat, "photo_organize.msg.macos_hint"),
        ],
        #[cfg(target_os = "linux")]
        HostPlatform::Linux => vec![
            tr(cat, "photo_organize.msg.no_external_candidates"),
            tr(cat, "photo_organize.msg.linux_hint"),
        ],
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        HostPlatform::Other => vec![
            tr(cat, "photo_organize.msg.no_external_candidates"),
            tr(cat, "photo_organize.msg.other_os_hint"),
        ],
    }
}

pub(super) fn resolve_existing_dir(raw: &str, cat: &TextCatalog) -> Result<PathBuf, String> {
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

pub(super) fn resolve_output_dir(
    obj: &Map<String, Value>,
    source_dir: &Path,
) -> Result<PathBuf, String> {
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

pub(super) fn resolve_organize_options(
    obj: &Map<String, Value>,
    cfg: &PhotoOrganizeConfig,
) -> OrganizeOptions {
    let group_by = parse_group_by_value(obj.get("group_by"))
        .filter(|fields| !fields.is_empty())
        .unwrap_or_else(GroupField::defaults);
    let capture_year = obj
        .get("capture_year")
        .or_else(|| obj.get("year"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(normalize_capture_year);
    let capture_month = obj
        .get("capture_month")
        .or_else(|| obj.get("month"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(normalize_capture_month);
    let capture_date = obj
        .get("capture_date")
        .or_else(|| obj.get("date"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(normalize_capture_date);
    let selected_brands = parse_string_list(
        obj.get("selected_brands")
            .or_else(|| obj.get("brands"))
            .or_else(|| obj.get("camera_brands")),
    )
    .into_iter()
    .filter_map(|brand| canonical_brand_name(&brand, cfg))
    .collect::<Vec<_>>();
    let selected_models = parse_selector_list(
        obj.get("selected_models")
            .or_else(|| obj.get("models"))
            .or_else(|| obj.get("camera_models")),
    );
    let selected_lenses = parse_selector_list(
        obj.get("selected_lenses")
            .or_else(|| obj.get("lenses"))
            .or_else(|| obj.get("lens_models")),
    );
    OrganizeOptions {
        group_by,
        capture_year,
        capture_month,
        capture_date,
        selected_brands,
        selected_models,
        selected_lenses,
    }
}

pub(super) fn parse_group_by_value(value: Option<&Value>) -> Option<Vec<GroupField>> {
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

pub(super) fn push_unique_group_field(out: &mut Vec<GroupField>, field: GroupField) {
    if !out.contains(&field) {
        out.push(field);
    }
}

pub(super) fn normalize_capture_year(raw: &str) -> String {
    let trimmed = raw.trim();
    let year = trimmed.chars().take(4).collect::<String>();
    if year.len() == 4 && year.chars().all(|ch| ch.is_ascii_digit()) {
        year
    } else {
        trimmed.to_string()
    }
}

pub(super) fn normalize_capture_month(raw: &str) -> String {
    let normalized = raw.trim().replace('/', "-").replace('.', "-");
    if normalized.len() == 6 && normalized.chars().all(|ch| ch.is_ascii_digit()) {
        return format!("{}-{}", &normalized[0..4], &normalized[4..6]);
    }
    let parts = normalized.split('-').collect::<Vec<_>>();
    if parts.len() >= 2
        && parts[0].len() == 4
        && parts[0].chars().all(|ch| ch.is_ascii_digit())
        && !parts[1].is_empty()
        && parts[1].len() <= 2
        && parts[1].chars().all(|ch| ch.is_ascii_digit())
    {
        if let Ok(month) = parts[1].parse::<u8>() {
            if (1..=12).contains(&month) {
                return format!("{}-{month:02}", parts[0]);
            }
        }
    }
    normalized
}

pub(super) fn normalize_capture_date(raw: &str) -> String {
    let normalized = raw
        .trim()
        .replace('/', "-")
        .replace('.', "-")
        .replace(':', "-");
    if normalized.len() == 8 && normalized.chars().all(|ch| ch.is_ascii_digit()) {
        return format!(
            "{}-{}-{}",
            &normalized[0..4],
            &normalized[4..6],
            &normalized[6..8]
        );
    }
    let parts = normalized.split('-').collect::<Vec<_>>();
    if parts.len() >= 3
        && parts[0].len() == 4
        && parts[0].chars().all(|ch| ch.is_ascii_digit())
        && !parts[1].is_empty()
        && parts[1].len() <= 2
        && parts[1].chars().all(|ch| ch.is_ascii_digit())
        && !parts[2].is_empty()
        && parts[2].len() <= 2
        && parts[2].chars().all(|ch| ch.is_ascii_digit())
    {
        if let (Ok(month), Ok(day)) = (parts[1].parse::<u8>(), parts[2].parse::<u8>()) {
            if (1..=12).contains(&month) && (1..=31).contains(&day) {
                return format!("{}-{month:02}-{day:02}", parts[0]);
            }
        }
    }
    normalized
}

pub(super) fn parse_string_list(value: Option<&Value>) -> Vec<String> {
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

pub(super) fn parse_selector_list(value: Option<&Value>) -> Vec<String> {
    let Some(value) = value else {
        return Vec::new();
    };
    let mut out = Vec::new();
    match value {
        Value::String(text) => {
            for token in text.split([',', '|', '、', '，']) {
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

pub(super) fn canonical_brand_name(raw: &str, cfg: &PhotoOrganizeConfig) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    configured_camera_brand_aliases(cfg)
        .iter()
        .find_map(|entry| {
            entry
                .aliases
                .iter()
                .any(|alias| token_matches_brand_alias(trimmed, alias))
                .then(|| entry.canonical.trim().to_string())
        })
        .or_else(|| {
            static_camera_brand_aliases().iter().find_map(|entry| {
                entry
                    .aliases
                    .iter()
                    .any(|alias| token_matches_brand_alias(trimmed, alias))
                    .then(|| entry.canonical.to_string())
            })
        })
        .or_else(|| Some(trimmed.to_string()))
}

pub(super) fn brand_matches(
    camera_make: &str,
    selected_brands: &[String],
    cfg: &PhotoOrganizeConfig,
) -> bool {
    if selected_brands.is_empty() {
        return true;
    }
    let make_canonical = canonical_brand_name(camera_make, cfg);
    let make_lower = camera_make.to_ascii_lowercase();
    selected_brands.iter().any(|brand| {
        let selected_canonical = canonical_brand_name(brand, cfg);
        if make_canonical.is_some() && make_canonical == selected_canonical {
            return true;
        }
        let Some(canonical) = selected_canonical else {
            return false;
        };
        let brand_lower = canonical.to_ascii_lowercase();
        !brand_lower.is_empty() && brand_lower.is_ascii() && make_lower.contains(&brand_lower)
    })
}

struct CameraBrandAlias {
    canonical: &'static str,
    aliases: &'static [&'static str],
}

fn static_camera_brand_aliases() -> &'static [CameraBrandAlias] {
    const ALIASES: &[CameraBrandAlias] = &[
        CameraBrandAlias {
            canonical: "Canon",
            aliases: &["canon"],
        },
        CameraBrandAlias {
            canonical: "Sony",
            aliases: &["sony"],
        },
        CameraBrandAlias {
            canonical: "Nikon",
            aliases: &["nikon"],
        },
        CameraBrandAlias {
            canonical: "Fujifilm",
            aliases: &["fujifilm", "fuji"],
        },
        CameraBrandAlias {
            canonical: "Panasonic",
            aliases: &["panasonic", "lumix"],
        },
        CameraBrandAlias {
            canonical: "Leica",
            aliases: &["leica"],
        },
    ];
    ALIASES
}

fn configured_camera_brand_aliases(cfg: &PhotoOrganizeConfig) -> &[CameraBrandAliasConfig] {
    cfg.camera_brand_aliases.as_deref().unwrap_or_default()
}

fn token_matches_brand_alias(token: &str, alias: &str) -> bool {
    if alias.is_ascii() {
        token.eq_ignore_ascii_case(alias)
    } else {
        token == alias
    }
}

pub(super) fn text_matches_any(value: Option<&str>, selectors: &[String]) -> bool {
    if selectors.is_empty() {
        return true;
    }
    let Some(value) = value else {
        return false;
    };
    let value_lower = value.to_ascii_lowercase();
    selectors.iter().any(|selector| {
        let selector_lower = selector.trim().to_ascii_lowercase();
        !selector_lower.is_empty() && value_lower.contains(&selector_lower)
    })
}

pub(super) fn collect_photo_files(
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

pub(super) fn collect_photo_files_inner(
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

pub(super) fn build_photo_plans(
    source_dir: &Path,
    output_dir: &Path,
    photo_files: Vec<PathBuf>,
    options: &OrganizeOptions,
    cat: &TextCatalog,
    cfg: &PhotoOrganizeConfig,
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
        let capture_year = metadata
            .as_ref()
            .and_then(|meta| meta.get("capture_year"))
            .and_then(Value::as_str)
            .map(str::to_string);
        let year_month = metadata
            .as_ref()
            .and_then(|meta| meta.get("year_month"))
            .and_then(Value::as_str)
            .map(str::to_string);
        let capture_date = metadata
            .as_ref()
            .and_then(|meta| meta.get("capture_date"))
            .and_then(Value::as_str)
            .map(str::to_string);
        if let Some(make_text) = make.as_deref() {
            if !brand_matches(make_text, &options.selected_brands, cfg) {
                continue;
            }
        } else if !options.selected_brands.is_empty() {
            continue;
        }
        if !text_matches_any(model.as_deref(), &options.selected_models) {
            continue;
        }
        if !text_matches_any(lens_model.as_deref(), &options.selected_lenses) {
            continue;
        }
        if let Some(capture_year_filter) = &options.capture_year {
            if capture_year.as_deref() != Some(capture_year_filter.as_str()) {
                continue;
            }
        }
        if let Some(capture_month) = &options.capture_month {
            if year_month.as_deref() != Some(capture_month.as_str()) {
                continue;
            }
        }
        if let Some(capture_date_filter) = &options.capture_date {
            if capture_date.as_deref() != Some(capture_date_filter.as_str()) {
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
        let year_dir = capture_year
            .as_deref()
            .map(sanitize_component)
            .unwrap_or_else(|| "undated_year".to_string());
        let date_dir = year_month
            .as_deref()
            .map(sanitize_component)
            .unwrap_or_else(|| "undated".to_string());
        let day_dir = capture_date
            .as_deref()
            .map(sanitize_component)
            .unwrap_or_else(|| "undated_day".to_string());
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
                GroupField::Year => &year_dir,
                GroupField::YearMonth => &date_dir,
                GroupField::Date => &day_dir,
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
            capture_year,
            year_month,
            capture_date,
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

pub(super) fn build_plan_output(
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
                "capture_year": plan.capture_year,
                "year_month": plan.year_month,
                "capture_date": plan.capture_date,
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
            "capture_year": options.capture_year,
            "capture_month": options.capture_month,
            "capture_date": options.capture_date,
            "selected_brands": options.selected_brands,
            "selected_models": options.selected_models,
            "selected_lenses": options.selected_lenses,
            "non_exif_files": non_exif_files,
            "top_camera_groups": top_groups,
            "top_lens_groups": top_lens_groups,
            "preview": preview,
        })),
    }
}

pub(super) fn apply_plan(
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

pub(super) fn copy_file(source: &Path, dest: &Path, cat: &TextCatalog) -> Result<(), String> {
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

pub(super) fn move_file(source: &Path, dest: &Path, cat: &TextCatalog) -> Result<(), String> {
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

pub(super) fn allocate_destination_path(destination_dir: &Path, file_name: &str) -> PathBuf {
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
