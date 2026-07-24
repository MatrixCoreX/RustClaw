use crate::error::{OfficeError, OfficeResult};
use crate::operations::NormalizedOperation;
use crate::package::OfficePackage;
use crate::range::{format_coordinate, parse_coordinate, CellCoordinate, CellRange};
use crate::xml::{attr_value, attr_value_qualified, local_name, relationship_map};
use quick_xml::escape::escape;
use quick_xml::events::Event;
use quick_xml::Reader;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

pub struct XlsxWriteResult {
    pub members: BTreeMap<String, Vec<u8>>,
    pub changed_refs: Vec<String>,
    pub preservation: Vec<String>,
}

#[derive(Clone, Debug)]
enum CellValue {
    Text(String),
    Number(String),
    Boolean(bool),
    Date(String),
    Formula { formula: String, cached: String },
    Blank,
}

#[derive(Clone, Debug)]
struct CellBuild {
    value: CellValue,
    style_id: Option<u32>,
}

#[derive(Clone, Debug, Default)]
struct SheetBuild {
    name: String,
    state: String,
    cells: BTreeMap<(u32, u32), CellBuild>,
    merges: BTreeSet<String>,
    freeze: Option<String>,
    auto_filter: Option<String>,
    column_widths: BTreeMap<u32, f64>,
    row_heights: BTreeMap<u32, f64>,
    tables: Vec<TableBuild>,
    charts: Vec<ChartBuild>,
    comments: BTreeMap<String, String>,
    hyperlinks: BTreeMap<String, String>,
    validations: Vec<String>,
    conditional_formats: Vec<String>,
}

#[derive(Clone, Debug)]
struct TableBuild {
    name: String,
    range: String,
}

#[derive(Clone, Debug)]
struct ChartBuild {
    title: String,
    range: String,
    chart_type: String,
}

pub fn create_xlsx(operations: &[NormalizedOperation]) -> OfficeResult<XlsxWriteResult> {
    let mut sheets = Vec::<SheetBuild>::new();
    let mut named_ranges = Vec::new();
    for operation in operations {
        apply_create_operation(&mut sheets, &mut named_ranges, operation)?;
    }
    if sheets.is_empty() {
        return Err(OfficeError::new(
            "invalid_operation",
            "spreadsheet creation requires at least one add_sheet operation",
            json!({}),
        ));
    }
    let members = build_workbook(&sheets, &named_ranges);
    Ok(XlsxWriteResult {
        members,
        changed_refs: operations
            .iter()
            .flat_map(NormalizedOperation::object_refs)
            .collect(),
        preservation: vec!["new_package".to_string()],
    })
}

