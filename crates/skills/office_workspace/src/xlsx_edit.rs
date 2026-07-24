use crate::error::{OfficeError, OfficeResult};
use crate::operations::NormalizedOperation;
use crate::package::OfficePackage;
use crate::range::{format_coordinate, parse_coordinate, CellCoordinate, CellRange};
use crate::xlsx_write::{
    add_merge, cell_from_operation, cell_xml, cells_for_range, find_cell_range, insert_before,
    malformed_xml, member_text, remove_cell, remove_merge, rename_sheet, replace_or_add_attribute,
    set_auto_filter, set_freeze, set_sheet_hidden, upsert_cell, validate_sheet_name, xml,
    XlsxWriteResult,
};
use crate::xml::{attr_value, attr_value_qualified, local_name, relationship_map};
use quick_xml::events::Event;
use quick_xml::Reader;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use uuid::Uuid;

#[derive(Clone, Debug)]
struct SheetEntry {
    name: String,
    id: u32,
    relationship_id: String,
    path: String,
}

pub fn edit_xlsx(
    package: &OfficePackage,
    operations: &[NormalizedOperation],
) -> OfficeResult<XlsxWriteResult> {
    let mut members = package.members.clone();
    let mut changed_refs = Vec::new();
    for operation in operations {
        match operation.kind.as_str() {
            "add_sheet" => add_sheet(&mut members, operation.string("name")?, None)?,
            "copy_sheet" => copy_sheet(&mut members, operation)?,
            "rename_sheet" => {
                let sheet = operation.string("sheet")?;
                let new_name = operation.string("new_name")?;
                validate_sheet_name(new_name)?;
                reject_duplicate_sheet_name(&members, new_name, Some(sheet))?;
                let workbook = member_text(&members, "xl/workbook.xml")?.to_string();
                members.insert(
                    "xl/workbook.xml".into(),
                    rename_sheet(&workbook, sheet, new_name)?.into_bytes(),
                );
            }
            "reorder_sheet" => reorder_sheet(&mut members, operation)?,
            "hide_sheet" => {
                let sheet = operation.string("sheet")?;
                let hidden = operation.bool("hidden").unwrap_or(true);
                let workbook = member_text(&members, "xl/workbook.xml")?.to_string();
                members.insert(
                    "xl/workbook.xml".into(),
                    set_sheet_hidden(&workbook, sheet, hidden)?.into_bytes(),
                );
            }
            "delete_sheet" => delete_sheet(&mut members, operation.string("sheet")?)?,
            "set_cell" => edit_cell(&mut members, operation, false, &mut changed_refs)?,
            "clear_cell" => edit_cell(&mut members, operation, true, &mut changed_refs)?,
            "set_range" => set_range(&mut members, operation, &mut changed_refs)?,
            "fill_range" => fill_range(&mut members, operation, &mut changed_refs)?,
            "move_range" => move_range(&mut members, operation, &mut changed_refs)?,
            "merge_cells" | "unmerge_cells" => {
                edit_merge(&mut members, operation, &mut changed_refs)?
            }
            "freeze_panes" => edit_freeze(&mut members, operation, &mut changed_refs)?,
            "set_auto_filter" => edit_auto_filter(&mut members, operation, &mut changed_refs)?,
            "set_column_width" => edit_column_width(&mut members, operation, &mut changed_refs)?,
            "set_row_height" => edit_row_height(&mut members, operation, &mut changed_refs)?,
            "add_named_range" => add_named_range(&mut members, operation)?,
            "add_data_validation" => add_data_validation(&mut members, operation)?,
            "add_conditional_format" => add_conditional_format(&mut members, operation)?,
            "add_table" => add_table(&mut members, operation)?,
            "add_chart" => add_chart(&mut members, operation)?,
            "add_comment" => add_comment(&mut members, operation)?,
            "add_hyperlink" => add_hyperlink(&mut members, operation)?,
            "add_image" => add_image(&mut members, operation)?,
            _ => {
                return Err(OfficeError::unsupported(
                    "XLSX edit operation is not implemented without potential package loss",
                    json!({"operation_id": operation.id, "op": operation.kind}),
                ))
            }
        }
        if matches!(
            operation.kind.as_str(),
            "add_sheet"
                | "copy_sheet"
                | "rename_sheet"
                | "reorder_sheet"
                | "hide_sheet"
                | "delete_sheet"
                | "add_named_range"
                | "add_data_validation"
                | "add_conditional_format"
                | "add_table"
                | "add_chart"
                | "add_comment"
                | "add_hyperlink"
                | "add_image"
        ) {
            changed_refs.extend(operation.object_refs());
        }
    }
    Ok(XlsxWriteResult {
        members,
        changed_refs,
        preservation: vec![
            "unknown_package_parts_preserved".to_string(),
            "untouched_worksheets_preserved".to_string(),
        ],
    })
}

fn edit_cell(
    members: &mut BTreeMap<String, Vec<u8>>,
    operation: &NormalizedOperation,
    clear: bool,
    changed_refs: &mut Vec<String>,
) -> OfficeResult<()> {
    let sheet = operation.string("sheet")?;
    let reference = operation.string("cell")?;
    parse_coordinate(reference)?;
    let path = require_sheet_path(members, sheet)?;
    let source = member_text(members, &path)?.to_string();
    let updated = if clear {
        remove_cell(&source, reference)?
    } else {
        let cell = cell_from_operation(operation)?;
        upsert_cell(&source, reference, &cell_xml(reference, &cell))?
    };
    members.insert(path, updated.into_bytes());
    changed_refs.push(format!("{sheet}!{reference}"));
    Ok(())
}

fn set_range(
    members: &mut BTreeMap<String, Vec<u8>>,
    operation: &NormalizedOperation,
    changed_refs: &mut Vec<String>,
) -> OfficeResult<()> {
    let sheet = operation.string("sheet")?;
    let range = CellRange::parse(operation.string("range")?)?;
    let path = require_sheet_path(members, sheet)?;
    let mut source = member_text(members, &path)?.to_string();
    for (coordinate, value) in cells_for_range(operation, range)? {
        let reference = format_coordinate(coordinate);
        source = upsert_cell(&source, &reference, &cell_xml(&reference, &value))?;
        changed_refs.push(format!("{sheet}!{reference}"));
    }
    members.insert(path, source.into_bytes());
    Ok(())
}

fn fill_range(
    members: &mut BTreeMap<String, Vec<u8>>,
    operation: &NormalizedOperation,
    changed_refs: &mut Vec<String>,
) -> OfficeResult<()> {
    let sheet = operation.string("sheet")?;
    let range = CellRange::parse(operation.string("range")?)?;
    let path = require_sheet_path(members, sheet)?;
    let cell = cell_from_operation(operation)?;
    let mut source = member_text(members, &path)?.to_string();
    for row in range.start.row..=range.end.row {
        for column in range.start.column..=range.end.column {
            let reference = format_coordinate(CellCoordinate { row, column });
            source = upsert_cell(&source, &reference, &cell_xml(&reference, &cell))?;
            changed_refs.push(format!("{sheet}!{reference}"));
        }
    }
    members.insert(path, source.into_bytes());
    Ok(())
}

