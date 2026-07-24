use crate::error::{OfficeError, OfficeResult};
use crate::model::{OfficeFormat, OperationRecord};
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;

#[derive(Clone, Debug)]
pub struct NormalizedOperation {
    pub id: String,
    pub kind: String,
    pub fields: Map<String, Value>,
}

impl NormalizedOperation {
    pub fn string(&self, key: &str) -> OfficeResult<&str> {
        self.fields
            .get(key)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                OfficeError::new(
                    "invalid_operation",
                    "operation requires a non-empty string field",
                    json!({"operation_id": self.id, "op": self.kind, "field": key}),
                )
            })
    }

    pub fn optional_string(&self, key: &str) -> Option<&str> {
        self.fields
            .get(key)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
    }

    pub fn usize(&self, key: &str) -> OfficeResult<usize> {
        self.fields
            .get(key)
            .and_then(Value::as_u64)
            .map(|value| value as usize)
            .ok_or_else(|| {
                OfficeError::new(
                    "invalid_operation",
                    "operation requires a non-negative integer field",
                    json!({"operation_id": self.id, "op": self.kind, "field": key}),
                )
            })
    }

    pub fn optional_usize(&self, key: &str) -> Option<usize> {
        self.fields
            .get(key)
            .and_then(Value::as_u64)
            .map(|value| value as usize)
    }

    pub fn bool(&self, key: &str) -> Option<bool> {
        self.fields.get(key).and_then(Value::as_bool)
    }

    pub fn value(&self, key: &str) -> Option<&Value> {
        self.fields.get(key)
    }

    pub fn object_refs(&self) -> Vec<String> {
        [
            "block_id", "table_id", "media_id", "sheet", "cell", "range", "slide_id", "shape_id",
        ]
        .iter()
        .filter_map(|key| self.optional_string(key).map(ToOwned::to_owned))
        .collect()
    }

    pub fn record(&self, status: &str) -> OperationRecord {
        OperationRecord {
            id: self.id.clone(),
            operation: self.kind.clone(),
            object_refs: self.object_refs(),
            status: status.to_string(),
        }
    }

    pub fn as_value(&self) -> Value {
        let mut fields = self.fields.clone();
        fields.insert("id".to_string(), Value::String(self.id.clone()));
        fields.insert("op".to_string(), Value::String(self.kind.clone()));
        Value::Object(fields)
    }
}

pub fn normalize_operations(
    value: Option<&Value>,
    format: OfficeFormat,
    editing: bool,
) -> OfficeResult<Vec<NormalizedOperation>> {
    let operations = value
        .and_then(Value::as_array)
        .ok_or_else(|| OfficeError::invalid("operations must be an array"))?;
    if operations.is_empty() {
        return Err(OfficeError::invalid(
            "operations must contain at least one operation",
        ));
    }
    if operations.len() > 500 {
        return Err(OfficeError::new(
            "operation_limit_exceeded",
            "Office mutation batch contains too many operations",
            json!({"count": operations.len(), "limit": 500}),
        ));
    }
    let allowed = allowed_operations(format, editing);
    operations
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let fields = value.as_object().ok_or_else(|| {
                OfficeError::new(
                    "invalid_operation",
                    "each operation must be an object",
                    json!({"operation_index": index}),
                )
            })?;
            let kind = fields
                .get("op")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    OfficeError::new(
                        "invalid_operation",
                        "operation requires an op token",
                        json!({"operation_index": index}),
                    )
                })?
                .to_string();
            if !allowed.contains(kind.as_str()) {
                return Err(OfficeError::unsupported(
                    "operation is not supported for this Office format and mutation mode",
                    json!({
                        "operation_index": index,
                        "op": kind,
                        "format": format.as_str(),
                        "editing": editing
                    }),
                ));
            }
            let id = fields
                .get("id")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| format!("op_{}", index + 1));
            let mut fields = fields.clone();
            fields.remove("id");
            fields.remove("op");
            Ok(NormalizedOperation { id, kind, fields })
        })
        .collect()
}

fn allowed_operations(format: OfficeFormat, editing: bool) -> BTreeSet<&'static str> {
    let values: &[&str] = match (format, editing) {
        (OfficeFormat::Docx, false) => &[
            "set_properties",
            "set_section",
            "set_header",
            "set_footer",
            "add_heading",
            "add_paragraph",
            "add_list_item",
            "add_table",
            "add_image",
            "add_hyperlink",
            "add_bookmark",
            "add_footnote",
            "add_endnote",
            "add_comment",
            "add_page_break",
            "add_section_break",
        ],
        (OfficeFormat::Docx, true) => &[
            "set_properties",
            "set_section",
            "set_header",
            "set_footer",
            "add_heading",
            "add_paragraph",
            "add_list_item",
            "add_table",
            "add_image",
            "add_hyperlink",
            "add_bookmark",
            "add_footnote",
            "add_endnote",
            "add_comment",
            "add_page_break",
            "add_section_break",
            "replace_block",
            "delete_block",
            "set_block_style",
            "replace_match",
            "table_set_cell",
            "replace_image",
        ],
        (OfficeFormat::Xlsx, false) => &[
            "add_sheet",
            "set_cell",
            "set_range",
            "clear_cell",
            "merge_cells",
            "freeze_panes",
            "set_auto_filter",
            "set_column_width",
            "set_row_height",
            "add_table",
            "add_chart",
            "add_comment",
            "add_hyperlink",
            "add_image",
            "add_named_range",
            "add_data_validation",
            "add_conditional_format",
        ],
        (OfficeFormat::Xlsx, true) => &[
            "add_sheet",
            "copy_sheet",
            "rename_sheet",
            "reorder_sheet",
            "hide_sheet",
            "delete_sheet",
            "set_cell",
            "set_range",
            "clear_cell",
            "move_range",
            "fill_range",
            "merge_cells",
            "unmerge_cells",
            "freeze_panes",
            "set_auto_filter",
            "set_column_width",
            "set_row_height",
            "add_table",
            "add_chart",
            "add_comment",
            "add_hyperlink",
            "add_image",
            "add_named_range",
            "add_data_validation",
            "add_conditional_format",
        ],
        (OfficeFormat::Pptx, false) => &[
            "set_properties",
            "add_slide",
            "add_text",
            "add_notes",
            "add_image",
            "add_table",
            "add_chart",
            "add_shape",
            "add_link",
            "set_transition",
        ],
        (OfficeFormat::Pptx, true) => &[
            "set_properties",
            "add_slide",
            "duplicate_slide",
            "move_slide",
            "hide_slide",
            "delete_slide",
            "replace_slide_text",
            "set_slide_layout",
            "add_text",
            "add_notes",
            "replace_image",
            "add_image",
            "add_table",
            "add_chart",
            "add_shape",
            "add_link",
            "set_transition",
        ],
    };
    values.iter().copied().collect()
}

#[cfg(test)]
#[path = "operations_tests.rs"]
mod tests;
