use super::*;

pub(super) fn compare_paths(
    workspace_root: &Path,
    obj: &Map<String, Value>,
    allow_path_outside_workspace: bool,
) -> SkillResult<String> {
    let left = required_str(obj, "left_path")?;
    let right = required_str(obj, "right_path")?;
    let left_real = resolve_path(workspace_root, left, allow_path_outside_workspace)?;
    let right_real = resolve_path(workspace_root, right, allow_path_outside_workspace)?;
    let left_meta =
        std::fs::metadata(&left_real).map_err(|err| SkillError::io("metadata", &left_real, err))?;
    let right_meta = std::fs::metadata(&right_real)
        .map_err(|err| SkillError::io("metadata", &right_real, err))?;

    let left_kind = path_kind(&left_meta);
    let right_kind = path_kind(&right_meta);
    let left_mtime = left_meta.modified().ok().and_then(system_time_to_ts);
    let right_mtime = right_meta.modified().ok().and_then(system_time_to_ts);
    let left_name = left_real.file_name().and_then(OsStr::to_str).unwrap_or("");
    let right_name = right_real.file_name().and_then(OsStr::to_str).unwrap_or("");
    let same_content = if left_meta.is_file() && right_meta.is_file() {
        same_file_content(&left_real, &right_real).ok()
    } else {
        None
    };

    Ok(json!({
        "action": "compare_paths",
        "left": build_path_fact(workspace_root, &left_real, &left_meta),
        "right": build_path_fact(workspace_root, &right_real, &right_meta),
        "comparison": {
            "same_kind": left_kind == right_kind,
            "same_name": left_name == right_name,
            "same_size": left_meta.len() == right_meta.len(),
            "size_delta_bytes": (left_meta.len() as i128 - right_meta.len() as i128),
            "left_newer": match (left_mtime, right_mtime) {
                (Some(l), Some(r)) => Some(l > r),
                _ => None,
            },
            "same_content": same_content,
        }
    })
    .to_string())
}

pub(super) fn path_batch_facts(
    workspace_root: &Path,
    obj: &Map<String, Value>,
    allow_path_outside_workspace: bool,
) -> SkillResult<String> {
    let paths = string_list_arg(obj, "paths");
    if paths.is_empty() {
        return Err(SkillError::invalid_input("paths is required"));
    }
    let fields = string_list_arg(obj, "fields");
    let include_missing = bool_arg(obj, "include_missing", true);
    let mut facts = Vec::new();

    for path in paths {
        let real = resolve_path(workspace_root, &path, allow_path_outside_workspace)?;
        match std::fs::metadata(&real) {
            Ok(meta) => facts.push(json!({
                "path": path,
                "exists": true,
                "fact": build_path_fact(workspace_root, &real, &meta),
            })),
            Err(err) if include_missing && err.kind() == io::ErrorKind::NotFound => {
                if let Some(resolved) =
                    resolve_case_insensitive_leaf(&real).or_else(|| resolve_unique_stem_leaf(&real))
                {
                    let meta = std::fs::metadata(&resolved)
                        .map_err(|err| SkillError::io("metadata", &resolved, err))?;
                    facts.push(json!({
                        "path": path,
                        "exists": true,
                        "resolved_from_case_insensitive": case_equivalent_path_leaf(&resolved, &real),
                        "resolved_from_stem": path_leaf_matches_file_stem(&resolved, &real),
                        "fact": build_path_fact(workspace_root, &resolved, &meta),
                    }));
                } else {
                    facts.push(json!({
                        "path": path,
                        "exists": false,
                        "kind": "missing",
                        "error": "not found",
                    }))
                }
            }
            Err(err) => return Err(SkillError::io("metadata", &real, err)),
        }
    }

    let mut response = json!({
        "action": "path_batch_facts",
        "count": facts.len(),
        "include_missing": include_missing,
        "facts": facts,
    });
    if !fields.is_empty() {
        response["fields"] = json!(fields);
    }
    Ok(response.to_string())
}