fn move_range(
    members: &mut BTreeMap<String, Vec<u8>>,
    operation: &NormalizedOperation,
    changed_refs: &mut Vec<String>,
) -> OfficeResult<()> {
    let source_sheet = operation.string("sheet")?;
    let target_sheet = operation
        .optional_string("target_sheet")
        .unwrap_or(source_sheet);
    let range = CellRange::parse(operation.string("range")?)?;
    let target = parse_coordinate(operation.string("target_cell")?)?;
    let row_span = range.end.row - range.start.row;
    let column_span = range.end.column - range.start.column;
    parse_coordinate(&format_coordinate(CellCoordinate {
        row: target
            .row
            .checked_add(row_span)
            .ok_or_else(|| OfficeError::invalid("moved range exceeds worksheet row limit"))?,
        column: target
            .column
            .checked_add(column_span)
            .ok_or_else(|| OfficeError::invalid("moved range exceeds worksheet column limit"))?,
    }))?;
    let source_path = require_sheet_path(members, source_sheet)?;
    let target_path = require_sheet_path(members, target_sheet)?;
    let mut source_xml = member_text(members, &source_path)?.to_string();
    let mut target_xml = if source_path == target_path {
        source_xml.clone()
    } else {
        member_text(members, &target_path)?.to_string()
    };
    let mut captured = Vec::new();
    for row in range.start.row..=range.end.row {
        for column in range.start.column..=range.end.column {
            let source_ref = format_coordinate(CellCoordinate { row, column });
            let destination_ref = format_coordinate(CellCoordinate {
                row: target.row + (row - range.start.row),
                column: target.column + (column - range.start.column),
            });
            if let Some((start, end)) = find_cell_range(&source_xml, &source_ref)? {
                let rewritten = rewrite_cell_reference(&source_xml[start..end], &destination_ref)?;
                captured.push((source_ref, destination_ref, rewritten));
            }
        }
    }
    for (source_ref, _, _) in &captured {
        source_xml = remove_cell(&source_xml, source_ref)?;
    }
    if source_path == target_path {
        target_xml = source_xml.clone();
    }
    for (source_ref, destination_ref, cell) in captured {
        target_xml = upsert_cell(&target_xml, &destination_ref, &cell)?;
        changed_refs.push(format!("{source_sheet}!{source_ref}"));
        changed_refs.push(format!("{target_sheet}!{destination_ref}"));
    }
    if source_path != target_path {
        members.insert(source_path, source_xml.into_bytes());
    }
    members.insert(target_path, target_xml.into_bytes());
    Ok(())
}

fn rewrite_cell_reference(cell: &str, reference: &str) -> OfficeResult<String> {
    let opening_end = cell
        .find('>')
        .map(|index| index + 1)
        .ok_or_else(|| malformed_xml("c"))?;
    let opening = replace_or_add_attribute(&cell[..opening_end], "r", reference);
    Ok(format!("{opening}{}", &cell[opening_end..]))
}

fn edit_merge(
    members: &mut BTreeMap<String, Vec<u8>>,
    operation: &NormalizedOperation,
    changed_refs: &mut Vec<String>,
) -> OfficeResult<()> {
    let sheet = operation.string("sheet")?;
    let range = operation.string("range")?;
    CellRange::parse(range)?;
    let path = require_sheet_path(members, sheet)?;
    let source = member_text(members, &path)?.to_string();
    let updated = if operation.kind == "merge_cells" {
        add_merge(&source, range)?
    } else {
        remove_merge(&source, range)?
    };
    members.insert(path, updated.into_bytes());
    changed_refs.push(format!("{sheet}!{range}"));
    Ok(())
}

fn edit_freeze(
    members: &mut BTreeMap<String, Vec<u8>>,
    operation: &NormalizedOperation,
    changed_refs: &mut Vec<String>,
) -> OfficeResult<()> {
    let sheet = operation.string("sheet")?;
    let cell = operation.string("cell")?;
    parse_coordinate(cell)?;
    let path = require_sheet_path(members, sheet)?;
    let source = member_text(members, &path)?.to_string();
    members.insert(path, set_freeze(&source, cell)?.into_bytes());
    changed_refs.push(format!("{sheet}!freeze:{cell}"));
    Ok(())
}

fn edit_auto_filter(
    members: &mut BTreeMap<String, Vec<u8>>,
    operation: &NormalizedOperation,
    changed_refs: &mut Vec<String>,
) -> OfficeResult<()> {
    let sheet = operation.string("sheet")?;
    let range = operation.string("range")?;
    CellRange::parse(range)?;
    let path = require_sheet_path(members, sheet)?;
    let source = member_text(members, &path)?.to_string();
    members.insert(path, set_auto_filter(&source, range)?.into_bytes());
    changed_refs.push(format!("{sheet}!filter:{range}"));
    Ok(())
}

fn edit_column_width(
    members: &mut BTreeMap<String, Vec<u8>>,
    operation: &NormalizedOperation,
    changed_refs: &mut Vec<String>,
) -> OfficeResult<()> {
    let sheet = operation.string("sheet")?;
    let column = operation.usize("column")?;
    if !(1..=16_384).contains(&column) {
        return Err(OfficeError::invalid("column must be between 1 and 16384"));
    }
    let width = operation
        .value("width")
        .and_then(Value::as_f64)
        .filter(|value| *value > 0.0 && *value <= 255.0)
        .ok_or_else(|| OfficeError::invalid("column width must be between 0 and 255"))?;
    let path = require_sheet_path(members, sheet)?;
    let source = member_text(members, &path)?.to_string();
    members.insert(
        path,
        set_column_width_xml(&source, column, width)?.into_bytes(),
    );
    changed_refs.push(format!("{sheet}!column:{column}"));
    Ok(())
}

fn edit_row_height(
    members: &mut BTreeMap<String, Vec<u8>>,
    operation: &NormalizedOperation,
    changed_refs: &mut Vec<String>,
) -> OfficeResult<()> {
    let sheet = operation.string("sheet")?;
    let row = operation.usize("row")?;
    if !(1..=1_048_576).contains(&row) {
        return Err(OfficeError::invalid("row must be between 1 and 1048576"));
    }
    let height = operation
        .value("height")
        .and_then(Value::as_f64)
        .filter(|value| *value > 0.0)
        .ok_or_else(|| OfficeError::invalid("row height must be positive"))?;
    let path = require_sheet_path(members, sheet)?;
    let source = member_text(members, &path)?.to_string();
    members.insert(path, set_row_height_xml(&source, row, height)?.into_bytes());
    changed_refs.push(format!("{sheet}!row:{row}"));
    Ok(())
}

