use super::*;

#[derive(Debug, Clone)]
pub(super) struct TextCatalog {
    current: HashMap<String, String>,
}

pub(super) fn resolve_lang(req: &Req, cfg: &RootConfig) -> String {
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

pub(super) fn normalize_lang_tag(raw: &str) -> String {
    let lower = raw.trim().to_ascii_lowercase().replace('_', "-");
    match lower.as_str() {
        "zh" | "zh-cn" => "zh-CN".to_string(),
        "en" | "en-us" => "en-US".to_string(),
        _ => raw.trim().to_string(),
    }
}

pub(super) fn load_external_i18n(path: &Path) -> Option<HashMap<String, String>> {
    let raw = std::fs::read_to_string(path).ok()?;
    let value: TomlValue = toml::from_str(&raw).ok()?;
    let dict = value.get("dict")?.as_table()?;
    let mut out = HashMap::new();
    for (k, v) in dict {
        collect_i18n_entries(k, v, &mut out);
    }
    Some(out)
}

pub(super) fn collect_i18n_entries(
    prefix: &str,
    value: &TomlValue,
    out: &mut HashMap<String, String>,
) {
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

pub(super) fn default_i18n_dict() -> HashMap<String, String> {
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
        "Unsupported action `{action}`; allowed: prepare|organize|plan|preview|dry_run|copy|move"
            .to_string(),
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
        "photo_organize.err.no_photos_for_brands".to_string(),
        "No photos matching brands {brands} were found under `{path}`.".to_string(),
    );
    m.insert(
        "photo_organize.err.no_photos_for_filters".to_string(),
        "No photos matching filter `{filter_desc}` were found under `{path}`.".to_string(),
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
        "photo_organize.msg.group_field.year".to_string(),
        "year".to_string(),
    );
    m.insert(
        "photo_organize.msg.group_field.year_month".to_string(),
        "year-month".to_string(),
    );
    m.insert(
        "photo_organize.msg.group_field.date".to_string(),
        "date".to_string(),
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
        "photo_organize.msg.filter.capture_year".to_string(),
        "only photos shot in {capture_year}".to_string(),
    );
    m.insert(
        "photo_organize.msg.filter.capture_date".to_string(),
        "only photos shot on {capture_date}".to_string(),
    );
    m.insert(
        "photo_organize.msg.filter.selected_brands".to_string(),
        "only photos with brands {brands}".to_string(),
    );
    m.insert(
        "photo_organize.msg.filter.selected_models".to_string(),
        "only photos with camera models {models}".to_string(),
    );
    m.insert(
        "photo_organize.msg.filter.selected_lenses".to_string(),
        "only photos with lenses {lenses}".to_string(),
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
        "Common Linux / Raspberry Pi path examples: `/media/<user>/<disk-name>`, `/media/pi/<disk-name>`, `/mnt/<disk-name>`, or `/mnt/usb0`.".to_string(),
    );
    m.insert(
        "photo_organize.msg.other_os_hint".to_string(),
        "This skill has explicit mount-path discovery for macOS and Linux. On the current OS, please provide the photo directory manually as an absolute path.".to_string(),
    );
    m.insert(
        "photo_organize.msg.directory_prompt".to_string(),
        "The photo directory could not be determined uniquely.\n\nDetected external drive / USB candidate paths:\n{candidates}\n\nIf exactly one external drive is connected, this skill will automatically use it and continue with a preview. In the current case, call `photo_organize` again with an explicit `source_dir`. By default it organizes by brand/model/lens/focal length/year-month. Start with `mode=\"plan\"` to preview before using `copy` or `move`.\nExample: {{\"action\":\"organize\",\"source_dir\":\"/media/pi/SDCARD/DCIM\",\"mode\":\"plan\"}}\nNatural language also works: \"Organize the photos in /media/pi/SDCARD/DCIM, preview first, do not move originals\"".to_string(),
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

pub(super) fn load_catalog(
    workspace_root: &Path,
    cfg: &PhotoOrganizeConfig,
    lang: &str,
) -> TextCatalog {
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

pub(super) fn tr(cat: &TextCatalog, key: &str) -> String {
    cat.current
        .get(key)
        .cloned()
        .unwrap_or_else(|| key.to_string())
}

pub(super) fn tr_with(cat: &TextCatalog, key: &str, vars: &[(&str, String)]) -> String {
    let mut out = tr(cat, key);
    for (name, value) in vars {
        out = out.replace(&format!("{{{name}}}"), value);
    }
    out
}

pub(super) fn group_by_display(cat: &TextCatalog, fields: &[GroupField]) -> String {
    fields
        .iter()
        .map(|field| tr(cat, field.i18n_key()))
        .collect::<Vec<_>>()
        .join(" / ")
}

pub(super) fn filter_display(cat: &TextCatalog, options: &OrganizeOptions) -> String {
    let mut parts = Vec::new();
    if let Some(capture_year) = &options.capture_year {
        parts.push(tr_with(
            cat,
            "photo_organize.msg.filter.capture_year",
            &[("capture_year", capture_year.clone())],
        ));
    }
    if let Some(capture_month) = &options.capture_month {
        parts.push(tr_with(
            cat,
            "photo_organize.msg.filter.capture_month",
            &[("capture_month", capture_month.clone())],
        ));
    }
    if let Some(capture_date) = &options.capture_date {
        parts.push(tr_with(
            cat,
            "photo_organize.msg.filter.capture_date",
            &[("capture_date", capture_date.clone())],
        ));
    }
    if !options.selected_brands.is_empty() {
        parts.push(tr_with(
            cat,
            "photo_organize.msg.filter.selected_brands",
            &[("brands", options.selected_brands.join(" / "))],
        ));
    }
    if !options.selected_models.is_empty() {
        parts.push(tr_with(
            cat,
            "photo_organize.msg.filter.selected_models",
            &[("models", options.selected_models.join(" / "))],
        ));
    }
    if !options.selected_lenses.is_empty() {
        parts.push(tr_with(
            cat,
            "photo_organize.msg.filter.selected_lenses",
            &[("lenses", options.selected_lenses.join(" / "))],
        ));
    }
    if parts.is_empty() {
        tr(cat, "photo_organize.msg.filter.none")
    } else {
        parts.join("；")
    }
}

pub(super) fn build_no_matches_error(
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
    if !options.selected_brands.is_empty()
        && options.capture_year.is_none()
        && options.capture_month.is_none()
        && options.capture_date.is_none()
        && options.selected_models.is_empty()
        && options.selected_lenses.is_empty()
    {
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
        if options.capture_year.is_none()
            && options.capture_date.is_none()
            && options.selected_brands.is_empty()
            && options.selected_models.is_empty()
            && options.selected_lenses.is_empty()
        {
            return tr_with(
                cat,
                "photo_organize.err.no_photos_for_month",
                &[
                    ("path", source_dir.display().to_string()),
                    ("capture_month", capture_month.clone()),
                ],
            );
        }
    }
    if options.has_filters() {
        return tr_with(
            cat,
            "photo_organize.err.no_photos_for_filters",
            &[
                ("path", source_dir.display().to_string()),
                ("filter_desc", filter_display(cat, options)),
            ],
        );
    }
    tr_with(
        cat,
        "photo_organize.err.no_photos_found",
        &[("path", source_dir.display().to_string())],
    )
}

pub(super) fn non_exif_list_text(cat: &TextCatalog, non_exif_files: &[String]) -> Option<String> {
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
