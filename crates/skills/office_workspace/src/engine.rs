use crate::docx::read_docx;
use crate::error::{OfficeError, OfficeResult};
use crate::model::{
    OfficeArtifactEnvelope, OfficeFormat, OfficeWarning, OperationRecord, PageCursor,
    ValidationEvidence, ENVELOPE_SCHEMA_VERSION,
};
use crate::mutation;
use crate::package::{resolve_input_path, OfficePackage};
use crate::pptx::read_presentation;
use crate::range::{parse_coordinate, CellRange};
use crate::renderer;
use crate::xlsx::read_workbook;
use serde_json::{json, Map, Value};

const DEFAULT_PAGE_LIMIT: usize = 100;
const MAX_PAGE_LIMIT: usize = 1_000;

pub fn execute(args: &Value) -> OfficeResult<Value> {
    let object = args
        .as_object()
        .ok_or_else(|| OfficeError::invalid("args must be an object"))?;
    let action = required_string(object, "action")?;
    match action {
        "office.render_status" | "office.render" => renderer::execute(action, object),
        "office.inspect"
        | "office.validate"
        | "word.read"
        | "spreadsheet.inspect"
        | "spreadsheet.read_range"
        | "presentation.read"
        | "word.find" => inspect_action(action, object),
        "word.preview_create"
        | "word.create"
        | "word.preview_edit"
        | "word.edit"
        | "spreadsheet.preview_create"
        | "spreadsheet.create"
        | "spreadsheet.preview_edit"
        | "spreadsheet.edit"
        | "presentation.preview_create"
        | "presentation.create"
        | "presentation.preview_edit"
        | "presentation.edit" => mutation::execute_mutation(action, object),
        _ => Err(OfficeError::new(
            "unsupported_action",
            "unsupported Office workspace action",
            json!({"action": action}),
        )),
    }
}

fn inspect_action(action: &str, object: &Map<String, Value>) -> OfficeResult<Value> {
    let path = required_string_any(object, &["path", "source_path"])?;
    let path = resolve_input_path(path)?;
    let expected_format = action_format(action);
    let package = OfficePackage::open(&path, expected_format)?;
    let mut envelope = base_envelope(&package);
    match package.format {
        OfficeFormat::Docx => {
            let evidence = read_docx(&package)?;
            envelope.document_blocks = evidence.blocks;
            envelope.tables = evidence.tables;
        }
        OfficeFormat::Xlsx => {
            envelope.workbook = Some(read_workbook(&package)?);
        }
        OfficeFormat::Pptx => {
            envelope.presentation = Some(read_presentation(&package)?);
        }
    }

    if action == "word.find" {
        select_word_matches(&mut envelope, object)?;
    }
    if action == "spreadsheet.read_range" {
        select_spreadsheet_range(&mut envelope, object)?;
    }
    apply_page(&mut envelope, object)?;
    envelope.validation = validate_envelope(&envelope);
    Ok(serde_json::to_value(envelope).map_err(|error| {
        OfficeError::new(
            "serialization_failed",
            format!("cannot serialize Office evidence: {error}"),
            json!({}),
        )
    })?)
}

pub fn inspect_output(path: &std::path::Path, format: OfficeFormat) -> OfficeResult<Value> {
    let action = match format {
        OfficeFormat::Docx => "word.read",
        OfficeFormat::Xlsx => "spreadsheet.inspect",
        OfficeFormat::Pptx => "presentation.read",
    };
    let object = serde_json::Map::from_iter([
        (
            "path".to_string(),
            Value::String(path.display().to_string()),
        ),
        ("limit".to_string(), Value::Number(1_000u64.into())),
    ]);
    inspect_action(action, &object)
}

fn select_word_matches(
    envelope: &mut OfficeArtifactEnvelope,
    object: &Map<String, Value>,
) -> OfficeResult<()> {
    let query = required_string(object, "query")?;
    let matches = envelope
        .document_blocks
        .iter()
        .filter(|block| block.text.contains(query))
        .map(|block| {
            json!({
                "match_id": format!("match:{}", block.id),
                "block_id": block.id,
                "text": block.text,
                "source_part": block.source_part,
                "source_sha256": envelope.source.sha256,
            })
        })
        .collect::<Vec<_>>();
    envelope.metadata["find"] = json!({
        "query": query,
        "matches": matches,
        "match_count": matches.len(),
    });
    Ok(())
}