fn set_column_width_xml(source: &str, column: usize, width: f64) -> OfficeResult<String> {
    let column_xml =
        format!("<col min=\"{column}\" max=\"{column}\" width=\"{width}\" customWidth=\"1\"/>");
    if let Some((start, end)) = find_empty_element(source, "col", |opening| {
        attribute_equals(opening, "min", &column.to_string())
            && attribute_equals(opening, "max", &column.to_string())
    })? {
        return Ok(format!(
            "{}{}{}",
            &source[..start],
            column_xml,
            &source[end..]
        ));
    }
    if source.contains("</cols>") {
        insert_before(source, "</cols>", &column_xml)
    } else {
        let cols = format!("<cols>{column_xml}</cols>");
        insert_before(source, "<sheetData", &cols)
    }
}

fn set_row_height_xml(source: &str, row: usize, height: f64) -> OfficeResult<String> {
    if let Some((start, end)) = find_opening_element(source, "row", |opening| {
        attribute_equals(opening, "r", &row.to_string())
    })? {
        let opening = replace_or_add_attribute(&source[start..end], "ht", &height.to_string());
        let opening = replace_or_add_attribute(&opening, "customHeight", "1");
        return Ok(format!("{}{}{}", &source[..start], opening, &source[end..]));
    }
    insert_before(
        source,
        "</sheetData>",
        &format!("<row r=\"{row}\" ht=\"{height}\" customHeight=\"1\"></row>"),
    )
}

fn add_named_range(
    members: &mut BTreeMap<String, Vec<u8>>,
    operation: &NormalizedOperation,
) -> OfficeResult<()> {
    let name = operation.string("name")?;
    let reference = operation.string("reference")?;
    if name.chars().any(char::is_whitespace) || name.is_empty() {
        return Err(OfficeError::invalid(
            "named range name must be a non-empty machine-safe token",
        ));
    }
    let workbook = member_text(members, "xl/workbook.xml")?.to_string();
    if workbook.contains(&format!("name=\"{}\"", xml(name))) {
        return Err(OfficeError::new(
            "duplicate_named_range",
            "named range already exists",
            json!({"name": name}),
        ));
    }
    let node = format!(
        "<definedName name=\"{}\">{}</definedName>",
        xml(name),
        xml(reference)
    );
    let updated = if workbook.contains("</definedNames>") {
        insert_before(&workbook, "</definedNames>", &node)?
    } else {
        insert_before(
            &workbook,
            "</workbook>",
            &format!("<definedNames>{node}</definedNames>"),
        )?
    };
    members.insert("xl/workbook.xml".into(), updated.into_bytes());
    Ok(())
}

fn add_data_validation(
    members: &mut BTreeMap<String, Vec<u8>>,
    operation: &NormalizedOperation,
) -> OfficeResult<()> {
    let sheet = operation.string("sheet")?;
    let range = operation.string("range")?;
    CellRange::parse(range)?;
    let kind = operation
        .optional_string("validation_type")
        .unwrap_or("list");
    let formula = operation
        .value("formula1")
        .or_else(|| operation.value("formula"))
        .map(scalar_text)
        .unwrap_or_default();
    let allow_blank = operation.bool("allow_blank").unwrap_or(true);
    let node = format!(
        "<dataValidation type=\"{}\" allowBlank=\"{}\" sqref=\"{}\"><formula1>{}</formula1></dataValidation>",
        xml(kind),
        if allow_blank { 1 } else { 0 },
        xml(range),
        xml(&formula)
    );
    let path = require_sheet_path(members, sheet)?;
    let source = member_text(members, &path)?.to_string();
    let updated = append_counted_container(&source, "dataValidations", "count", &node)?;
    members.insert(path, updated.into_bytes());
    Ok(())
}

fn add_conditional_format(
    members: &mut BTreeMap<String, Vec<u8>>,
    operation: &NormalizedOperation,
) -> OfficeResult<()> {
    let sheet = operation.string("sheet")?;
    let range = operation.string("range")?;
    CellRange::parse(range)?;
    let formula = operation
        .value("formula")
        .map(scalar_text)
        .unwrap_or_else(|| "TRUE".to_string());
    let node = format!(
        "<conditionalFormatting sqref=\"{}\"><cfRule type=\"expression\" priority=\"1\"><formula>{}</formula></cfRule></conditionalFormatting>",
        xml(range),
        xml(&formula)
    );
    let path = require_sheet_path(members, sheet)?;
    let source = member_text(members, &path)?.to_string();
    let updated = insert_before_first(
        &source,
        &[
            "<dataValidations",
            "<hyperlinks",
            "<drawing",
            "<legacyDrawing",
            "<tableParts",
            "</worksheet>",
        ],
        &node,
    )?;
    members.insert(path, updated.into_bytes());
    Ok(())
}

fn add_table(
    members: &mut BTreeMap<String, Vec<u8>>,
    operation: &NormalizedOperation,
) -> OfficeResult<()> {
    let sheet = operation.string("sheet")?;
    let range_text = operation.string("range")?;
    let range = CellRange::parse(range_text)?;
    let name = operation.string("name")?;
    let sheet_path = require_sheet_path(members, sheet)?;
    let table_number = next_part_number(members, "xl/tables/table", ".xml");
    let table_path = format!("xl/tables/table{table_number}.xml");
    let relationship_id = format!("rIdRustClawTable{}", Uuid::new_v4().simple());
    add_part_relationship(
        members,
        &worksheet_relationships_path(&sheet_path),
        &relationship_id,
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/table",
        &format!("../tables/table{table_number}.xml"),
        false,
    )?;
    let columns = (range.start.column..=range.end.column)
        .enumerate()
        .map(|(index, _)| {
            format!(
                "<tableColumn id=\"{}\" name=\"Column{}\"/>",
                index + 1,
                index + 1
            )
        })
        .collect::<String>();
    members.insert(
        table_path.clone(),
        format!(
            "<?xml version=\"1.0\"?><table xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" id=\"{table_number}\" name=\"{}\" displayName=\"{}\" ref=\"{}\" totalsRowShown=\"0\"><autoFilter ref=\"{}\"/><tableColumns count=\"{}\">{columns}</tableColumns><tableStyleInfo name=\"TableStyleMedium2\" showFirstColumn=\"0\" showLastColumn=\"0\" showRowStripes=\"1\" showColumnStripes=\"0\"/></table>",
            xml(name),
            xml(name),
            xml(range_text),
            xml(range_text),
            range.end.column - range.start.column + 1
        )
        .into_bytes(),
    );
    ensure_content_override(
        members,
        &format!("/{table_path}"),
        "application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml",
    )?;
    let source = member_text(members, &sheet_path)?.to_string();
    let child = format!("<tablePart r:id=\"{}\"/>", xml(&relationship_id));
    let updated = append_table_part(&source, &child)?;
    members.insert(sheet_path, updated.into_bytes());
    Ok(())
}