pub(super) fn resolve_case_insensitive_leaf(path: &Path) -> Option<PathBuf> {
    let parent = path.parent()?;
    let target_name = path.file_name()?.to_str()?;
    let entries = std::fs::read_dir(parent).ok()?;
    for entry in entries.flatten() {
        let candidate_name = entry.file_name();
        let Some(candidate_name) = candidate_name.to_str() else {
            continue;
        };
        if candidate_name.eq_ignore_ascii_case(target_name) {
            return Some(entry.path());
        }
    }
    None
}

pub(super) fn case_equivalent(a: &str, b: &str) -> bool {
    a == b || a.eq_ignore_ascii_case(b) || a.to_lowercase() == b.to_lowercase()
}

pub(super) fn case_equivalent_path_leaf(resolved: &Path, requested: &Path) -> bool {
    match (
        resolved.file_name().and_then(|name| name.to_str()),
        requested.file_name().and_then(|name| name.to_str()),
    ) {
        (Some(resolved), Some(requested)) => case_equivalent(resolved, requested),
        _ => false,
    }
}

pub(super) fn path_leaf_matches_file_stem(resolved: &Path, requested: &Path) -> bool {
    match (
        resolved.file_stem().and_then(|name| name.to_str()),
        requested.file_name().and_then(|name| name.to_str()),
    ) {
        (Some(resolved_stem), Some(requested_leaf)) => {
            !requested_leaf.contains('.') && case_equivalent(resolved_stem, requested_leaf)
        }
        _ => false,
    }
}

pub(super) fn resolve_unique_stem_leaf(path: &Path) -> Option<PathBuf> {
    let parent = path.parent()?;
    let target_name = path.file_name()?.to_str()?;
    if target_name.contains('.') {
        return None;
    }
    let mut matched: Option<PathBuf> = None;
    for entry in std::fs::read_dir(parent).ok()?.flatten() {
        let candidate_path = entry.path();
        if !entry.metadata().ok().is_some_and(|meta| meta.is_file()) {
            continue;
        }
        let Some(candidate_stem) = candidate_path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        if !case_equivalent(candidate_stem, target_name) {
            continue;
        }
        if matched.is_some() {
            return None;
        }
        matched = Some(candidate_path);
    }
    matched
}

pub(super) fn path_kind(meta: &std::fs::Metadata) -> &'static str {
    if meta.is_dir() {
        "dir"
    } else if meta.is_file() {
        "file"
    } else {
        "other"
    }
}

pub(super) fn build_path_fact(
    workspace_root: &Path,
    path: &Path,
    meta: &std::fs::Metadata,
) -> Value {
    json!({
        "path": to_rel(workspace_root, path),
        "resolved_path": path.display().to_string(),
        "kind": path_kind(meta),
        "size_bytes": meta.len(),
        "modified_ts": meta.modified().ok().and_then(system_time_to_ts),
    })
}

pub(super) fn top_extension_pairs(
    counts: &std::collections::BTreeMap<String, usize>,
    limit: usize,
) -> Vec<Value> {
    let mut pairs = counts
        .iter()
        .map(|(ext, count)| (ext.clone(), *count))
        .collect::<Vec<_>>();
    pairs.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    pairs
        .into_iter()
        .take(limit)
        .map(|(ext, count)| json!({ "ext": ext, "count": count }))
        .collect()
}