fn base_envelope(package: &OfficePackage) -> OfficeArtifactEnvelope {
    OfficeArtifactEnvelope {
        schema_version: ENVELOPE_SCHEMA_VERSION,
        format: package.format,
        source: package.source.clone(),
        package: package.evidence.clone(),
        metadata: json!({
            "format": package.format.as_str(),
            "active_content_executed": false,
            "pure_parser": true,
        }),
        document_blocks: Vec::new(),
        tables: Vec::new(),
        workbook: None,
        presentation: None,
        media: package.media.clone(),
        warnings: package.warnings.clone(),
        truncated: false,
        cursor: PageCursor {
            offset: 0,
            limit: 0,
            returned: 0,
            total: 0,
            next_cursor: None,
        },
        operation_log: Vec::<OperationRecord>::new(),
        validation: ValidationEvidence {
            valid: false,
            checks: Vec::new(),
            errors: Vec::new(),
        },
        artifacts: vec![json!({
            "kind": "office_source",
            "path": package.source.path,
            "sha256": package.source.sha256,
            "size_bytes": package.source.size_bytes,
            "format": package.format.as_str(),
        })],
    }
}

fn select_spreadsheet_range(
    envelope: &mut OfficeArtifactEnvelope,
    object: &Map<String, Value>,
) -> OfficeResult<()> {
    let sheet_name = required_string_any(object, &["sheet", "sheet_name"])?;
    let range = required_string(object, "range")?;
    let range = CellRange::parse(range)?;
    let Some(workbook) = envelope.workbook.as_mut() else {
        return Err(OfficeError::new(
            "format_mismatch",
            "spreadsheet range reads require an XLSX workbook",
            json!({}),
        ));
    };
    let Some(sheet) = workbook
        .sheets
        .iter_mut()
        .find(|sheet| sheet.name == sheet_name)
    else {
        return Err(OfficeError::new(
            "worksheet_not_found",
            "requested worksheet does not exist",
            json!({
                "sheet": sheet_name,
                "available_sheets": workbook.sheets.iter().map(|sheet| &sheet.name).collect::<Vec<_>>()
            }),
        ));
    };
    sheet.cells.retain(|cell| {
        parse_coordinate(&cell.reference)
            .map(|coordinate| range.contains(coordinate))
            .unwrap_or(false)
    });
    workbook.sheets.retain(|item| item.name == sheet_name);
    Ok(())
}

fn apply_page(
    envelope: &mut OfficeArtifactEnvelope,
    object: &Map<String, Value>,
) -> OfficeResult<()> {
    let requested_offset = object
        .get("offset")
        .and_then(Value::as_u64)
        .map(|value| value as usize);
    let cursor_offset = object
        .get("cursor")
        .and_then(Value::as_str)
        .map(|cursor| parse_cursor(cursor, &envelope.source.sha256))
        .transpose()?;
    let offset = requested_offset.or(cursor_offset).unwrap_or(0);
    let limit = object
        .get("limit")
        .and_then(Value::as_u64)
        .map(|value| value as usize)
        .unwrap_or(DEFAULT_PAGE_LIMIT)
        .clamp(1, MAX_PAGE_LIMIT);

    let total = match envelope.format {
        OfficeFormat::Docx => envelope.document_blocks.len(),
        OfficeFormat::Xlsx => envelope
            .workbook
            .as_ref()
            .map(|workbook| workbook.sheets.iter().map(|sheet| sheet.cells.len()).sum())
            .unwrap_or(0),
        OfficeFormat::Pptx => envelope
            .presentation
            .as_ref()
            .map(|presentation| presentation.slides.len())
            .unwrap_or(0),
    };
    if offset > total {
        return Err(OfficeError::new(
            "cursor_out_of_range",
            "Office evidence cursor is beyond the available result",
            json!({"offset": offset, "total": total}),
        ));
    }
    let end = offset.saturating_add(limit).min(total);
    match envelope.format {
        OfficeFormat::Docx => {
            envelope.document_blocks = envelope.document_blocks[offset..end].to_vec();
        }
        OfficeFormat::Xlsx => {
            retain_cell_page(envelope, offset, end);
        }
        OfficeFormat::Pptx => {
            if let Some(presentation) = envelope.presentation.as_mut() {
                presentation.slides = presentation.slides[offset..end].to_vec();
            }
        }
    }
    let returned = end.saturating_sub(offset);
    let next_cursor = (end < total).then(|| format_cursor(end, &envelope.source.sha256));
    envelope.truncated = next_cursor.is_some();
    envelope.cursor = PageCursor {
        offset,
        limit,
        returned,
        total,
        next_cursor,
    };
    if envelope.truncated {
        envelope.warnings.push(OfficeWarning {
            code: "evidence_page_truncated".to_string(),
            object_ref: None,
            details: json!({"offset": offset, "returned": returned, "total": total}),
            untrusted: false,
        });
    }
    Ok(())
}