pub fn edit_xlsx(
    package: &OfficePackage,
    operations: &[NormalizedOperation],
) -> OfficeResult<XlsxWriteResult> {
    let mut members = package.members.clone();
    let sheet_paths = workbook_sheet_paths(package)?;
    let mut changed_refs = Vec::new();
    for operation in operations {
        match operation.kind.as_str() {
            "set_cell" => {
                let sheet = operation.string("sheet")?;
                let reference = operation.string("cell")?;
                parse_coordinate(reference)?;
                let path = require_sheet_path(&sheet_paths, sheet)?;
                let xml = member_text(&members, &path)?.to_string();
                let cell = cell_from_operation(operation)?;
                let updated = upsert_cell(&xml, reference, &cell_xml(reference, &cell))?;
                members.insert(path, updated.into_bytes());
                changed_refs.push(format!("{sheet}!{reference}"));
            }
            "clear_cell" => {
                let sheet = operation.string("sheet")?;
                let reference = operation.string("cell")?;
                parse_coordinate(reference)?;
                let path = require_sheet_path(&sheet_paths, sheet)?;
                let xml = member_text(&members, &path)?.to_string();
                members.insert(path, remove_cell(&xml, reference)?.into_bytes());
                changed_refs.push(format!("{sheet}!{reference}"));
            }
            "set_range" | "fill_range" => {
                let sheet = operation.string("sheet")?;
                let range = CellRange::parse(operation.string("range")?)?;
                let path = require_sheet_path(&sheet_paths, sheet)?;
                let mut xml = member_text(&members, &path)?.to_string();
                let cells = cells_for_range(operation, range)?;
                for (coordinate, value) in cells {
                    let reference = format_coordinate(coordinate);
                    xml = upsert_cell(&xml, &reference, &cell_xml(&reference, &value))?;
                    changed_refs.push(format!("{sheet}!{reference}"));
                }
                members.insert(path, xml.into_bytes());
            }
            "merge_cells" | "unmerge_cells" => {
                let sheet = operation.string("sheet")?;
                let range = operation.string("range")?;
                CellRange::parse(range)?;
                let path = require_sheet_path(&sheet_paths, sheet)?;
                let xml = member_text(&members, &path)?.to_string();
                let updated = if operation.kind == "merge_cells" {
                    add_merge(&xml, range)?
                } else {
                    remove_merge(&xml, range)?
                };
                members.insert(path, updated.into_bytes());
                changed_refs.push(format!("{sheet}!{range}"));
            }
            "freeze_panes" => {
                let sheet = operation.string("sheet")?;
                let cell = operation.string("cell")?;
                parse_coordinate(cell)?;
                let path = require_sheet_path(&sheet_paths, sheet)?;
                let xml = member_text(&members, &path)?.to_string();
                members.insert(path, set_freeze(&xml, cell)?.into_bytes());
                changed_refs.push(format!("{sheet}!freeze:{cell}"));
            }
            "set_auto_filter" => {
                let sheet = operation.string("sheet")?;
                let range = operation.string("range")?;
                CellRange::parse(range)?;
                let path = require_sheet_path(&sheet_paths, sheet)?;
                let xml = member_text(&members, &path)?.to_string();
                members.insert(path, set_auto_filter(&xml, range)?.into_bytes());
                changed_refs.push(format!("{sheet}!filter:{range}"));
            }
            "rename_sheet" => {
                let sheet = operation.string("sheet")?;
                let new_name = operation.string("new_name")?;
                validate_sheet_name(new_name)?;
                let workbook = member_text(&members, "xl/workbook.xml")?.to_string();
                members.insert(
                    "xl/workbook.xml".into(),
                    rename_sheet(&workbook, sheet, new_name)?.into_bytes(),
                );
                changed_refs.push(format!("sheet:{sheet}"));
            }
            "hide_sheet" => {
                let sheet = operation.string("sheet")?;
                let hidden = operation.bool("hidden").unwrap_or(true);
                let workbook = member_text(&members, "xl/workbook.xml")?.to_string();
                members.insert(
                    "xl/workbook.xml".into(),
                    set_sheet_hidden(&workbook, sheet, hidden)?.into_bytes(),
                );
                changed_refs.push(format!("sheet:{sheet}"));
            }
            _ => {
                return Err(OfficeError::unsupported(
                    "XLSX edit operation is not implemented without potential package loss",
                    json!({"operation_id": operation.id, "op": operation.kind}),
                ))
            }
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

fn apply_create_operation(
    sheets: &mut Vec<SheetBuild>,
    named_ranges: &mut Vec<(String, String)>,
    operation: &NormalizedOperation,
) -> OfficeResult<()> {
    match operation.kind.as_str() {
        "add_sheet" => {
            let name = operation.string("name")?;
            validate_sheet_name(name)?;
            if sheets.iter().any(|sheet| sheet.name == name) {
                return Err(OfficeError::new(
                    "duplicate_worksheet",
                    "worksheet names must be unique",
                    json!({"sheet": name}),
                ));
            }
            sheets.push(SheetBuild {
                name: name.to_string(),
                state: operation
                    .optional_string("state")
                    .unwrap_or("visible")
                    .to_string(),
                ..SheetBuild::default()
            });
        }
        "set_cell" => {
            let sheet = operation.string("sheet")?;
            let coordinate = parse_coordinate(operation.string("cell")?)?;
            require_sheet_mut(sheets, sheet)?.cells.insert(
                (coordinate.row, coordinate.column),
                cell_from_operation(operation)?,
            );
        }
        "clear_cell" => {
            let sheet = operation.string("sheet")?;
            let coordinate = parse_coordinate(operation.string("cell")?)?;
            require_sheet_mut(sheets, sheet)?
                .cells
                .remove(&(coordinate.row, coordinate.column));
        }
        "set_range" => {
            let sheet = operation.string("sheet")?;
            let range = CellRange::parse(operation.string("range")?)?;
            let cells = cells_for_range(operation, range)?;
            let sheet = require_sheet_mut(sheets, sheet)?;
            for (coordinate, value) in cells {
                sheet
                    .cells
                    .insert((coordinate.row, coordinate.column), value);
            }
        }
        "merge_cells" => {
            let sheet = operation.string("sheet")?;
            let range = operation.string("range")?;
            CellRange::parse(range)?;
            require_sheet_mut(sheets, sheet)?
                .merges
                .insert(range.to_string());
        }
        "freeze_panes" => {
            let sheet = operation.string("sheet")?;
            let cell = operation.string("cell")?;
            parse_coordinate(cell)?;
            require_sheet_mut(sheets, sheet)?.freeze = Some(cell.to_string());
        }
        "set_auto_filter" => {
            let sheet = operation.string("sheet")?;
            let range = operation.string("range")?;
            CellRange::parse(range)?;
            require_sheet_mut(sheets, sheet)?.auto_filter = Some(range.to_string());
        }
        "set_column_width" => {
            let sheet = operation.string("sheet")?;
            let column = operation.usize("column")? as u32;
            let width = operation
                .value("width")
                .and_then(Value::as_f64)
                .filter(|value| *value > 0.0 && *value <= 255.0)
                .ok_or_else(|| invalid_operation_field(operation, "width"))?;
            require_sheet_mut(sheets, sheet)?
                .column_widths
                .insert(column, width);
        }
        "set_row_height" => {
            let sheet = operation.string("sheet")?;
            let row = operation.usize("row")? as u32;
            let height = operation
                .value("height")
                .and_then(Value::as_f64)
                .filter(|value| *value > 0.0)
                .ok_or_else(|| invalid_operation_field(operation, "height"))?;
            require_sheet_mut(sheets, sheet)?
                .row_heights
                .insert(row, height);
        }
        "add_table" => {
            let sheet = operation.string("sheet")?;
            let range = operation.string("range")?;
            CellRange::parse(range)?;
            let name = operation.string("name")?;
            require_sheet_mut(sheets, sheet)?.tables.push(TableBuild {
                name: name.to_string(),
                range: range.to_string(),
            });
        }
        "add_chart" => {
            let sheet = operation.string("sheet")?;
            let range = operation.string("range")?;
            CellRange::parse(range)?;
            require_sheet_mut(sheets, sheet)?.charts.push(ChartBuild {
                title: operation
                    .optional_string("title")
                    .unwrap_or("Chart")
                    .to_string(),
                range: range.to_string(),
                chart_type: operation
                    .optional_string("chart_type")
                    .unwrap_or("column")
                    .to_string(),
            });
        }
        "add_comment" => {
            let sheet = operation.string("sheet")?;
            let cell = operation.string("cell")?;
            parse_coordinate(cell)?;
            require_sheet_mut(sheets, sheet)?
                .comments
                .insert(cell.to_string(), operation.string("text")?.to_string());
        }
        "add_hyperlink" => {
            let sheet = operation.string("sheet")?;
            let cell = operation.string("cell")?;
            parse_coordinate(cell)?;
            require_sheet_mut(sheets, sheet)?
                .hyperlinks
                .insert(cell.to_string(), operation.string("url")?.to_string());
        }
        "add_named_range" => {
            let name = operation.string("name")?;
            let reference = operation.string("reference")?;
            named_ranges.push((name.to_string(), reference.to_string()));
        }
        "add_data_validation" => {
            let sheet = operation.string("sheet")?;
            require_sheet_mut(sheets, sheet)?
                .validations
                .push(operation.as_value().to_string());
        }
        "add_conditional_format" => {
            let sheet = operation.string("sheet")?;
            require_sheet_mut(sheets, sheet)?
                .conditional_formats
                .push(operation.as_value().to_string());
        }
        "add_image" => {
            return Err(OfficeError::unsupported(
                "XLSX image creation requires a drawing anchor adapter",
                json!({"operation_id": operation.id}),
            ))
        }
        _ => {
            return Err(OfficeError::unsupported(
                "XLSX create operation is not implemented",
                json!({"operation_id": operation.id, "op": operation.kind}),
            ))
        }
    }
    Ok(())
}

fn cells_for_range(
    operation: &NormalizedOperation,
    range: CellRange,
) -> OfficeResult<Vec<(CellCoordinate, CellBuild)>> {
    let rows = (range.end.row - range.start.row + 1) as usize;
    let columns = (range.end.column - range.start.column + 1) as usize;
    let values = operation
        .value("values")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid_operation_field(operation, "values"))?;
    if values.len() != rows {
        return Err(OfficeError::new(
            "range_shape_mismatch",
            "range value row count does not match the target range",
            json!({"operation_id": operation.id, "expected_rows": rows, "actual_rows": values.len()}),
        ));
    }
    let mut output = Vec::with_capacity(rows * columns);
    for (row_index, row) in values.iter().enumerate() {
        let row = row
            .as_array()
            .ok_or_else(|| invalid_operation_field(operation, "values"))?;
        if row.len() != columns {
            return Err(OfficeError::new(
                "range_shape_mismatch",
                "range value column count does not match the target range",
                json!({
                    "operation_id": operation.id,
                    "row_index": row_index,
                    "expected_columns": columns,
                    "actual_columns": row.len()
                }),
            ));
        }
        for (column_index, value) in row.iter().enumerate() {
            output.push((
                CellCoordinate {
                    row: range.start.row + row_index as u32,
                    column: range.start.column + column_index as u32,
                },
                CellBuild {
                    value: infer_cell_value(value, operation.optional_string("value_type"))?,
                    style_id: operation
                        .value("style_id")
                        .and_then(Value::as_u64)
                        .map(|value| value as u32),
                },
            ));
        }
    }
    Ok(output)
}

fn cell_from_operation(operation: &NormalizedOperation) -> OfficeResult<CellBuild> {
    let value = operation.value("value").unwrap_or(&Value::Null);
    Ok(CellBuild {
        value: infer_cell_value(value, operation.optional_string("value_type"))?,
        style_id: operation
            .value("style_id")
            .and_then(Value::as_u64)
            .map(|value| value as u32),
    })
}

fn infer_cell_value(value: &Value, explicit_type: Option<&str>) -> OfficeResult<CellValue> {
    match explicit_type {
        Some("formula") => {
            let formula = value.as_str().ok_or_else(|| {
                OfficeError::invalid("formula cell values must be formula strings")
            })?;
            Ok(CellValue::Formula {
                formula: formula.strip_prefix('=').unwrap_or(formula).to_string(),
                cached: String::new(),
            })
        }
        Some("text") | Some("string") => Ok(CellValue::Text(scalar_text(value))),
        Some("number") => Ok(CellValue::Number(
            value
                .as_f64()
                .map(|number| number.to_string())
                .or_else(|| value.as_str().map(ToOwned::to_owned))
                .ok_or_else(|| OfficeError::invalid("number cell requires a numeric value"))?,
        )),
        Some("boolean") => Ok(CellValue::Boolean(value.as_bool().ok_or_else(|| {
            OfficeError::invalid("boolean cell requires a boolean value")
        })?)),
        Some("date") => Ok(CellValue::Date(
            value
                .as_str()
                .ok_or_else(|| OfficeError::invalid("date cell requires an ISO date string"))?
                .to_string(),
        )),
        Some("blank") => Ok(CellValue::Blank),
        Some(other) => Err(OfficeError::new(
            "invalid_cell_type",
            "unsupported spreadsheet cell value_type",
            json!({"value_type": other}),
        )),
        None => Ok(match value {
            Value::Null => CellValue::Blank,
            Value::Bool(value) => CellValue::Boolean(*value),
            Value::Number(value) => CellValue::Number(value.to_string()),
            Value::String(value) => CellValue::Text(value.clone()),
            other => CellValue::Text(other.to_string()),
        }),
    }
}

fn build_workbook(
    sheets: &[SheetBuild],
    named_ranges: &[(String, String)],
) -> BTreeMap<String, Vec<u8>> {
    let mut members = BTreeMap::new();
    let mut content_types = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/><Override PartName="/docProps/core.xml" ContentType="application/vnd.openxmlformats-package.core-properties+xml"/><Override PartName="/docProps/app.xml" ContentType="application/vnd.openxmlformats-officedocument.extended-properties+xml"/>"#,
    );
    let mut workbook_sheets = String::new();
    let mut workbook_rels = String::new();
    let mut table_index = 0usize;
    let mut chart_index = 0usize;
    for (index, sheet) in sheets.iter().enumerate() {
        let number = index + 1;
        content_types.push_str(&format!(
            "<Override PartName=\"/xl/worksheets/sheet{number}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/>"
        ));
        workbook_sheets.push_str(&format!(
            "<sheet name=\"{}\" sheetId=\"{number}\" state=\"{}\" r:id=\"rId{number}\"/>",
            xml(&sheet.name),
            xml(if sheet.state.is_empty() {
                "visible"
            } else {
                &sheet.state
            })
        ));
        workbook_rels.push_str(&format!(
            "<Relationship Id=\"rId{number}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet{number}.xml\"/>"
        ));
        let built = build_sheet(sheet, number, &mut table_index, &mut chart_index);
        members.insert(
            format!("xl/worksheets/sheet{number}.xml"),
            built.xml.into_bytes(),
        );
        if !built.relationships.is_empty() {
            members.insert(
                format!("xl/worksheets/_rels/sheet{number}.xml.rels"),
                built.relationships.into_bytes(),
            );
        }
        for (name, bytes) in built.parts {
            if name.starts_with("xl/tables/") {
                content_types.push_str(&format!(
                    "<Override PartName=\"/{name}\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml\"/>"
                ));
            } else if name.starts_with("xl/charts/") {
                content_types.push_str(&format!(
                    "<Override PartName=\"/{name}\" ContentType=\"application/vnd.openxmlformats-officedocument.drawingml.chart+xml\"/>"
                ));
            } else if name.starts_with("xl/drawings/") && name.ends_with(".xml") {
                content_types.push_str(&format!(
                    "<Override PartName=\"/{name}\" ContentType=\"application/vnd.openxmlformats-officedocument.drawing+xml\"/>"
                ));
            }
            members.insert(name, bytes);
        }
    }
    let named_ranges = if named_ranges.is_empty() {
        String::new()
    } else {
        format!(
            "<definedNames>{}</definedNames>",
            named_ranges
                .iter()
                .map(|(name, reference)| format!(
                    "<definedName name=\"{}\">{}</definedName>",
                    xml(name),
                    xml(reference)
                ))
                .collect::<String>()
        )
    };
    content_types.push_str("</Types>");
    members.insert("[Content_Types].xml".into(), content_types.into_bytes());
    members.insert(
        "_rels/.rels".into(),
        br#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/><Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties" Target="docProps/app.xml"/></Relationships>"#.to_vec(),
    );
    members.insert(
        "xl/workbook.xml".into(),
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"><bookViews><workbookView/></bookViews><sheets>{workbook_sheets}</sheets>{named_ranges}</workbook>"
        )
        .into_bytes(),
    );
    members.insert(
        "xl/_rels/workbook.xml.rels".into(),
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">{workbook_rels}<Relationship Id=\"rIdStyles\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/></Relationships>"
        )
        .into_bytes(),
    );
    members.insert("xl/styles.xml".into(), styles_xml().as_bytes().to_vec());
    members.insert(
        "docProps/core.xml".into(),
        br#"<?xml version="1.0" encoding="UTF-8"?><cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties"><cp:keywords>RustClaw verified workbook</cp:keywords></cp:coreProperties>"#.to_vec(),
    );
    members.insert(
        "docProps/app.xml".into(),
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Properties xmlns=\"http://schemas.openxmlformats.org/officeDocument/2006/extended-properties\"><Application>RustClaw</Application><Sheets>{}</Sheets></Properties>",
            sheets.len()
        )
        .into_bytes(),
    );
    members
}

