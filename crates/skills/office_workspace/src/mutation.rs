use crate::docx_write::{create_docx, edit_docx};
use crate::engine;
use crate::error::{OfficeError, OfficeResult};
use crate::model::OfficeFormat;
use crate::operations::{normalize_operations, NormalizedOperation};
use crate::package::{resolve_input_path, OfficePackage};
use crate::package_write::{publish_package, resolve_output_path};
use crate::pptx_edit::edit_pptx;
use crate::pptx_write::create_pptx;
use crate::xlsx_edit::edit_xlsx;
use crate::xlsx_write::create_xlsx;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::path::Path;

struct MutationBuild {
    members: BTreeMap<String, Vec<u8>>,
    changed_refs: Vec<String>,
    preservation: Vec<String>,
}

pub fn execute_mutation(action: &str, object: &Map<String, Value>) -> OfficeResult<Value> {
    let format = action_format(action)?;
    let preview = action.contains(".preview_");
    let editing = action.ends_with(".edit");
    let output_path = resolve_output_path(required_string(object, "output_path")?)?;
    require_extension(&output_path, format)?;
    let operations = normalize_operations(object.get("operations"), format, editing)?;
    let overwrite = object
        .get("overwrite")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let in_place = object
        .get("in_place")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if in_place && !editing {
        return Err(OfficeError::invalid(
            "in_place is valid only for edit capabilities",
        ));
    }

    let (source, build) = if editing {
        let source_path = resolve_input_path(required_string(object, "source_path")?)?;
        let expected_hash = required_string(object, "source_sha256")?.to_ascii_lowercase();
        let source = OfficePackage::open(&source_path, Some(format))?;
        if source.source.sha256 != expected_hash {
            return Err(OfficeError::new(
                "source_conflict",
                "source package hash does not match the requested revision",
                json!({
                    "source_path": source_path.display().to_string(),
                    "expected_sha256": expected_hash,
                    "actual_sha256": source.source.sha256
                }),
            ));
        }
        if in_place && source_path != output_path {
            return Err(OfficeError::new(
                "invalid_in_place_target",
                "in-place edit requires output_path to equal source_path",
                json!({
                    "source_path": source_path.display().to_string(),
                    "output_path": output_path.display().to_string()
                }),
            ));
        }
        let build = edit_package(format, &source, &operations)?;
        (Some(source), build)
    } else if let Some(template_path) = object.get("template_path").and_then(Value::as_str) {
        let template_path = resolve_input_path(template_path)?;
        let template = OfficePackage::open(&template_path, Some(format))?;
        let build = edit_package(format, &template, &operations)?;
        (Some(template), build)
    } else {
        (None, create_package(format, &operations)?)
    };

    if preview {
        return Ok(preview_value(
            format,
            editing,
            &output_path,
            overwrite,
            in_place,
            source.as_ref(),
            &operations,
            &build,
        ));
    }
    let in_place_source = in_place
        .then(|| source.as_ref().map(|source| Path::new(&source.source.path)))
        .flatten();
    let publish = publish_package(
        &build.members,
        &output_path,
        format,
        overwrite || in_place,
        in_place_source,
        source.as_ref().map(|source| source.source.sha256.as_str()),
    )?;
    let mut value = engine::inspect_output(&publish.output_path, format)?;
    attach_mutation_evidence(
        &mut value,
        action,
        source.as_ref(),
        &operations,
        &build,
        &publish,
        in_place,
    );
    Ok(value)
}

fn create_package(
    format: OfficeFormat,
    operations: &[NormalizedOperation],
) -> OfficeResult<MutationBuild> {
    match format {
        OfficeFormat::Docx => create_docx(operations).map(|result| MutationBuild {
            members: result.members,
            changed_refs: result.changed_refs,
            preservation: result.preservation,
        }),
        OfficeFormat::Xlsx => create_xlsx(operations).map(|result| MutationBuild {
            members: result.members,
            changed_refs: result.changed_refs,
            preservation: result.preservation,
        }),
        OfficeFormat::Pptx => create_pptx(operations).map(|result| MutationBuild {
            members: result.members,
            changed_refs: result.changed_refs,
            preservation: result.preservation,
        }),
    }
}

fn edit_package(
    format: OfficeFormat,
    source: &OfficePackage,
    operations: &[NormalizedOperation],
) -> OfficeResult<MutationBuild> {
    match format {
        OfficeFormat::Docx => edit_docx(&source.members, operations).map(|result| MutationBuild {
            members: result.members,
            changed_refs: result.changed_refs,
            preservation: result.preservation,
        }),
        OfficeFormat::Xlsx => edit_xlsx(source, operations).map(|result| MutationBuild {
            members: result.members,
            changed_refs: result.changed_refs,
            preservation: result.preservation,
        }),
        OfficeFormat::Pptx => edit_pptx(source, operations).map(|result| MutationBuild {
            members: result.members,
            changed_refs: result.changed_refs,
            preservation: result.preservation,
        }),
    }
}

