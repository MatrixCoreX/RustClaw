use crate::error::{OfficeError, OfficeResult};
use crate::operations::NormalizedOperation;
use quick_xml::escape::escape;
use serde_json::{json, Value};

pub fn supports(kind: &str) -> bool {
    matches!(
        kind,
        "insert_block_before"
            | "insert_block_after"
            | "move_block"
            | "replace_run"
            | "insert_run"
            | "delete_run"
            | "move_run"
            | "table_add_row"
            | "table_delete_row"
            | "table_add_column"
            | "table_delete_column"
    )
}

pub fn apply(document: &mut String, operation: &NormalizedOperation) -> OfficeResult<Vec<String>> {
    match operation.kind.as_str() {
        "insert_block_before" | "insert_block_after" => {
            insert_block(document, operation)?;
        }
        "move_block" => move_block(document, operation)?,
        "replace_run" => edit_run(document, operation, RunEdit::Replace)?,
        "insert_run" => edit_run(document, operation, RunEdit::Insert)?,
        "delete_run" => edit_run(document, operation, RunEdit::Delete)?,
        "move_run" => edit_run(document, operation, RunEdit::Move)?,
        "table_add_row" => table_add_row(document, operation)?,
        "table_delete_row" => table_delete_row(document, operation)?,
        "table_add_column" => table_add_column(document, operation)?,
        "table_delete_column" => table_delete_column(document, operation)?,
        _ => {
            return Err(OfficeError::unsupported(
                "DOCX structure operation is not implemented",
                json!({"operation_id": operation.id, "op": operation.kind}),
            ))
        }
    }
    Ok(operation.object_refs())
}

fn insert_block(document: &mut String, operation: &NormalizedOperation) -> OfficeResult<()> {
    let block_id = operation.string("block_id")?;
    let index = document_paragraph_index(block_id)?;
    let range = nth_element_range(document, "w:p", index)?;
    let paragraph = paragraph_xml(
        operation.string("text")?,
        operation.optional_string("style"),
    );
    let insertion = if operation.kind == "insert_block_before" {
        range.0
    } else {
        range.1
    };
    *document = format!(
        "{}{}{}",
        &document[..insertion],
        paragraph,
        &document[insertion..]
    );
    Ok(())
}

fn move_block(document: &mut String, operation: &NormalizedOperation) -> OfficeResult<()> {
    let source_index = document_paragraph_index(operation.string("block_id")?)?;
    let target_index = document_paragraph_index(operation.string("target_block_id")?)?;
    if source_index == target_index {
        return Err(OfficeError::invalid(
            "move_block source and target must be different",
        ));
    }
    let position = operation.optional_string("position").unwrap_or("before");
    if !matches!(position, "before" | "after") {
        return Err(OfficeError::invalid(
            "move_block position must be before or after",
        ));
    }
    let source_range = nth_element_range(document, "w:p", source_index)?;
    let paragraph = document[source_range.0..source_range.1].to_string();
    let mut without = format!(
        "{}{}",
        &document[..source_range.0],
        &document[source_range.1..]
    );
    let adjusted_target = if source_index < target_index {
        target_index - 1
    } else {
        target_index
    };
    let target_range = nth_element_range(&without, "w:p", adjusted_target)?;
    let insertion = if position == "before" {
        target_range.0
    } else {
        target_range.1
    };
    without.insert_str(insertion, &paragraph);
    *document = without;
    Ok(())
}

enum RunEdit {
    Replace,
    Insert,
    Delete,
    Move,
}