pub(super) fn build_tree_summary_node(
    workspace_root: &Path,
    path: &Path,
    include_hidden: bool,
    max_depth: usize,
    max_children_per_dir: usize,
    depth: usize,
    state: &mut TreeSummaryState,
) -> SkillResult<Value> {
    if state.remaining_nodes == 0 {
        state.truncated_nodes += 1;
        return Ok(json!({
            "path": to_rel(workspace_root, path),
            "truncated": true,
        }));
    }
    state.remaining_nodes -= 1;

    let meta = std::fs::metadata(path).map_err(|err| SkillError::io("metadata", path, err))?;
    let mut node = build_path_fact(workspace_root, path, &meta);
    if !meta.is_dir() {
        return Ok(node);
    }

    let mut visible_entries: Vec<PathBuf> = std::fs::read_dir(path)
        .map_err(|err| SkillError::io("read_dir", path, err))?
        .filter_map(|entry| entry.ok().map(|v| v.path()))
        .filter(|p| {
            include_hidden
                || p.file_name()
                    .and_then(OsStr::to_str)
                    .map(|v| !v.starts_with('.'))
                    .unwrap_or(true)
        })
        .collect();
    visible_entries.sort_by(|a, b| {
        a.file_name()
            .and_then(OsStr::to_str)
            .unwrap_or("")
            .cmp(b.file_name().and_then(OsStr::to_str).unwrap_or(""))
    });

    let child_count = visible_entries.len();
    let omitted_children = if depth >= max_depth {
        child_count
    } else {
        child_count.saturating_sub(max_children_per_dir)
    };
    let mut children = Vec::new();
    if depth < max_depth {
        for child_path in visible_entries.into_iter().take(max_children_per_dir) {
            children.push(build_tree_summary_node(
                workspace_root,
                &child_path,
                include_hidden,
                max_depth,
                max_children_per_dir,
                depth + 1,
                state,
            )?);
        }
    }

    if let Some(obj) = node.as_object_mut() {
        obj.insert("depth".to_string(), json!(depth));
        obj.insert("child_count".to_string(), json!(child_count));
        obj.insert("omitted_children".to_string(), json!(omitted_children));
        obj.insert("children".to_string(), Value::Array(children));
    }
    Ok(node)
}

pub(super) fn same_file_content(left: &Path, right: &Path) -> SkillResult<bool> {
    const MAX_COMPARE_BYTES: u64 = 4 * 1024 * 1024;
    let left_meta = std::fs::metadata(left).map_err(|err| SkillError::io("metadata", left, err))?;
    let right_meta =
        std::fs::metadata(right).map_err(|err| SkillError::io("metadata", right, err))?;
    if left_meta.len() != right_meta.len() {
        return Ok(false);
    }
    if left_meta.len() > MAX_COMPARE_BYTES {
        return Err(SkillError::invalid_input(format!(
            "file too large to compare content directly: {} bytes exceeds {}",
            left_meta.len(),
            MAX_COMPARE_BYTES
        )));
    }
    let left_bytes = std::fs::read(left).map_err(|err| SkillError::io("read_file", left, err))?;
    let right_bytes =
        std::fs::read(right).map_err(|err| SkillError::io("read_file", right, err))?;
    Ok(left_bytes == right_bytes)
}

pub(super) fn resolve_path(
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

pub(super) fn walk_collect(path: &Path, f: &mut dyn FnMut(&Path) -> bool) -> SkillResult<()> {
    if path.is_file() {
        let _ = f(path);
        return Ok(());
    }
    if path.is_dir() && f(path) {
        return Ok(());
    }
    let meta = std::fs::metadata(path).map_err(|err| SkillError::io("metadata", path, err))?;
    if !meta.is_dir() {
        return Err(SkillError::not_a_directory(format!(
            "path search requires a directory: {}",
            path.display()
        )));
    }
    let iter = std::fs::read_dir(path).map_err(|err| SkillError::io("read_dir", path, err))?;
    for entry in iter {
        let entry = entry.map_err(|err| SkillError::io("dir_entry", path, err))?;
        let p = entry.path();
        if p.is_dir() {
            walk_collect(&p, f)?;
        } else if f(&p) {
            return Ok(());
        }
    }
    Ok(())
}

pub(super) fn to_rel(root: &Path, p: &Path) -> String {
    p.strip_prefix(root)
        .unwrap_or(p)
        .to_string_lossy()
        .to_string()
}

pub(super) fn system_time_to_ts(st: SystemTime) -> Option<u64> {
    st.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs())
}