fn add_chart(
    members: &mut BTreeMap<String, Vec<u8>>,
    operation: &NormalizedOperation,
) -> OfficeResult<()> {
    let sheet = operation.string("sheet")?;
    let range = operation.string("range")?;
    CellRange::parse(range)?;
    let title = operation.optional_string("title").unwrap_or("Chart");
    let chart_type = operation.optional_string("chart_type").unwrap_or("column");
    let chart_number = next_part_number(members, "xl/charts/chart", ".xml");
    let chart_path = format!("xl/charts/chart{chart_number}.xml");
    let drawing_path = ensure_sheet_drawing(members, sheet)?;
    let relationship_id = format!("rIdRustClawChart{}", Uuid::new_v4().simple());
    add_part_relationship(
        members,
        &relationships_path_for_part(&drawing_path),
        &relationship_id,
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart",
        &format!("../charts/chart{chart_number}.xml"),
        false,
    )?;
    members.insert(
        chart_path.clone(),
        chart_xml(title, chart_type, sheet, range).into_bytes(),
    );
    ensure_content_override(
        members,
        &format!("/{chart_path}"),
        "application/vnd.openxmlformats-officedocument.drawingml.chart+xml",
    )?;
    let drawing = member_text(members, &drawing_path)?.to_string();
    let anchor = chart_anchor(chart_number, &relationship_id);
    members.insert(
        drawing_path,
        insert_before(&drawing, "</xdr:wsDr>", &anchor)?.into_bytes(),
    );
    Ok(())
}

fn add_image(
    members: &mut BTreeMap<String, Vec<u8>>,
    operation: &NormalizedOperation,
) -> OfficeResult<()> {
    let sheet = operation.string("sheet")?;
    let source_path = crate::package::resolve_input_path(operation.string("path")?)?;
    let extension = image_extension(&source_path)?;
    let cell = operation.optional_string("cell").unwrap_or("A1");
    let coordinate = parse_coordinate(cell)?;
    let alt = operation.optional_string("alt").unwrap_or("image");
    let image_number = next_part_number(members, "xl/media/image", "");
    let image_path = format!("xl/media/image{image_number}.{extension}");
    let bytes = fs::read(&source_path).map_err(|error| {
        OfficeError::new(
            "source_unavailable",
            format!("cannot read spreadsheet image: {error}"),
            json!({"path": source_path.display().to_string()}),
        )
    })?;
    let drawing_path = ensure_sheet_drawing(members, sheet)?;
    let relationship_id = format!("rIdRustClawImage{}", Uuid::new_v4().simple());
    add_part_relationship(
        members,
        &relationships_path_for_part(&drawing_path),
        &relationship_id,
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image",
        &format!("../media/image{image_number}.{extension}"),
        false,
    )?;
    members.insert(image_path, bytes);
    ensure_content_default(members, extension, image_content_type(extension))?;
    let drawing = member_text(members, &drawing_path)?.to_string();
    let anchor = image_anchor(image_number, &relationship_id, coordinate, alt);
    members.insert(
        drawing_path,
        insert_before(&drawing, "</xdr:wsDr>", &anchor)?.into_bytes(),
    );
    Ok(())
}

fn add_hyperlink(
    members: &mut BTreeMap<String, Vec<u8>>,
    operation: &NormalizedOperation,
) -> OfficeResult<()> {
    let sheet = operation.string("sheet")?;
    let cell = operation.string("cell")?;
    parse_coordinate(cell)?;
    let url = operation.string("url")?;
    let sheet_path = require_sheet_path(members, sheet)?;
    let relationship_id = format!("rIdRustClawLink{}", Uuid::new_v4().simple());
    add_part_relationship(
        members,
        &worksheet_relationships_path(&sheet_path),
        &relationship_id,
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink",
        url,
        true,
    )?;
    let source = member_text(members, &sheet_path)?.to_string();
    let child = format!(
        "<hyperlink ref=\"{}\" r:id=\"{}\"/>",
        xml(cell),
        xml(&relationship_id)
    );
    let updated = if source.contains("</hyperlinks>") {
        insert_before(&source, "</hyperlinks>", &child)?
    } else {
        insert_before_first(
            &source,
            &["<drawing", "<legacyDrawing", "<tableParts", "</worksheet>"],
            &format!("<hyperlinks>{child}</hyperlinks>"),
        )?
    };
    members.insert(sheet_path, updated.into_bytes());
    Ok(())
}

fn add_comment(
    members: &mut BTreeMap<String, Vec<u8>>,
    operation: &NormalizedOperation,
) -> OfficeResult<()> {
    let sheet = operation.string("sheet")?;
    let cell = operation.string("cell")?;
    let coordinate = parse_coordinate(cell)?;
    let text = operation.string("text")?;
    let sheet_path = require_sheet_path(members, sheet)?;
    let relationships_path = worksheet_relationships_path(&sheet_path);
    let relationships = members
        .get(&relationships_path)
        .and_then(|bytes| std::str::from_utf8(bytes).ok())
        .map(relationship_map)
        .unwrap_or_default();
    let comments_path = relationships
        .values()
        .find(|(_, kind, external)| !external && kind.ends_with("/comments"))
        .map(|(target, _, _)| normalize_part_target(&sheet_path, target));
    let vml_path = relationships
        .values()
        .find(|(_, kind, external)| !external && kind.ends_with("/vmlDrawing"))
        .map(|(target, _, _)| normalize_part_target(&sheet_path, target));
    let (comments_path, vml_path) = match (comments_path, vml_path) {
        (Some(comments), Some(vml)) => (comments, vml),
        (None, None) => create_comment_parts(members, &sheet_path)?,
        _ => {
            return Err(OfficeError::new(
                "malformed_package",
                "worksheet comment relationships are incomplete",
                json!({"sheet": sheet}),
            ))
        }
    };
    let comments = member_text(members, &comments_path)?.to_string();
    if comments.contains(&format!("ref=\"{}\"", xml(cell))) {
        return Err(OfficeError::new(
            "duplicate_comment",
            "worksheet cell already has a comment",
            json!({"sheet": sheet, "cell": cell}),
        ));
    }
    let comment = format!(
        "<comment ref=\"{}\" authorId=\"0\"><text><r><t xml:space=\"preserve\">{}</t></r></text></comment>",
        xml(cell),
        xml(text)
    );
    members.insert(
        comments_path,
        insert_before(&comments, "</commentList>", &comment)?.into_bytes(),
    );
    let vml = member_text(members, &vml_path)?.to_string();
    let shape_id = 1025 + vml.matches("<v:shape ").count();
    let shape = comment_shape(shape_id, coordinate);
    members.insert(
        vml_path,
        insert_before(&vml, "</xml>", &shape)?.into_bytes(),
    );
    Ok(())
}