fn edit_run(
    document: &mut String,
    operation: &NormalizedOperation,
    mode: RunEdit,
) -> OfficeResult<()> {
    let paragraph_index = document_paragraph_index(operation.string("block_id")?)?;
    let paragraph_range = nth_element_range(document, "w:p", paragraph_index)?;
    let paragraph = &document[paragraph_range.0..paragraph_range.1];
    let updated = match mode {
        RunEdit::Replace => {
            let run = operation.usize("run")?;
            let range = nth_element_range(paragraph, "w:r", run)?;
            let selected = &paragraph[range.0..range.1];
            if let Some(expected) = operation.optional_string("expected_text") {
                let actual = text_nodes(selected).join("");
                if actual != expected {
                    return Err(OfficeError::new(
                        "source_conflict",
                        "selected run text does not match the expected revision",
                        json!({"expected_text": expected, "actual_text": actual}),
                    ));
                }
            }
            let selected = replace_text_nodes(selected, operation.string("text")?)?;
            format!(
                "{}{}{}",
                &paragraph[..range.0],
                selected,
                &paragraph[range.1..]
            )
        }
        RunEdit::Insert => {
            let index = operation.usize("index")?;
            let runs = element_ranges(paragraph, "w:r")?;
            if index > runs.len() {
                return Err(index_error("run", index, runs.len()));
            }
            let insertion = if index == runs.len() {
                paragraph.rfind("</w:p>").ok_or_else(|| malformed("w:p"))?
            } else {
                runs[index].0
            };
            let run = run_xml(
                operation.string("text")?,
                operation.optional_string("style"),
            );
            format!(
                "{}{}{}",
                &paragraph[..insertion],
                run,
                &paragraph[insertion..]
            )
        }
        RunEdit::Delete => {
            let run = operation.usize("run")?;
            let range = nth_element_range(paragraph, "w:r", run)?;
            format!("{}{}", &paragraph[..range.0], &paragraph[range.1..])
        }
        RunEdit::Move => {
            let run = operation.usize("run")?;
            let target_index = operation.usize("target_index")?;
            let ranges = element_ranges(paragraph, "w:r")?;
            if run == 0 || run > ranges.len() {
                return Err(index_error("run", run, ranges.len()));
            }
            if target_index >= ranges.len() {
                return Err(index_error("target_index", target_index, ranges.len()));
            }
            let selected = ranges[run - 1];
            let run_xml = paragraph[selected.0..selected.1].to_string();
            let mut without = format!("{}{}", &paragraph[..selected.0], &paragraph[selected.1..]);
            let remaining = element_ranges(&without, "w:r")?;
            let adjusted = if run - 1 < target_index {
                target_index.saturating_sub(1)
            } else {
                target_index
            };
            let insertion = remaining
                .get(adjusted)
                .map(|range| range.0)
                .unwrap_or_else(|| without.rfind("</w:p>").unwrap_or(without.len()));
            without.insert_str(insertion, &run_xml);
            without
        }
    };
    *document = format!(
        "{}{}{}",
        &document[..paragraph_range.0],
        updated,
        &document[paragraph_range.1..]
    );
    Ok(())
}

fn table_add_row(document: &mut String, operation: &NormalizedOperation) -> OfficeResult<()> {
    let table_index = document_table_index(operation.string("table_id")?)?;
    let table_range = nth_element_range(document, "w:tbl", table_index)?;
    let table = &document[table_range.0..table_range.1];
    let rows = element_ranges(table, "w:tr")?;
    let index = operation.optional_usize("index").unwrap_or(rows.len());
    if index > rows.len() {
        return Err(index_error("row", index, rows.len()));
    }
    let values = string_values(operation.value("values"))?;
    let row = table_row_xml(&values);
    let insertion = rows
        .get(index)
        .map(|range| range.0)
        .unwrap_or_else(|| table.rfind("</w:tbl>").unwrap_or(table.len()));
    replace_table(
        document,
        table_range,
        format!("{}{}{}", &table[..insertion], row, &table[insertion..]),
    );
    Ok(())
}

fn table_delete_row(document: &mut String, operation: &NormalizedOperation) -> OfficeResult<()> {
    let table_index = document_table_index(operation.string("table_id")?)?;
    let table_range = nth_element_range(document, "w:tbl", table_index)?;
    let table = &document[table_range.0..table_range.1];
    let rows = element_ranges(table, "w:tr")?;
    if rows.len() <= 1 {
        return Err(OfficeError::invalid(
            "table_delete_row cannot remove the final row",
        ));
    }
    let index = operation.usize("row")?;
    let selected = rows
        .get(index)
        .copied()
        .ok_or_else(|| index_error("row", index, rows.len()))?;
    replace_table(
        document,
        table_range,
        format!("{}{}", &table[..selected.0], &table[selected.1..]),
    );
    Ok(())
}