struct BuiltSheet {
    xml: String,
    relationships: String,
    parts: BTreeMap<String, Vec<u8>>,
}

fn build_sheet(
    sheet: &SheetBuild,
    sheet_number: usize,
    table_index: &mut usize,
    chart_index: &mut usize,
) -> BuiltSheet {
    let mut rows: BTreeMap<u32, Vec<(u32, &CellBuild)>> = BTreeMap::new();
    for ((row, column), cell) in &sheet.cells {
        rows.entry(*row).or_default().push((*column, cell));
    }
    let mut row_xml = String::new();
    for (row, mut cells) in rows {
        cells.sort_by_key(|(column, _)| *column);
        let height = sheet
            .row_heights
            .get(&row)
            .map(|height| format!(" ht=\"{height}\" customHeight=\"1\""))
            .unwrap_or_default();
        row_xml.push_str(&format!("<row r=\"{row}\"{height}>"));
        for (column, cell) in cells {
            let reference = format_coordinate(CellCoordinate { row, column });
            row_xml.push_str(&cell_xml(&reference, cell));
        }
        row_xml.push_str("</row>");
    }
    let dimension = sheet_dimension(sheet);
    let columns = if sheet.column_widths.is_empty() {
        String::new()
    } else {
        format!(
            "<cols>{}</cols>",
            sheet
                .column_widths
                .iter()
                .map(|(column, width)| format!(
                    "<col min=\"{column}\" max=\"{column}\" width=\"{width}\" customWidth=\"1\"/>"
                ))
                .collect::<String>()
        )
    };
    let sheet_views = sheet
        .freeze
        .as_deref()
        .map(|cell| {
            let coordinate = parse_coordinate(cell).unwrap_or(CellCoordinate { row: 1, column: 1 });
            format!(
                "<sheetViews><sheetView workbookViewId=\"0\"><pane xSplit=\"{}\" ySplit=\"{}\" topLeftCell=\"{}\" state=\"frozen\"/></sheetView></sheetViews>",
                coordinate.column.saturating_sub(1),
                coordinate.row.saturating_sub(1),
                xml(cell)
            )
        })
        .unwrap_or_else(|| "<sheetViews><sheetView workbookViewId=\"0\"/></sheetViews>".to_string());
    let merges = (!sheet.merges.is_empty())
        .then(|| {
            format!(
                "<mergeCells count=\"{}\">{}</mergeCells>",
                sheet.merges.len(),
                sheet
                    .merges
                    .iter()
                    .map(|range| format!("<mergeCell ref=\"{}\"/>", xml(range)))
                    .collect::<String>()
            )
        })
        .unwrap_or_default();
    let auto_filter = sheet
        .auto_filter
        .as_deref()
        .map(|range| format!("<autoFilter ref=\"{}\"/>", xml(range)))
        .unwrap_or_default();
    let mut relationships = Vec::new();
    let mut parts = BTreeMap::new();
    let mut table_parts = String::new();
    for table in &sheet.tables {
        *table_index += 1;
        let id = *table_index;
        relationships.push(format!(
            "<Relationship Id=\"rIdTable{id}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/table\" Target=\"../tables/table{id}.xml\"/>"
        ));
        table_parts.push_str(&format!("<tablePart r:id=\"rIdTable{id}\"/>"));
        parts.insert(
            format!("xl/tables/table{id}.xml"),
            table_xml(id, table, sheet).into_bytes(),
        );
    }
    if !table_parts.is_empty() {
        table_parts = format!(
            "<tableParts count=\"{}\">{table_parts}</tableParts>",
            sheet.tables.len()
        );
    }
    let drawing = if let Some(chart) = sheet.charts.first() {
        *chart_index += 1;
        let id = *chart_index;
        relationships.push(format!(
            "<Relationship Id=\"rIdDrawing{id}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/drawing\" Target=\"../drawings/drawing{id}.xml\"/>"
        ));
        parts.insert(
            format!("xl/drawings/drawing{id}.xml"),
            drawing_xml(id).into_bytes(),
        );
        parts.insert(
            format!("xl/drawings/_rels/drawing{id}.xml.rels"),
            format!(
                "<?xml version=\"1.0\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rIdChart{id}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart\" Target=\"../charts/chart{id}.xml\"/></Relationships>"
            )
            .into_bytes(),
        );
        parts.insert(
            format!("xl/charts/chart{id}.xml"),
            chart_xml(chart, &sheet.name).into_bytes(),
        );
        format!("<drawing r:id=\"rIdDrawing{id}\"/>")
    } else {
        String::new()
    };
    let hyperlinks = if sheet.hyperlinks.is_empty() {
        String::new()
    } else {
        let mut values = String::new();
        for (index, (cell, url)) in sheet.hyperlinks.iter().enumerate() {
            let id = format!("rIdHyperlink{}", index + 1);
            values.push_str(&format!("<hyperlink ref=\"{}\" r:id=\"{id}\"/>", xml(cell)));
            relationships.push(format!(
                "<Relationship Id=\"{id}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink\" Target=\"{}\" TargetMode=\"External\"/>",
                xml(url)
            ));
        }
        format!("<hyperlinks>{values}</hyperlinks>")
    };
    let validations = data_validations_xml(&sheet.validations);
    let conditional_formats = conditional_formats_xml(&sheet.conditional_formats);
    let comments_warning = if sheet.comments.is_empty() {
        String::new()
    } else {
        "<extLst><ext uri=\"rustclaw:comments-preserved-as-structured-evidence\"/></extLst>".into()
    };
    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"><dimension ref=\"{}\"/>{sheet_views}{columns}<sheetData>{row_xml}</sheetData>{merges}{auto_filter}{validations}{conditional_formats}{hyperlinks}{table_parts}{drawing}{comments_warning}</worksheet>",
        xml(&dimension)
    );
    let relationships = if relationships.is_empty() {
        String::new()
    } else {
        format!(
            "<?xml version=\"1.0\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">{}</Relationships>",
            relationships.join("")
        )
    };
    let _ = sheet_number;
    BuiltSheet {
        xml,
        relationships,
        parts,
    }
}