fn create_comment_parts(
    members: &mut BTreeMap<String, Vec<u8>>,
    sheet_path: &str,
) -> OfficeResult<(String, String)> {
    let number = next_part_number(members, "xl/comments", ".xml");
    let comments_path = format!("xl/comments{number}.xml");
    let vml_path = format!("xl/drawings/vmlDrawing{number}.vml");
    let comments_id = format!("rIdRustClawComments{}", Uuid::new_v4().simple());
    let vml_id = format!("rIdRustClawVml{}", Uuid::new_v4().simple());
    let relationships_path = worksheet_relationships_path(sheet_path);
    add_part_relationship(
        members,
        &relationships_path,
        &comments_id,
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments",
        &format!("../comments{number}.xml"),
        false,
    )?;
    add_part_relationship(
        members,
        &relationships_path,
        &vml_id,
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/vmlDrawing",
        &format!("../drawings/vmlDrawing{number}.vml"),
        false,
    )?;
    members.insert(
        comments_path.clone(),
        br#"<?xml version="1.0" encoding="UTF-8"?><comments xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><authors><author>RustClaw</author></authors><commentList></commentList></comments>"#.to_vec(),
    );
    members.insert(vml_path.clone(), empty_comments_vml().as_bytes().to_vec());
    ensure_content_override(
        members,
        &format!("/{comments_path}"),
        "application/vnd.openxmlformats-officedocument.spreadsheetml.comments+xml",
    )?;
    ensure_content_default(
        members,
        "vml",
        "application/vnd.openxmlformats-officedocument.vmlDrawing",
    )?;
    let source = member_text(members, sheet_path)?.to_string();
    let legacy = format!("<legacyDrawing r:id=\"{}\"/>", xml(&vml_id));
    members.insert(
        sheet_path.to_string(),
        insert_before_first(&source, &["<tableParts", "</worksheet>"], &legacy)?.into_bytes(),
    );
    Ok((comments_path, vml_path))
}

fn ensure_sheet_drawing(
    members: &mut BTreeMap<String, Vec<u8>>,
    sheet: &str,
) -> OfficeResult<String> {
    let sheet_path = require_sheet_path(members, sheet)?;
    let relationships_path = worksheet_relationships_path(&sheet_path);
    let relationships = members
        .get(&relationships_path)
        .and_then(|bytes| std::str::from_utf8(bytes).ok())
        .map(relationship_map)
        .unwrap_or_default();
    if let Some((target, _, _)) = relationships
        .values()
        .find(|(_, kind, external)| !external && kind.ends_with("/drawing"))
    {
        return Ok(normalize_part_target(&sheet_path, target));
    }
    let number = next_part_number(members, "xl/drawings/drawing", ".xml");
    let drawing_path = format!("xl/drawings/drawing{number}.xml");
    let relationship_id = format!("rIdRustClawDrawing{}", Uuid::new_v4().simple());
    add_part_relationship(
        members,
        &relationships_path,
        &relationship_id,
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/drawing",
        &format!("../drawings/drawing{number}.xml"),
        false,
    )?;
    members.insert(drawing_path.clone(), empty_drawing().as_bytes().to_vec());
    ensure_content_override(
        members,
        &format!("/{drawing_path}"),
        "application/vnd.openxmlformats-officedocument.drawing+xml",
    )?;
    let source = member_text(members, &sheet_path)?.to_string();
    let drawing = format!("<drawing r:id=\"{}\"/>", xml(&relationship_id));
    members.insert(
        sheet_path,
        insert_before_first(
            &source,
            &["<legacyDrawing", "<tableParts", "</worksheet>"],
            &drawing,
        )?
        .into_bytes(),
    );
    Ok(drawing_path)
}

fn add_part_relationship(
    members: &mut BTreeMap<String, Vec<u8>>,
    relationships_path: &str,
    id: &str,
    kind: &str,
    target: &str,
    external: bool,
) -> OfficeResult<()> {
    let relationship = format!(
        "<Relationship Id=\"{}\" Type=\"{}\" Target=\"{}\"{}/>",
        xml(id),
        xml(kind),
        xml(target),
        if external {
            " TargetMode=\"External\""
        } else {
            ""
        }
    );
    let source = members
        .get(relationships_path)
        .and_then(|bytes| std::str::from_utf8(bytes).ok())
        .unwrap_or(
            "<?xml version=\"1.0\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"></Relationships>",
        )
        .to_string();
    members.insert(
        relationships_path.to_string(),
        insert_before(&source, "</Relationships>", &relationship)?.into_bytes(),
    );
    Ok(())
}

fn ensure_content_override(
    members: &mut BTreeMap<String, Vec<u8>>,
    part_name: &str,
    content_type: &str,
) -> OfficeResult<()> {
    let source = member_text(members, "[Content_Types].xml")?.to_string();
    if source.contains(&format!("PartName=\"{}\"", xml(part_name))) {
        return Ok(());
    }
    let value = format!(
        "<Override PartName=\"{}\" ContentType=\"{}\"/>",
        xml(part_name),
        xml(content_type)
    );
    members.insert(
        "[Content_Types].xml".into(),
        insert_before(&source, "</Types>", &value)?.into_bytes(),
    );
    Ok(())
}

fn ensure_content_default(
    members: &mut BTreeMap<String, Vec<u8>>,
    extension: &str,
    content_type: &str,
) -> OfficeResult<()> {
    let source = member_text(members, "[Content_Types].xml")?.to_string();
    if source.contains(&format!("Extension=\"{}\"", xml(extension))) {
        return Ok(());
    }
    let value = format!(
        "<Default Extension=\"{}\" ContentType=\"{}\"/>",
        xml(extension),
        xml(content_type)
    );
    members.insert(
        "[Content_Types].xml".into(),
        insert_before(&source, "</Types>", &value)?.into_bytes(),
    );
    Ok(())
}

fn add_sheet(
    members: &mut BTreeMap<String, Vec<u8>>,
    name: &str,
    source_xml: Option<Vec<u8>>,
) -> OfficeResult<()> {
    validate_sheet_name(name)?;
    reject_duplicate_sheet_name(members, name, None)?;
    let entries = sheet_entries(members)?;
    let number = next_sheet_part_number(members);
    let sheet_id = entries.iter().map(|entry| entry.id).max().unwrap_or(0) + 1;
    let relationship_id = format!("rIdRustClawSheet{}", Uuid::new_v4().simple());
    let path = format!("xl/worksheets/sheet{number}.xml");
    let workbook = member_text(members, "xl/workbook.xml")?.to_string();
    let sheet_node = format!(
        "<sheet name=\"{}\" sheetId=\"{sheet_id}\" state=\"visible\" r:id=\"{}\"/>",
        xml(name),
        xml(&relationship_id)
    );
    members.insert(
        "xl/workbook.xml".into(),
        insert_before(&workbook, "</sheets>", &sheet_node)?.into_bytes(),
    );
    let relationships = member_text(members, "xl/_rels/workbook.xml.rels")?.to_string();
    let relationship = format!(
        "<Relationship Id=\"{}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet{number}.xml\"/>",
        xml(&relationship_id)
    );
    members.insert(
        "xl/_rels/workbook.xml.rels".into(),
        insert_before(&relationships, "</Relationships>", &relationship)?.into_bytes(),
    );
    let content_types = member_text(members, "[Content_Types].xml")?.to_string();
    let content_type = format!(
        "<Override PartName=\"/{path}\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/>"
    );
    members.insert(
        "[Content_Types].xml".into(),
        insert_before(&content_types, "</Types>", &content_type)?.into_bytes(),
    );
    members.insert(
        path,
        source_xml.unwrap_or_else(|| {
            br#"<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><dimension ref="A1"/><sheetViews><sheetView workbookViewId="0"/></sheetViews><sheetData></sheetData></worksheet>"#.to_vec()
        }),
    );
    Ok(())
}