fn table_add_column(document: &mut String, operation: &NormalizedOperation) -> OfficeResult<()> {
    let table_index = document_table_index(operation.string("table_id")?)?;
    let table_range = nth_element_range(document, "w:tbl", table_index)?;
    let mut table = document[table_range.0..table_range.1].to_string();
    let values = string_values(operation.value("values"))?;
    let rows = element_ranges(&table, "w:tr")?;
    for (row_index, row_range) in rows.into_iter().enumerate().rev() {
        let row = &table[row_range.0..row_range.1];
        let cells = element_ranges(row, "w:tc")?;
        let index = operation.optional_usize("column").unwrap_or(cells.len());
        if index > cells.len() {
            return Err(index_error("column", index, cells.len()));
        }
        let insertion = cells
            .get(index)
            .map(|range| range.0)
            .unwrap_or_else(|| row.rfind("</w:tr>").unwrap_or(row.len()));
        let value = values.get(row_index).map(String::as_str).unwrap_or("");
        let updated = format!(
            "{}{}{}",
            &row[..insertion],
            table_cell_xml(value),
            &row[insertion..]
        );
        table = format!(
            "{}{}{}",
            &table[..row_range.0],
            updated,
            &table[row_range.1..]
        );
    }
    replace_table(document, table_range, table);
    Ok(())
}

fn table_delete_column(document: &mut String, operation: &NormalizedOperation) -> OfficeResult<()> {
    let table_index = document_table_index(operation.string("table_id")?)?;
    let table_range = nth_element_range(document, "w:tbl", table_index)?;
    let mut table = document[table_range.0..table_range.1].to_string();
    let column = operation.usize("column")?;
    let rows = element_ranges(&table, "w:tr")?;
    for row_range in rows.into_iter().rev() {
        let row = &table[row_range.0..row_range.1];
        let cells = element_ranges(row, "w:tc")?;
        if cells.len() <= 1 {
            return Err(OfficeError::invalid(
                "table_delete_column cannot remove the final column",
            ));
        }
        let selected = cells
            .get(column)
            .copied()
            .ok_or_else(|| index_error("column", column, cells.len()))?;
        let updated = format!("{}{}", &row[..selected.0], &row[selected.1..]);
        table = format!(
            "{}{}{}",
            &table[..row_range.0],
            updated,
            &table[row_range.1..]
        );
    }
    replace_table(document, table_range, table);
    Ok(())
}

fn replace_table(document: &mut String, range: (usize, usize), table: String) {
    *document = format!("{}{}{}", &document[..range.0], table, &document[range.1..]);
}

fn document_paragraph_index(id: &str) -> OfficeResult<usize> {
    object_index(id, "word_document", "paragraph")
}

fn document_table_index(id: &str) -> OfficeResult<usize> {
    object_index(id, "word_document", "table")
}

fn object_index(id: &str, part: &str, kind: &str) -> OfficeResult<usize> {
    let prefix = format!("{part}_{kind}_");
    id.strip_prefix(&prefix)
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .ok_or_else(|| {
            OfficeError::new(
                "invalid_selector",
                "object identifier does not address the required DOCX part and kind",
                json!({"object_id": id, "required_part": part, "required_kind": kind}),
            )
        })
}

fn element_ranges(source: &str, tag: &str) -> OfficeResult<Vec<(usize, usize)>> {
    let mut output = Vec::new();
    let mut index = 1usize;
    loop {
        match nth_element_range(source, tag, index) {
            Ok(range) => {
                output.push(range);
                index += 1;
            }
            Err(error) if error.code == "object_not_found" => break,
            Err(error) => return Err(error),
        }
    }
    Ok(output)
}

fn nth_element_range(source: &str, tag: &str, index: usize) -> OfficeResult<(usize, usize)> {
    if index == 0 {
        return Err(OfficeError::invalid("object identifiers are one-based"));
    }
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut cursor = 0usize;
    let mut current = 0usize;
    while let Some(relative) = source[cursor..].find(&open) {
        let start = cursor + relative;
        let boundary = source.as_bytes().get(start + open.len()).copied();
        if !matches!(boundary, Some(b'>') | Some(b' ') | Some(b'/')) {
            cursor = start + open.len();
            continue;
        }
        current += 1;
        let opening_end = source[start..]
            .find('>')
            .map(|relative| start + relative + 1)
            .ok_or_else(|| malformed(tag))?;
        let end = if source[start..opening_end].trim_end().ends_with("/>") {
            opening_end
        } else {
            source[opening_end..]
                .find(&close)
                .map(|relative| opening_end + relative + close.len())
                .ok_or_else(|| malformed(tag))?
        };
        if current == index {
            return Ok((start, end));
        }
        cursor = end;
    }
    Err(OfficeError::new(
        "object_not_found",
        "selected OOXML object does not exist",
        json!({"tag": tag, "index": index}),
    ))
}