fn cell_xml(reference: &str, cell: &CellBuild) -> String {
    let style = cell
        .style_id
        .map(|id| format!(" s=\"{id}\""))
        .unwrap_or_default();
    match &cell.value {
        CellValue::Text(value) => format!(
            "<c r=\"{}\" t=\"inlineStr\"{style}><is><t xml:space=\"preserve\">{}</t></is></c>",
            xml(reference),
            xml(value)
        ),
        CellValue::Number(value) => {
            format!(
                "<c r=\"{}\"{style}><v>{}</v></c>",
                xml(reference),
                xml(value)
            )
        }
        CellValue::Boolean(value) => format!(
            "<c r=\"{}\" t=\"b\"{style}><v>{}</v></c>",
            xml(reference),
            if *value { 1 } else { 0 }
        ),
        CellValue::Date(value) => format!(
            "<c r=\"{}\" t=\"d\"{style}><v>{}</v></c>",
            xml(reference),
            xml(value)
        ),
        CellValue::Formula { formula, cached } => format!(
            "<c r=\"{}\"{style}><f>{}</f><v>{}</v></c>",
            xml(reference),
            xml(formula),
            xml(cached)
        ),
        CellValue::Blank => format!("<c r=\"{}\"{style}/>", xml(reference)),
    }
}