fn copy_sheet(
    members: &mut BTreeMap<String, Vec<u8>>,
    operation: &NormalizedOperation,
) -> OfficeResult<()> {
    let source_name = operation.string("sheet")?;
    let new_name = operation.string("new_name")?;
    let source_path = require_sheet_path(members, source_name)?;
    let relationships_path = worksheet_relationships_path(&source_path);
    if members
        .get(&relationships_path)
        .is_some_and(|bytes| relationships_have_entries(bytes))
    {
        return Err(OfficeError::unsupported(
            "copy_sheet requires a source worksheet without dependent package relationships",
            json!({"sheet": source_name, "relationships_part": relationships_path}),
        ));
    }
    let source = members
        .get(&source_path)
        .cloned()
        .ok_or_else(|| missing_part(&source_path))?;
    add_sheet(members, new_name, Some(source))
}

fn reorder_sheet(
    members: &mut BTreeMap<String, Vec<u8>>,
    operation: &NormalizedOperation,
) -> OfficeResult<()> {
    let sheet = operation.string("sheet")?;
    let index = operation.usize("index")?;
    let workbook = member_text(members, "xl/workbook.xml")?.to_string();
    let (start, end) = element_body_range(&workbook, "sheets")?;
    let body = &workbook[start..end];
    let mut nodes = collect_sheet_nodes(body)?;
    let current = nodes
        .iter()
        .position(|(name, _)| name == sheet)
        .ok_or_else(|| worksheet_not_found(sheet, nodes.iter().map(|item| &item.0)))?;
    if index >= nodes.len() {
        return Err(OfficeError::new(
            "invalid_sheet_index",
            "worksheet index is outside the workbook",
            json!({"index": index, "sheet_count": nodes.len()}),
        ));
    }
    let node = nodes.remove(current);
    nodes.insert(index, node);
    let reordered = nodes.into_iter().map(|(_, node)| node).collect::<String>();
    members.insert(
        "xl/workbook.xml".into(),
        format!("{}{}{}", &workbook[..start], reordered, &workbook[end..]).into_bytes(),
    );
    Ok(())
}

fn delete_sheet(members: &mut BTreeMap<String, Vec<u8>>, sheet: &str) -> OfficeResult<()> {
    let entries = sheet_entries(members)?;
    if entries.len() <= 1 {
        return Err(OfficeError::new(
            "last_worksheet",
            "a workbook must retain at least one worksheet",
            json!({"sheet": sheet}),
        ));
    }
    let entry = entries
        .iter()
        .find(|entry| entry.name == sheet)
        .cloned()
        .ok_or_else(|| worksheet_not_found(sheet, entries.iter().map(|entry| &entry.name)))?;
    let relationships_path = worksheet_relationships_path(&entry.path);
    if members
        .get(&relationships_path)
        .is_some_and(|bytes| relationships_have_entries(bytes))
    {
        return Err(OfficeError::unsupported(
            "delete_sheet requires a worksheet without dependent package relationships",
            json!({"sheet": sheet, "relationships_part": relationships_path}),
        ));
    }
    let workbook = member_text(members, "xl/workbook.xml")?.to_string();
    let workbook = remove_sheet_node(&workbook, sheet)?;
    members.insert("xl/workbook.xml".into(), workbook.into_bytes());
    let relationships = member_text(members, "xl/_rels/workbook.xml.rels")?.to_string();
    members.insert(
        "xl/_rels/workbook.xml.rels".into(),
        remove_relationship(&relationships, &entry.relationship_id)?.into_bytes(),
    );
    let content_types = member_text(members, "[Content_Types].xml")?.to_string();
    members.insert(
        "[Content_Types].xml".into(),
        remove_content_type(&content_types, &format!("/{}", entry.path))?.into_bytes(),
    );
    members.remove(&entry.path);
    members.remove(&relationships_path);
    Ok(())
}

fn reject_duplicate_sheet_name(
    members: &BTreeMap<String, Vec<u8>>,
    name: &str,
    except: Option<&str>,
) -> OfficeResult<()> {
    if sheet_entries(members)?
        .iter()
        .any(|entry| entry.name == name && except != Some(entry.name.as_str()))
    {
        return Err(OfficeError::new(
            "duplicate_worksheet",
            "worksheet names must be unique",
            json!({"sheet": name}),
        ));
    }
    Ok(())
}

fn require_sheet_path(members: &BTreeMap<String, Vec<u8>>, sheet: &str) -> OfficeResult<String> {
    let entries = sheet_entries(members)?;
    entries
        .iter()
        .find(|entry| entry.name == sheet)
        .map(|entry| entry.path.clone())
        .ok_or_else(|| worksheet_not_found(sheet, entries.iter().map(|entry| &entry.name)))
}

fn sheet_entries(members: &BTreeMap<String, Vec<u8>>) -> OfficeResult<Vec<SheetEntry>> {
    let workbook = member_text(members, "xl/workbook.xml")?;
    let relationships = member_text(members, "xl/_rels/workbook.xml.rels").map(relationship_map)?;
    let mut reader = Reader::from_str(workbook);
    reader.config_mut().trim_text(true);
    let mut entries = Vec::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(element)) | Ok(Event::Empty(element))
                if local_name(element.name().as_ref()) == b"sheet" =>
            {
                let name = attr_value(&element, b"name").unwrap_or_default();
                let id = attr_value(&element, b"sheetId")
                    .and_then(|value| value.parse().ok())
                    .unwrap_or((entries.len() + 1) as u32);
                let relationship_id = attr_value_qualified(&element, b"r:id").unwrap_or_default();
                let path = relationships
                    .get(&relationship_id)
                    .filter(|(_, _, external)| !external)
                    .map(|(target, _, _)| normalize_xl_target(target))
                    .ok_or_else(|| {
                        OfficeError::new(
                            "missing_package_part",
                            "worksheet relationship is missing",
                            json!({"sheet": name, "relationship_id": relationship_id}),
                        )
                    })?;
                entries.push(SheetEntry {
                    name,
                    id,
                    relationship_id,
                    path,
                });
            }
            Ok(Event::Eof) => break,
            Err(error) => {
                return Err(OfficeError::new(
                    "malformed_xml",
                    format!("cannot parse workbook XML: {error}"),
                    json!({"part": "xl/workbook.xml"}),
                ))
            }
            _ => {}
        }
    }
    Ok(entries)
}

fn collect_sheet_nodes(body: &str) -> OfficeResult<Vec<(String, String)>> {
    let mut output = Vec::new();
    let mut cursor = 0usize;
    while let Some(relative) = body[cursor..].find("<sheet") {
        let start = cursor + relative;
        let boundary = body.as_bytes().get(start + 6).copied();
        if !matches!(boundary, Some(b' ') | Some(b'>') | Some(b'/')) {
            cursor = start + 6;
            continue;
        }
        let end = body[start..]
            .find('>')
            .map(|relative| start + relative + 1)
            .ok_or_else(|| malformed_xml("sheet"))?;
        let node = &body[start..end];
        let name = attribute_value(node, "name").unwrap_or_default();
        output.push((name, node.to_string()));
        cursor = end;
    }
    Ok(output)
}