fn replace_text_nodes(element: &str, replacement: &str) -> OfficeResult<String> {
    let mut output = String::new();
    let mut cursor = 0usize;
    let mut wrote = false;
    while let Some(relative) = element[cursor..].find("<w:t") {
        let start = cursor + relative;
        let opening_end = element[start..]
            .find('>')
            .map(|relative| start + relative + 1)
            .ok_or_else(|| malformed("w:t"))?;
        let closing = element[opening_end..]
            .find("</w:t>")
            .map(|relative| opening_end + relative)
            .ok_or_else(|| malformed("w:t"))?;
        output.push_str(&element[cursor..opening_end]);
        if !wrote {
            output.push_str(&xml(replacement));
            wrote = true;
        }
        output.push_str("</w:t>");
        cursor = closing + "</w:t>".len();
    }
    output.push_str(&element[cursor..]);
    if wrote {
        Ok(output)
    } else {
        Err(OfficeError::unsupported(
            "selected run has no editable text node",
            json!({}),
        ))
    }
}

fn text_nodes(element: &str) -> Vec<String> {
    let mut output = Vec::new();
    let mut cursor = 0usize;
    while let Some(relative) = element[cursor..].find("<w:t") {
        let start = cursor + relative;
        let Some(opening_end) = element[start..]
            .find('>')
            .map(|relative| start + relative + 1)
        else {
            break;
        };
        let Some(closing) = element[opening_end..]
            .find("</w:t>")
            .map(|relative| opening_end + relative)
        else {
            break;
        };
        output.push(element[opening_end..closing].to_string());
        cursor = closing + "</w:t>".len();
    }
    output
}

fn paragraph_xml(text: &str, style: Option<&str>) -> String {
    let properties = style
        .map(|style| format!("<w:pPr><w:pStyle w:val=\"{}\"/></w:pPr>", xml(style)))
        .unwrap_or_default();
    format!(
        "<w:p>{properties}<w:r><w:t xml:space=\"preserve\">{}</w:t></w:r></w:p>",
        xml(text)
    )
}

fn run_xml(text: &str, style: Option<&str>) -> String {
    let properties = style
        .map(|style| format!("<w:rPr><w:rStyle w:val=\"{}\"/></w:rPr>", xml(style)))
        .unwrap_or_default();
    format!(
        "<w:r>{properties}<w:t xml:space=\"preserve\">{}</w:t></w:r>",
        xml(text)
    )
}

fn table_row_xml(values: &[String]) -> String {
    format!(
        "<w:tr>{}</w:tr>",
        values
            .iter()
            .map(|value| table_cell_xml(value))
            .collect::<String>()
    )
}

fn table_cell_xml(value: &str) -> String {
    format!(
        "<w:tc><w:tcPr/><w:p><w:r><w:t xml:space=\"preserve\">{}</w:t></w:r></w:p></w:tc>",
        xml(value)
    )
}

fn string_values(value: Option<&Value>) -> OfficeResult<Vec<String>> {
    value
        .and_then(Value::as_array)
        .ok_or_else(|| OfficeError::invalid("operation requires values as an array"))
        .map(|values| values.iter().map(scalar_text).collect())
}

fn scalar_text(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(value) => value.clone(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        other => other.to_string(),
    }
}

fn index_error(kind: &str, index: usize, count: usize) -> OfficeError {
    OfficeError::new(
        "object_not_found",
        "selected object index is outside the current revision",
        json!({"kind": kind, "index": index, "count": count}),
    )
}

fn malformed(element: &str) -> OfficeError {
    OfficeError::new(
        "malformed_xml",
        "selected DOCX XML element is malformed",
        json!({"element": element}),
    )
}

fn xml(value: &str) -> String {
    escape(value).into_owned()
}

#[cfg(test)]
#[path = "docx_structure_edit_tests.rs"]
mod tests;