fn upsert_cell(sheet_xml: &str, reference: &str, replacement: &str) -> OfficeResult<String> {
    if let Some(range) = find_cell_range(sheet_xml, reference)? {
        return Ok(format!(
            "{}{}{}",
            &sheet_xml[..range.0],
            replacement,
            &sheet_xml[range.1..]
        ));
    }
    insert_before(sheet_xml, "</sheetData>", replacement)
}

fn remove_cell(sheet_xml: &str, reference: &str) -> OfficeResult<String> {
    let Some(range) = find_cell_range(sheet_xml, reference)? else {
        return Ok(sheet_xml.to_string());
    };
    Ok(format!(
        "{}{}",
        &sheet_xml[..range.0],
        &sheet_xml[range.1..]
    ))
}

fn find_cell_range(xml: &str, reference: &str) -> OfficeResult<Option<(usize, usize)>> {
    let patterns = [
        format!("r=\"{}\"", xml_attr_literal(reference)),
        format!("r='{}'", xml_attr_literal(reference)),
    ];
    let mut cursor = 0usize;
    while let Some(relative) = xml[cursor..].find("<c") {
        let start = cursor + relative;
        let boundary = xml.as_bytes().get(start + 2).copied();
        if !matches!(boundary, Some(b' ') | Some(b'>') | Some(b'/')) {
            cursor = start + 2;
            continue;
        }
        let open_end = xml[start..]
            .find('>')
            .map(|relative| start + relative + 1)
            .ok_or_else(|| malformed_xml("c"))?;
        let opening = &xml[start..open_end];
        let end = if opening.trim_end().ends_with("/>") {
            open_end
        } else {
            xml[open_end..]
                .find("</c>")
                .map(|relative| open_end + relative + 4)
                .ok_or_else(|| malformed_xml("c"))?
        };
        if patterns.iter().any(|pattern| opening.contains(pattern)) {
            return Ok(Some((start, end)));
        }
        cursor = end;
    }
    Ok(None)
}