fn remove_sheet_node(workbook: &str, sheet: &str) -> OfficeResult<String> {
    let (body_start, body_end) = element_body_range(workbook, "sheets")?;
    let body = &workbook[body_start..body_end];
    let nodes = collect_sheet_nodes(body)?;
    let mut removed = false;
    let replacement = nodes
        .into_iter()
        .filter_map(|(name, node)| {
            if name == sheet {
                removed = true;
                None
            } else {
                Some(node)
            }
        })
        .collect::<String>();
    if !removed {
        return Err(worksheet_not_found(sheet, std::iter::empty::<&String>()));
    }
    Ok(format!(
        "{}{}{}",
        &workbook[..body_start],
        replacement,
        &workbook[body_end..]
    ))
}

fn remove_relationship(source: &str, relationship_id: &str) -> OfficeResult<String> {
    remove_empty_element(source, "Relationship", |opening| {
        attribute_equals(opening, "Id", relationship_id)
    })
}

fn remove_content_type(source: &str, part_name: &str) -> OfficeResult<String> {
    remove_empty_element(source, "Override", |opening| {
        attribute_equals(opening, "PartName", part_name)
    })
}

fn remove_empty_element(
    source: &str,
    element: &str,
    predicate: impl Fn(&str) -> bool,
) -> OfficeResult<String> {
    let Some((start, end)) = find_empty_element(source, element, predicate)? else {
        return Ok(source.to_string());
    };
    Ok(format!("{}{}", &source[..start], &source[end..]))
}

fn find_empty_element(
    source: &str,
    element: &str,
    predicate: impl Fn(&str) -> bool,
) -> OfficeResult<Option<(usize, usize)>> {
    let mut cursor = 0usize;
    let token = format!("<{element}");
    while let Some(relative) = source[cursor..].find(&token) {
        let start = cursor + relative;
        let end = source[start..]
            .find('>')
            .map(|relative| start + relative + 1)
            .ok_or_else(|| malformed_xml(element))?;
        let opening = &source[start..end];
        if opening.trim_end().ends_with("/>") && predicate(opening) {
            return Ok(Some((start, end)));
        }
        cursor = end;
    }
    Ok(None)
}

fn find_opening_element(
    source: &str,
    element: &str,
    predicate: impl Fn(&str) -> bool,
) -> OfficeResult<Option<(usize, usize)>> {
    let mut cursor = 0usize;
    let token = format!("<{element}");
    while let Some(relative) = source[cursor..].find(&token) {
        let start = cursor + relative;
        let boundary = source.as_bytes().get(start + token.len()).copied();
        if !matches!(boundary, Some(b' ') | Some(b'>') | Some(b'/')) {
            cursor = start + token.len();
            continue;
        }
        let end = source[start..]
            .find('>')
            .map(|relative| start + relative + 1)
            .ok_or_else(|| malformed_xml(element))?;
        if predicate(&source[start..end]) {
            return Ok(Some((start, end)));
        }
        cursor = end;
    }
    Ok(None)
}

fn append_counted_container(
    source: &str,
    element: &str,
    count_attribute: &str,
    child: &str,
) -> OfficeResult<String> {
    let closing = format!("</{element}>");
    if let Some(close) = source.find(&closing) {
        let opening_start = source[..close]
            .rfind(&format!("<{element}"))
            .ok_or_else(|| malformed_xml(element))?;
        let opening_end = source[opening_start..]
            .find('>')
            .map(|relative| opening_start + relative + 1)
            .ok_or_else(|| malformed_xml(element))?;
        let opening = &source[opening_start..opening_end];
        let count = attribute_value(opening, count_attribute)
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0)
            + 1;
        let opening = replace_or_add_attribute(opening, count_attribute, &count.to_string());
        return Ok(format!(
            "{}{}{}{}{}",
            &source[..opening_start],
            opening,
            &source[opening_end..close],
            child,
            &source[close..]
        ));
    }
    let node = format!("<{element} {count_attribute}=\"1\">{child}</{element}>");
    insert_before_first(
        source,
        &[
            "<hyperlinks",
            "<drawing",
            "<legacyDrawing",
            "<tableParts",
            "</worksheet>",
        ],
        &node,
    )
}

fn append_table_part(source: &str, child: &str) -> OfficeResult<String> {
    if source.contains("</tableParts>") {
        return append_counted_container(source, "tableParts", "count", child);
    }
    insert_before(
        source,
        "</worksheet>",
        &format!("<tableParts count=\"1\">{child}</tableParts>"),
    )
}

fn insert_before_first(source: &str, anchors: &[&str], value: &str) -> OfficeResult<String> {
    let index = anchors
        .iter()
        .filter_map(|anchor| source.find(anchor))
        .min()
        .ok_or_else(|| malformed_xml("worksheet"))?;
    Ok(format!("{}{}{}", &source[..index], value, &source[index..]))
}

fn element_body_range(source: &str, element: &str) -> OfficeResult<(usize, usize)> {
    let start_token = format!("<{element}");
    let opening = source
        .find(&start_token)
        .ok_or_else(|| malformed_xml(element))?;
    let body_start = source[opening..]
        .find('>')
        .map(|relative| opening + relative + 1)
        .ok_or_else(|| malformed_xml(element))?;
    let closing = format!("</{element}>");
    let body_end = source[body_start..]
        .find(&closing)
        .map(|relative| body_start + relative)
        .ok_or_else(|| malformed_xml(element))?;
    Ok((body_start, body_end))
}

fn attribute_equals(opening: &str, name: &str, expected: &str) -> bool {
    attribute_value(opening, name).as_deref() == Some(expected)
}

fn attribute_value(opening: &str, name: &str) -> Option<String> {
    for quote in ['"', '\''] {
        let prefix = format!("{name}={quote}");
        let start = opening.find(&prefix)? + prefix.len();
        let end = opening[start..].find(quote)? + start;
        return Some(opening[start..end].to_string());
    }
    None
}

fn worksheet_relationships_path(sheet_path: &str) -> String {
    let (base, file) = sheet_path.rsplit_once('/').unwrap_or(("", sheet_path));
    format!("{base}/_rels/{file}.rels")
}

fn relationships_have_entries(bytes: &[u8]) -> bool {
    std::str::from_utf8(bytes)
        .ok()
        .is_some_and(|value| value.contains("<Relationship "))
}

fn relationships_path_for_part(part: &str) -> String {
    let (base, file) = part.rsplit_once('/').unwrap_or(("", part));
    format!("{base}/_rels/{file}.rels")
}

fn normalize_part_target(source: &str, target: &str) -> String {
    if target.starts_with('/') {
        return target.trim_start_matches('/').to_string();
    }
    let base = source.rsplit_once('/').map(|(base, _)| base).unwrap_or("");
    normalize_segments(&format!("{base}/{target}"))
}

fn normalize_segments(value: &str) -> String {
    let mut parts = Vec::new();
    for part in value.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(part),
        }
    }
    parts.join("/")
}