fn retain_cell_page(envelope: &mut OfficeArtifactEnvelope, offset: usize, end: usize) {
    let Some(workbook) = envelope.workbook.as_mut() else {
        return;
    };
    let mut current = 0usize;
    for sheet in &mut workbook.sheets {
        let sheet_start = current;
        let sheet_end = current + sheet.cells.len();
        let start_in_sheet = offset.saturating_sub(sheet_start).min(sheet.cells.len());
        let end_in_sheet = end.saturating_sub(sheet_start).min(sheet.cells.len());
        if end <= sheet_start || offset >= sheet_end {
            sheet.cells.clear();
        } else {
            sheet.cells = sheet.cells[start_in_sheet..end_in_sheet].to_vec();
        }
        current = sheet_end;
    }
}

fn validate_envelope(envelope: &OfficeArtifactEnvelope) -> ValidationEvidence {
    let mut checks = vec![
        "zip_member_paths_canonical".to_string(),
        "package_limits_enforced".to_string(),
        "required_parts_present".to_string(),
        "active_content_not_executed".to_string(),
    ];
    if envelope.package.external_relationships.is_empty() {
        checks.push("no_external_relationships".to_string());
    }
    if envelope.package.embedded_members.is_empty() {
        checks.push("no_embedded_objects".to_string());
    }
    ValidationEvidence {
        valid: true,
        checks,
        errors: Vec::new(),
    }
}

fn action_format(action: &str) -> Option<OfficeFormat> {
    if action.starts_with("word.") {
        Some(OfficeFormat::Docx)
    } else if action.starts_with("spreadsheet.") {
        Some(OfficeFormat::Xlsx)
    } else if action.starts_with("presentation.") {
        Some(OfficeFormat::Pptx)
    } else {
        None
    }
}

fn format_cursor(offset: usize, hash: &str) -> String {
    format!("office-v1:{offset}:{}", &hash[..hash.len().min(16)])
}

fn parse_cursor(cursor: &str, hash: &str) -> OfficeResult<usize> {
    let mut parts = cursor.split(':');
    let version = parts.next();
    let offset = parts.next().and_then(|value| value.parse::<usize>().ok());
    let hash_prefix = parts.next();
    if version != Some("office-v1")
        || parts.next().is_some()
        || offset.is_none()
        || hash_prefix.is_none()
        || !hash.starts_with(hash_prefix.unwrap_or_default())
    {
        return Err(OfficeError::new(
            "invalid_cursor",
            "Office evidence cursor is invalid for this source revision",
            json!({"cursor": cursor}),
        ));
    }
    Ok(offset.unwrap_or_default())
}

fn required_string<'a>(object: &'a Map<String, Value>, key: &str) -> OfficeResult<&'a str> {
    object
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            OfficeError::new(
                "missing_argument",
                "required string argument is missing",
                json!({"argument": key}),
            )
        })
}

fn required_string_any<'a>(object: &'a Map<String, Value>, keys: &[&str]) -> OfficeResult<&'a str> {
    keys.iter()
        .find_map(|key| {
            object
                .get(*key)
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
        })
        .ok_or_else(|| {
            OfficeError::new(
                "missing_argument",
                "required string argument is missing",
                json!({"arguments": keys}),
            )
        })
}

#[cfg(test)]
#[path = "engine_tests.rs"]
mod tests;