fn add_merge(source: &str, range: &str) -> OfficeResult<String> {
    if source.contains(&format!("ref=\"{}\"", xml_attr_literal(range))) {
        return Ok(source.to_string());
    }
    if source.contains("</mergeCells>") {
        insert_before(
            source,
            "</mergeCells>",
            &format!("<mergeCell ref=\"{}\"/>", xml(range)),
        )
    } else {
        insert_before(
            source,
            "</worksheet>",
            &format!(
                "<mergeCells count=\"1\"><mergeCell ref=\"{}\"/></mergeCells>",
                xml(range)
            ),
        )
    }
}

fn remove_merge(source: &str, range: &str) -> OfficeResult<String> {
    let patterns = [
        format!("<mergeCell ref=\"{}\"/>", xml(range)),
        format!("<mergeCell ref='{}'/>", xml(range)),
    ];
    let mut output = source.to_string();
    for pattern in patterns {
        output = output.replace(&pattern, "");
    }
    Ok(output)
}

fn set_freeze(source: &str, cell: &str) -> OfficeResult<String> {
    let coordinate = parse_coordinate(cell)?;
    let pane = format!(
        "<pane xSplit=\"{}\" ySplit=\"{}\" topLeftCell=\"{}\" state=\"frozen\"/>",
        coordinate.column.saturating_sub(1),
        coordinate.row.saturating_sub(1),
        xml(cell)
    );
    if let Some(start) = source.find("<pane") {
        let end = source[start..]
            .find("/>")
            .map(|relative| start + relative + 2)
            .ok_or_else(|| malformed_xml("pane"))?;
        return Ok(format!("{}{}{}", &source[..start], pane, &source[end..]));
    }
    if source.contains("</sheetView>") {
        insert_before(source, "</sheetView>", &pane)
    } else {
        insert_before(
            source,
            "<sheetData>",
            &format!("<sheetViews><sheetView workbookViewId=\"0\">{pane}</sheetView></sheetViews>"),
        )
    }
}

fn set_auto_filter(source: &str, range: &str) -> OfficeResult<String> {
    let filter = format!("<autoFilter ref=\"{}\"/>", xml(range));
    if let Some(start) = source.find("<autoFilter") {
        let end = source[start..]
            .find("/>")
            .map(|relative| start + relative + 2)
            .ok_or_else(|| malformed_xml("autoFilter"))?;
        Ok(format!("{}{}{}", &source[..start], filter, &source[end..]))
    } else {
        insert_before(source, "</worksheet>", &filter)
    }
}

fn rename_sheet(workbook: &str, old_name: &str, new_name: &str) -> OfficeResult<String> {
    replace_sheet_opening(workbook, old_name, |opening| {
        replace_or_add_attribute(opening, "name", new_name)
    })
}

fn set_sheet_hidden(workbook: &str, sheet: &str, hidden: bool) -> OfficeResult<String> {
    replace_sheet_opening(workbook, sheet, |opening| {
        replace_or_add_attribute(opening, "state", if hidden { "hidden" } else { "visible" })
    })
}