fn preview_value(
    format: OfficeFormat,
    editing: bool,
    output_path: &Path,
    overwrite: bool,
    in_place: bool,
    source: Option<&OfficePackage>,
    operations: &[NormalizedOperation],
    build: &MutationBuild,
) -> Value {
    json!({
        "schema_version": 1,
        "format": format,
        "preview": true,
        "writes_performed": false,
        "mutation_mode": if editing { "edit" } else { "create" },
        "source": source.map(|source| &source.source),
        "expected_output_path": output_path.display().to_string(),
        "overwrite": overwrite,
        "in_place": in_place,
        "normalized_operations": operations.iter().map(NormalizedOperation::as_value).collect::<Vec<_>>(),
        "operation_log": operations.iter().map(|operation| operation.record("validated")).collect::<Vec<_>>(),
        "changed_object_refs": build.changed_refs,
        "preservation_report": build.preservation,
        "package_member_count_after": build.members.len(),
        "validation": {
            "valid": true,
            "checks": [
                "operation_schema_valid",
                "selectors_resolved",
                "source_precondition_valid",
                "output_extension_valid",
                "no_output_written"
            ],
            "errors": []
        },
        "artifacts": [],
    })
}

fn attach_mutation_evidence(
    value: &mut Value,
    action: &str,
    source: Option<&OfficePackage>,
    operations: &[NormalizedOperation],
    build: &MutationBuild,
    publish: &crate::package_write::PublishEvidence,
    in_place: bool,
) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    object.insert("preview".to_string(), Value::Bool(false));
    object.insert("writes_performed".to_string(), Value::Bool(true));
    object.insert("action".to_string(), Value::String(action.to_string()));
    object.insert(
        "operation_log".to_string(),
        serde_json::to_value(
            operations
                .iter()
                .map(|operation| operation.record("applied"))
                .collect::<Vec<_>>(),
        )
        .unwrap_or_else(|_| Value::Array(Vec::new())),
    );
    object.insert("changed_object_refs".to_string(), json!(build.changed_refs));
    object.insert("preservation_report".to_string(), json!(build.preservation));
    object.insert(
        "revision_lineage".to_string(),
        json!({
            "parent_sha256": source.map(|source| source.source.sha256.clone()),
            "output_sha256": publish.output_sha256,
            "in_place": in_place,
            "backup_path": publish.backup_path.as_ref().map(|path| path.display().to_string()),
        }),
    );
    object.insert(
        "transaction_recovery".to_string(),
        json!({
            "abandoned_temp_files_removed": publish.abandoned_temp_files_removed,
            "cleanup_error_kinds": publish.temp_cleanup_errors,
        }),
    );
    if let Some(source_object) = object.get_mut("source").and_then(Value::as_object_mut) {
        source_object.insert(
            "parent_sha256".to_string(),
            source
                .map(|source| Value::String(source.source.sha256.clone()))
                .unwrap_or(Value::Null),
        );
    }
    let artifacts = object
        .entry("artifacts")
        .or_insert_with(|| Value::Array(Vec::new()));
    if let Some(artifacts) = artifacts.as_array_mut() {
        artifacts.push(json!({
            "kind": "office_output",
            "path": publish.output_path.display().to_string(),
            "sha256": publish.output_sha256,
        }));
        if let Some(backup) = &publish.backup_path {
            artifacts.push(json!({
                "kind": "office_backup",
                "path": backup.display().to_string(),
                "parent_sha256": source.map(|source| source.source.sha256.clone()),
            }));
        }
    }
    if let Some(validation) = object.get_mut("validation").and_then(Value::as_object_mut) {
        if let Some(checks) = validation.get_mut("checks").and_then(Value::as_array_mut) {
            checks.extend([
                Value::String("operation_batch_validated_before_write".to_string()),
                Value::String("temporary_package_fsynced".to_string()),
                Value::String("output_reopened_after_write".to_string()),
                Value::String("atomic_publish_complete".to_string()),
            ]);
        }
    }
}

fn action_format(action: &str) -> OfficeResult<OfficeFormat> {
    if action.starts_with("word.") {
        Ok(OfficeFormat::Docx)
    } else if action.starts_with("spreadsheet.") {
        Ok(OfficeFormat::Xlsx)
    } else if action.starts_with("presentation.") {
        Ok(OfficeFormat::Pptx)
    } else {
        Err(OfficeError::new(
            "unsupported_action",
            "mutation action does not select an Office format",
            json!({"action": action}),
        ))
    }
}

fn require_extension(path: &Path, format: OfficeFormat) -> OfficeResult<()> {
    let actual = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if actual != format.as_str() {
        return Err(OfficeError::new(
            "format_mismatch",
            "output extension does not match the selected Office format",
            json!({"expected": format.as_str(), "actual": actual}),
        ));
    }
    Ok(())
}

fn required_string<'a>(object: &'a Map<String, Value>, key: &str) -> OfficeResult<&'a str> {
    object
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            OfficeError::new(
                "missing_argument",
                "required string argument is missing",
                json!({"argument": key}),
            )
        })
}

#[cfg(test)]
#[path = "mutation_tests.rs"]
mod tests;
