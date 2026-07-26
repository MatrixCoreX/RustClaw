use super::*;

const DEFAULT_MAX_PATHS: usize = 200;
const MAX_MAX_PATHS: usize = 1_000;
const MAX_VISITED_NODES: usize = 200_000;
const MAX_DEPTH: usize = 128;

#[derive(Default)]
struct StructureSummary {
    node_count: usize,
    scalar_count: usize,
    empty_string_count: usize,
    null_count: usize,
    empty_container_count: usize,
    false_boolean_count: usize,
    empty_string_paths: Vec<String>,
    null_paths: Vec<String>,
    empty_container_paths: Vec<String>,
    false_boolean_paths: Vec<String>,
    scan_complete: bool,
}

pub(super) fn summarize_structured(
    workspace_root: &Path,
    obj: &Map<String, Value>,
    allow_path_outside_workspace: bool,
) -> SkillResult<String> {
    let path = required_str(obj, "path")?;
    let real = resolve_path(workspace_root, path, allow_path_outside_workspace)?;
    let field_path = obj
        .get("field_path")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let max_paths =
        u64_arg(obj, "max_paths", DEFAULT_MAX_PATHS as u64).clamp(1, MAX_MAX_PATHS as u64) as usize;
    let (format, root_value) =
        parse_structured_root(&real, obj.get("format").and_then(Value::as_str))?;

    let target = if field_path.is_empty() {
        Some(&root_value)
    } else {
        lookup_field_value(&root_value, field_path)
    };
    let Some(target) = target else {
        return Ok(json!({
            "action": "summarize_structured",
            "path": path,
            "resolved_path": real.display().to_string(),
            "format": format,
            "field_path": field_path,
            "exists": false,
            "scan_complete": true,
            "node_count": 0,
            "scalar_count": 0,
            "empty_value_count": 0,
            "empty_string_count": 0,
            "empty_string_paths": [],
            "null_count": 0,
            "null_paths": [],
            "empty_container_count": 0,
            "empty_container_paths": [],
            "false_boolean_count": 0,
            "false_boolean_paths": [],
            "paths_omitted": 0,
        })
        .to_string());
    };

    let mut summary = StructureSummary {
        scan_complete: true,
        ..StructureSummary::default()
    };
    collect_structure_summary(target, field_path, 0, max_paths, &mut summary);
    summary.empty_string_paths.sort();
    summary.null_paths.sort();
    summary.empty_container_paths.sort();
    summary.false_boolean_paths.sort();
    let retained_path_count = summary.empty_string_paths.len()
        + summary.null_paths.len()
        + summary.empty_container_paths.len()
        + summary.false_boolean_paths.len();
    let total_path_count = summary.empty_string_count
        + summary.null_count
        + summary.empty_container_count
        + summary.false_boolean_count;

    Ok(json!({
        "action": "summarize_structured",
        "path": path,
        "resolved_path": real.display().to_string(),
        "format": format,
        "field_path": field_path,
        "exists": true,
        "scan_complete": summary.scan_complete,
        "node_count": summary.node_count,
        "scalar_count": summary.scalar_count,
        "empty_value_count": summary.empty_string_count + summary.null_count + summary.empty_container_count,
        "empty_string_count": summary.empty_string_count,
        "empty_string_paths": summary.empty_string_paths,
        "null_count": summary.null_count,
        "null_paths": summary.null_paths,
        "empty_container_count": summary.empty_container_count,
        "empty_container_paths": summary.empty_container_paths,
        "false_boolean_count": summary.false_boolean_count,
        "false_boolean_paths": summary.false_boolean_paths,
        "paths_omitted": total_path_count.saturating_sub(retained_path_count),
    })
    .to_string())
}

fn collect_structure_summary(
    value: &Value,
    path: &str,
    depth: usize,
    max_paths: usize,
    summary: &mut StructureSummary,
) {
    if summary.node_count >= MAX_VISITED_NODES || depth > MAX_DEPTH {
        summary.scan_complete = false;
        return;
    }
    summary.node_count += 1;

    match value {
        Value::Null => {
            summary.scalar_count += 1;
            summary.null_count += 1;
            retain_path(&mut summary.null_paths, path, max_paths);
        }
        Value::Bool(value) => {
            summary.scalar_count += 1;
            if !value {
                summary.false_boolean_count += 1;
                retain_path(&mut summary.false_boolean_paths, path, max_paths);
            }
        }
        Value::Number(_) => summary.scalar_count += 1,
        Value::String(value) => {
            summary.scalar_count += 1;
            if value.is_empty() {
                summary.empty_string_count += 1;
                retain_path(&mut summary.empty_string_paths, path, max_paths);
            }
        }
        Value::Array(items) => {
            if items.is_empty() {
                summary.empty_container_count += 1;
                retain_path(&mut summary.empty_container_paths, path, max_paths);
            }
            for (index, child) in items.iter().enumerate() {
                let child_path = if path.is_empty() {
                    format!("[{index}]")
                } else {
                    format!("{path}[{index}]")
                };
                collect_structure_summary(child, &child_path, depth + 1, max_paths, summary);
                if !summary.scan_complete {
                    break;
                }
            }
        }
        Value::Object(map) => {
            if map.is_empty() {
                summary.empty_container_count += 1;
                retain_path(&mut summary.empty_container_paths, path, max_paths);
            }
            for (key, child) in map {
                let child_path = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path}.{key}")
                };
                collect_structure_summary(child, &child_path, depth + 1, max_paths, summary);
                if !summary.scan_complete {
                    break;
                }
            }
        }
    }
}

fn retain_path(paths: &mut Vec<String>, path: &str, max_paths: usize) {
    if paths.len() < max_paths {
        paths.push(path.to_string());
    }
}