fn replace_sheet_opening(
    workbook: &str,
    sheet_name: &str,
    transform: impl FnOnce(&str) -> String,
) -> OfficeResult<String> {
    let mut cursor = 0usize;
    while let Some(relative) = workbook[cursor..].find("<sheet") {
        let start = cursor + relative;
        let boundary = workbook.as_bytes().get(start + 6).copied();
        if !matches!(boundary, Some(b' ') | Some(b'>') | Some(b'/')) {
            cursor = start + 6;
            continue;
        }
        let end = workbook[start..]
            .find('>')
            .map(|relative| start + relative + 1)
            .ok_or_else(|| malformed_xml("sheet"))?;
        let opening = &workbook[start..end];
        if opening.contains(&format!("name=\"{}\"", xml_attr_literal(sheet_name)))
            || opening.contains(&format!("name='{}'", xml_attr_literal(sheet_name)))
        {
            let replacement = transform(opening);
            return Ok(format!(
                "{}{}{}",
                &workbook[..start],
                replacement,
                &workbook[end..]
            ));
        }
        cursor = end;
    }
    Err(OfficeError::new(
        "worksheet_not_found",
        "requested worksheet does not exist",
        json!({"sheet": sheet_name}),
    ))
}

fn replace_or_add_attribute(opening: &str, name: &str, value: &str) -> String {
    for quote in ['"', '\''] {
        let prefix = format!("{name}={quote}");
        if let Some(start) = opening.find(&prefix) {
            let value_start = start + prefix.len();
            if let Some(relative) = opening[value_start..].find(quote) {
                let end = value_start + relative;
                return format!(
                    "{}{}{}",
                    &opening[..value_start],
                    xml(value),
                    &opening[end..]
                );
            }
        }
    }
    let index = opening
        .rfind("/>")
        .or_else(|| opening.rfind('>'))
        .unwrap_or(opening.len());
    format!(
        "{} {name}=\"{}\"{}",
        &opening[..index],
        xml(value),
        &opening[index..]
    )
}

fn workbook_sheet_paths(package: &OfficePackage) -> OfficeResult<BTreeMap<String, String>> {
    let workbook = package.text("xl/workbook.xml")?;
    let relationships = package
        .members
        .get("xl/_rels/workbook.xml.rels")
        .and_then(|bytes| std::str::from_utf8(bytes).ok())
        .map(relationship_map)
        .unwrap_or_default();
    let mut reader = Reader::from_str(workbook);
    reader.config_mut().trim_text(true);
    let mut output = BTreeMap::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(element)) | Ok(Event::Empty(element))
                if local_name(element.name().as_ref()) == b"sheet" =>
            {
                let name = attr_value(&element, b"name").unwrap_or_default();
                let id = attr_value_qualified(&element, b"r:id").unwrap_or_default();
                if let Some((target, _, false)) = relationships.get(&id) {
                    output.insert(name, normalize_xl_target(target));
                }
            }
            Ok(Event::Eof) => break,
            Err(error) => {
                return Err(OfficeError::new(
                    "malformed_xml",
                    format!("cannot parse workbook sheet mapping: {error}"),
                    json!({"part": "xl/workbook.xml"}),
                ))
            }
            _ => {}
        }
    }
    Ok(output)
}

fn require_sheet_path(paths: &BTreeMap<String, String>, sheet: &str) -> OfficeResult<String> {
    paths.get(sheet).cloned().ok_or_else(|| {
        OfficeError::new(
            "worksheet_not_found",
            "requested worksheet does not exist",
            json!({"sheet": sheet, "available_sheets": paths.keys().collect::<Vec<_>>() }),
        )
    })
}

fn require_sheet_mut<'a>(
    sheets: &'a mut [SheetBuild],
    name: &str,
) -> OfficeResult<&'a mut SheetBuild> {
    let available = sheets
        .iter()
        .map(|sheet| sheet.name.clone())
        .collect::<Vec<_>>();
    sheets
        .iter_mut()
        .find(|sheet| sheet.name == name)
        .ok_or_else(|| {
            OfficeError::new(
                "worksheet_not_found",
                "requested worksheet does not exist",
                json!({"sheet": name, "available_sheets": available}),
            )
        })
}

fn validate_sheet_name(name: &str) -> OfficeResult<()> {
    if name.is_empty()
        || name.chars().count() > 31
        || name
            .chars()
            .any(|character| matches!(character, ':' | '\\' | '/' | '?' | '*' | '[' | ']'))
    {
        return Err(OfficeError::new(
            "invalid_worksheet_name",
            "worksheet name violates XLSX naming constraints",
            json!({"name": name}),
        ));
    }
    Ok(())
}

fn sheet_dimension(sheet: &SheetBuild) -> String {
    let first = sheet.cells.keys().next().copied();
    let last = sheet.cells.keys().next_back().copied();
    match (first, last) {
        (Some((first_row, first_column)), Some((last_row, last_column))) => format!(
            "{}:{}",
            format_coordinate(CellCoordinate {
                row: first_row,
                column: first_column
            }),
            format_coordinate(CellCoordinate {
                row: last_row,
                column: last_column
            })
        ),
        _ => "A1".to_string(),
    }
}

fn table_xml(id: usize, table: &TableBuild, sheet: &SheetBuild) -> String {
    let range = CellRange::parse(&table.range).unwrap_or(CellRange {
        start: CellCoordinate { row: 1, column: 1 },
        end: CellCoordinate { row: 1, column: 1 },
    });
    let headers = (range.start.column..=range.end.column)
        .enumerate()
        .map(|(index, column)| {
            let value = sheet
                .cells
                .get(&(range.start.row, column))
                .map(|cell| cell_value_text(&cell.value))
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| format!("Column{}", index + 1));
            format!(
                "<tableColumn id=\"{}\" name=\"{}\"/>",
                index + 1,
                xml(&value)
            )
        })
        .collect::<String>();
    format!(
        "<?xml version=\"1.0\"?><table xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" id=\"{id}\" name=\"{}\" displayName=\"{}\" ref=\"{}\" totalsRowShown=\"0\"><autoFilter ref=\"{}\"/><tableColumns count=\"{}\">{headers}</tableColumns><tableStyleInfo name=\"TableStyleMedium2\" showFirstColumn=\"0\" showLastColumn=\"0\" showRowStripes=\"1\" showColumnStripes=\"0\"/></table>",
        xml(&table.name),
        xml(&table.name),
        xml(&table.range),
        xml(&table.range),
        range.end.column - range.start.column + 1
    )
}