fn next_part_number(members: &BTreeMap<String, Vec<u8>>, prefix: &str, suffix: &str) -> usize {
    let used = members
        .keys()
        .filter_map(|name| {
            let value = name.strip_prefix(prefix)?;
            let value = if suffix.is_empty() {
                value
                    .split_once('.')
                    .map(|(number, _)| number)
                    .unwrap_or(value)
            } else {
                value.strip_suffix(suffix)?
            };
            value.parse::<usize>().ok()
        })
        .collect::<BTreeSet<_>>();
    (1..).find(|number| !used.contains(number)).unwrap_or(1)
}

fn next_sheet_part_number(members: &BTreeMap<String, Vec<u8>>) -> usize {
    let used = members
        .keys()
        .filter_map(|name| {
            name.strip_prefix("xl/worksheets/sheet")
                .and_then(|value| value.strip_suffix(".xml"))
                .and_then(|value| value.parse::<usize>().ok())
        })
        .collect::<BTreeSet<_>>();
    (1..).find(|number| !used.contains(number)).unwrap_or(1)
}

fn empty_drawing() -> &'static str {
    r#"<?xml version="1.0"?><xdr:wsDr xmlns:xdr="http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"></xdr:wsDr>"#
}

fn chart_anchor(id: usize, relationship_id: &str) -> String {
    format!(
        "<xdr:twoCellAnchor><xdr:from><xdr:col>0</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>4</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from><xdr:to><xdr:col>8</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>20</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:to><xdr:graphicFrame><xdr:nvGraphicFramePr><xdr:cNvPr id=\"{id}\" name=\"Chart {id}\"/><xdr:cNvGraphicFramePr/></xdr:nvGraphicFramePr><xdr:xfrm/><a:graphic><a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/chart\"><c:chart r:id=\"{}\"/></a:graphicData></a:graphic></xdr:graphicFrame><xdr:clientData/></xdr:twoCellAnchor>",
        xml(relationship_id)
    )
}

fn chart_xml(title: &str, chart_type: &str, sheet: &str, range: &str) -> String {
    let chart_tag = match chart_type {
        "line" => "lineChart",
        "pie" => "pieChart",
        "bar" => "barChart",
        _ => "barChart",
    };
    let formula = format!("'{}'!{}", sheet.replace('\'', "''"), range);
    format!(
        "<?xml version=\"1.0\"?><c:chartSpace xmlns:c=\"http://schemas.openxmlformats.org/drawingml/2006/chart\" xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\"><c:chart><c:title><c:tx><c:rich><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>{}</a:t></a:r></a:p></c:rich></c:tx></c:title><c:plotArea><c:layout/><c:{chart_tag}><c:ser><c:idx val=\"0\"/><c:order val=\"0\"/><c:val><c:numRef><c:f>{}</c:f></c:numRef></c:val></c:ser></c:{chart_tag}></c:plotArea></c:chart></c:chartSpace>",
        xml(title),
        xml(&formula)
    )
}

fn image_anchor(id: usize, relationship_id: &str, coordinate: CellCoordinate, alt: &str) -> String {
    let column = coordinate.column.saturating_sub(1);
    let row = coordinate.row.saturating_sub(1);
    format!(
        "<xdr:oneCellAnchor><xdr:from><xdr:col>{column}</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>{row}</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from><xdr:ext cx=\"2400000\" cy=\"1600000\"/><xdr:pic><xdr:nvPicPr><xdr:cNvPr id=\"{id}\" name=\"Image {id}\" descr=\"{}\"/><xdr:cNvPicPr/></xdr:nvPicPr><xdr:blipFill><a:blip r:embed=\"{}\"/><a:stretch><a:fillRect/></a:stretch></xdr:blipFill><xdr:spPr><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></xdr:spPr></xdr:pic><xdr:clientData/></xdr:oneCellAnchor>",
        xml(alt),
        xml(relationship_id)
    )
}

fn image_extension(path: &Path) -> OfficeResult<&str> {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())
        .ok_or_else(|| OfficeError::invalid("image source requires a file extension"))?;
    match extension.as_str() {
        "png" => Ok("png"),
        "jpg" | "jpeg" => Ok("jpeg"),
        "gif" => Ok("gif"),
        _ => Err(OfficeError::unsupported(
            "supported spreadsheet image inputs are PNG, JPEG, and GIF",
            json!({"extension": extension}),
        )),
    }
}

fn image_content_type(extension: &str) -> &'static str {
    match extension {
        "jpeg" | "jpg" => "image/jpeg",
        "gif" => "image/gif",
        _ => "image/png",
    }
}

fn empty_comments_vml() -> &'static str {
    r#"<?xml version="1.0" encoding="UTF-8"?><xml xmlns:v="urn:schemas-microsoft-com:vml" xmlns:o="urn:schemas-microsoft-com:office:office" xmlns:x="urn:schemas-microsoft-com:office:excel"><o:shapelayout v:ext="edit"><o:idmap v:ext="edit" data="1"/></o:shapelayout><v:shapetype id="_x0000_t202" coordsize="21600,21600" o:spt="202" path="m,l,21600r21600,l21600,xe"><v:stroke joinstyle="miter"/><v:path gradientshapeok="t" o:connecttype="rect"/></v:shapetype></xml>"#
}

fn comment_shape(id: usize, coordinate: CellCoordinate) -> String {
    let row = coordinate.row.saturating_sub(1);
    let column = coordinate.column.saturating_sub(1);
    format!(
        "<v:shape id=\"_x0000_s{id}\" type=\"#_x0000_t202\" style=\"position:absolute;visibility:hidden\" fillcolor=\"#ffffe1\" o:insetmode=\"auto\"><v:fill color2=\"#ffffe1\"/><v:shadow on=\"t\" color=\"black\" obscured=\"t\"/><v:path o:connecttype=\"none\"/><v:textbox style=\"mso-direction-alt:auto\"><div style=\"text-align:left\"/></v:textbox><x:ClientData ObjectType=\"Note\"><x:MoveWithCells/><x:SizeWithCells/><x:Anchor>{column}, 15, {row}, 2, {}, 15, {}, 4</x:Anchor><x:AutoFill>False</x:AutoFill><x:Row>{row}</x:Row><x:Column>{column}</x:Column></x:ClientData></v:shape>",
        column + 3,
        row + 4
    )
}

fn normalize_xl_target(target: &str) -> String {
    let target = if target.starts_with('/') {
        target.trim_start_matches('/').to_string()
    } else if target.starts_with("xl/") {
        target.to_string()
    } else {
        format!("xl/{target}")
    };
    let mut parts = Vec::new();
    for part in target.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(part),
        }
    }
    parts.join("/")
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

fn missing_part(part: &str) -> OfficeError {
    OfficeError::new(
        "missing_package_part",
        "required XLSX package part is missing",
        json!({"part": part}),
    )
}

fn worksheet_not_found<'a>(
    sheet: &str,
    available: impl Iterator<Item = &'a String>,
) -> OfficeError {
    OfficeError::new(
        "worksheet_not_found",
        "requested worksheet does not exist",
        json!({"sheet": sheet, "available_sheets": available.collect::<Vec<_>>() }),
    )
}

#[cfg(test)]
#[path = "xlsx_edit_tests.rs"]
mod tests;