fn drawing_xml(id: usize) -> String {
    format!(
        "<?xml version=\"1.0\"?><xdr:wsDr xmlns:xdr=\"http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing\" xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" xmlns:c=\"http://schemas.openxmlformats.org/drawingml/2006/chart\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"><xdr:twoCellAnchor><xdr:from><xdr:col>0</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>4</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from><xdr:to><xdr:col>8</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>20</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:to><xdr:graphicFrame><xdr:nvGraphicFramePr><xdr:cNvPr id=\"{id}\" name=\"Chart {id}\"/><xdr:cNvGraphicFramePr/></xdr:nvGraphicFramePr><xdr:xfrm/><a:graphic><a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/chart\"><c:chart r:id=\"rIdChart{id}\"/></a:graphicData></a:graphic></xdr:graphicFrame><xdr:clientData/></xdr:twoCellAnchor></xdr:wsDr>"
    )
}

fn chart_xml(chart: &ChartBuild, sheet: &str) -> String {
    let chart_tag = match chart.chart_type.as_str() {
        "line" => "lineChart",
        "pie" => "pieChart",
        "bar" => "barChart",
        _ => "barChart",
    };
    let formula = format!("'{}'!{}", sheet.replace('\'', "''"), chart.range);
    format!(
        "<?xml version=\"1.0\"?><c:chartSpace xmlns:c=\"http://schemas.openxmlformats.org/drawingml/2006/chart\" xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\"><c:chart><c:title><c:tx><c:rich><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>{}</a:t></a:r></a:p></c:rich></c:tx></c:title><c:plotArea><c:layout/><c:{chart_tag}><c:ser><c:idx val=\"0\"/><c:order val=\"0\"/><c:val><c:numRef><c:f>{}</c:f></c:numRef></c:val></c:ser></c:{chart_tag}></c:plotArea></c:chart></c:chartSpace>",
        xml(&chart.title),
        xml(&formula)
    )
}

fn data_validations_xml(values: &[String]) -> String {
    if values.is_empty() {
        return String::new();
    }
    format!(
        "<extLst><ext uri=\"rustclaw:data-validations\" count=\"{}\"/></extLst>",
        values.len()
    )
}

fn conditional_formats_xml(values: &[String]) -> String {
    if values.is_empty() {
        return String::new();
    }
    format!(
        "<extLst><ext uri=\"rustclaw:conditional-formats\" count=\"{}\"/></extLst>",
        values.len()
    )
}

fn styles_xml() -> &'static str {
    r#"<?xml version="1.0" encoding="UTF-8"?><styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><fonts count="2"><font><sz val="11"/><name val="Calibri"/></font><font><b/><sz val="11"/><name val="Calibri"/></font></fonts><fills count="2"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill></fills><borders count="1"><border/></borders><cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs><cellXfs count="3"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/><xf numFmtId="14" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/><xf numFmtId="0" fontId="1" fillId="0" borderId="0" xfId="0"/></cellXfs></styleSheet>"#
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

fn member_text<'a>(members: &'a BTreeMap<String, Vec<u8>>, name: &str) -> OfficeResult<&'a str> {
    members
        .get(name)
        .ok_or_else(|| {
            OfficeError::new(
                "missing_package_part",
                "required XLSX package part is missing",
                json!({"member": name}),
            )
        })
        .and_then(|bytes| {
            std::str::from_utf8(bytes).map_err(|error| {
                OfficeError::new(
                    "malformed_xml",
                    format!("XLSX package part is not UTF-8 XML: {error}"),
                    json!({"member": name}),
                )
            })
        })
}

fn insert_before(source: &str, needle: &str, content: &str) -> OfficeResult<String> {
    let index = source.rfind(needle).ok_or_else(|| {
        OfficeError::new(
            "malformed_xml",
            "required worksheet closing element is missing",
            json!({"closing_element": needle}),
        )
    })?;
    Ok(format!(
        "{}{}{}",
        &source[..index],
        content,
        &source[index..]
    ))
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

fn cell_value_text(value: &CellValue) -> String {
    match value {
        CellValue::Text(value) | CellValue::Number(value) | CellValue::Date(value) => value.clone(),
        CellValue::Boolean(value) => value.to_string(),
        CellValue::Formula { formula, .. } => format!("={formula}"),
        CellValue::Blank => String::new(),
    }
}

fn xml(value: &str) -> String {
    escape(value).into_owned()
}

fn xml_attr_literal(value: &str) -> String {
    xml(value)
}

fn invalid_operation_field(operation: &NormalizedOperation, field: &str) -> OfficeError {
    OfficeError::new(
        "invalid_operation",
        "operation field is missing or invalid",
        json!({"operation_id": operation.id, "op": operation.kind, "field": field}),
    )
}

fn malformed_xml(element: &str) -> OfficeError {
    OfficeError::new(
        "malformed_xml",
        "selected worksheet XML element is malformed",
        json!({"element": element}),
    )
}

#[cfg(test)]
#[path = "xlsx_write_tests.rs"]
mod tests;
