use crate::error::{OfficeError, OfficeResult};
use crate::operations::NormalizedOperation;
use crate::range::{format_coordinate, parse_coordinate, CellCoordinate, CellRange};
use quick_xml::escape::escape;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;

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
pub(super) struct CellBuild {
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
    images: Vec<ImageBuild>,
    comments: BTreeMap<String, String>,
    hyperlinks: BTreeMap<String, String>,
    validations: Vec<Value>,
    conditional_formats: Vec<Value>,
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

#[derive(Clone, Debug)]
struct ImageBuild {
    path: String,
    cell: String,
    alt: String,
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
    let members = build_workbook(&sheets, &named_ranges)?;
    Ok(XlsxWriteResult {
        members,
        changed_refs: operations
            .iter()
            .flat_map(NormalizedOperation::object_refs)
            .collect(),
        preservation: vec!["new_package".to_string()],
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
                .push(operation.as_value());
        }
        "add_conditional_format" => {
            let sheet = operation.string("sheet")?;
            require_sheet_mut(sheets, sheet)?
                .conditional_formats
                .push(operation.as_value());
        }
        "add_image" => {
            let sheet = operation.string("sheet")?;
            let path = crate::package::resolve_input_path(operation.string("path")?)?;
            image_extension(&path)?;
            let cell = operation.optional_string("cell").unwrap_or("A1");
            parse_coordinate(cell)?;
            require_sheet_mut(sheets, sheet)?.images.push(ImageBuild {
                path: path.display().to_string(),
                cell: cell.to_string(),
                alt: operation
                    .optional_string("alt")
                    .unwrap_or("image")
                    .to_string(),
            });
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

pub(super) fn cells_for_range(
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

pub(super) fn cell_from_operation(operation: &NormalizedOperation) -> OfficeResult<CellBuild> {
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
) -> OfficeResult<BTreeMap<String, Vec<u8>>> {
    let mut members = BTreeMap::new();
    let mut content_types = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/><Override PartName="/docProps/core.xml" ContentType="application/vnd.openxmlformats-package.core-properties+xml"/><Override PartName="/docProps/app.xml" ContentType="application/vnd.openxmlformats-officedocument.extended-properties+xml"/>"#,
    );
    let mut workbook_sheets = String::new();
    let mut workbook_rels = String::new();
    let mut table_index = 0usize;
    let mut chart_index = 0usize;
    let mut image_index = 0usize;
    let mut image_extensions = BTreeSet::new();
    let mut has_vml = false;
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
        let built = build_sheet(
            sheet,
            number,
            &mut table_index,
            &mut chart_index,
            &mut image_index,
        )?;
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
            } else if name.starts_with("xl/comments") {
                content_types.push_str(&format!(
                    "<Override PartName=\"/{name}\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.comments+xml\"/>"
                ));
            } else if name.starts_with("xl/media/") {
                if let Some(extension) = std::path::Path::new(&name)
                    .extension()
                    .and_then(|value| value.to_str())
                {
                    image_extensions.insert(extension.to_string());
                }
            } else if name.ends_with(".vml") {
                has_vml = true;
            }
            members.insert(name, bytes);
        }
    }
    for extension in image_extensions {
        content_types.push_str(&format!(
            "<Default Extension=\"{}\" ContentType=\"{}\"/>",
            xml(&extension),
            image_content_type(&extension)
        ));
    }
    if has_vml {
        content_types.push_str(
            r#"<Default Extension="vml" ContentType="application/vnd.openxmlformats-officedocument.vmlDrawing"/>"#,
        );
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
    Ok(members)
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
    image_index: &mut usize,
) -> OfficeResult<BuiltSheet> {
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
    let drawing = if !sheet.charts.is_empty() || !sheet.images.is_empty() {
        let drawing_id = sheet_number;
        relationships.push(format!(
            "<Relationship Id=\"rIdDrawing{drawing_id}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/drawing\" Target=\"../drawings/drawing{drawing_id}.xml\"/>"
        ));
        let mut anchors = String::new();
        let mut drawing_relationships = String::new();
        for chart in &sheet.charts {
            *chart_index += 1;
            let chart_id = *chart_index;
            anchors.push_str(&chart_anchor(chart_id));
            drawing_relationships.push_str(&format!(
                "<Relationship Id=\"rIdChart{chart_id}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart\" Target=\"../charts/chart{chart_id}.xml\"/>"
            ));
            parts.insert(
                format!("xl/charts/chart{chart_id}.xml"),
                chart_xml(chart, &sheet.name).into_bytes(),
            );
        }
        for image in &sheet.images {
            *image_index += 1;
            let image_id = *image_index;
            let extension = image_extension(std::path::Path::new(&image.path))?;
            let bytes = fs::read(&image.path).map_err(|error| {
                OfficeError::new(
                    "source_unavailable",
                    format!("cannot read spreadsheet image: {error}"),
                    json!({"path": image.path}),
                )
            })?;
            parts.insert(format!("xl/media/image{image_id}.{extension}"), bytes);
            drawing_relationships.push_str(&format!(
                "<Relationship Id=\"rIdImage{image_id}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/image\" Target=\"../media/image{image_id}.{extension}\"/>"
            ));
            anchors.push_str(&image_anchor(image_id, image)?);
        }
        parts.insert(
            format!("xl/drawings/drawing{drawing_id}.xml"),
            drawing_xml(&anchors).into_bytes(),
        );
        parts.insert(
            format!("xl/drawings/_rels/drawing{drawing_id}.xml.rels"),
            format!(
                "<?xml version=\"1.0\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">{drawing_relationships}</Relationships>"
            )
            .into_bytes(),
        );
        format!("<drawing r:id=\"rIdDrawing{drawing_id}\"/>")
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
    let validations = data_validations_xml(&sheet.validations)?;
    let conditional_formats = conditional_formats_xml(&sheet.conditional_formats)?;
    let comments_parts = if sheet.comments.is_empty() {
        String::new()
    } else {
        relationships.push(format!(
            "<Relationship Id=\"rIdComments{sheet_number}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments\" Target=\"../comments{sheet_number}.xml\"/>"
        ));
        relationships.push(format!(
            "<Relationship Id=\"rIdVml{sheet_number}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/vmlDrawing\" Target=\"../drawings/vmlDrawing{sheet_number}.vml\"/>"
        ));
        parts.insert(
            format!("xl/comments{sheet_number}.xml"),
            comments_xml(&sheet.comments).into_bytes(),
        );
        parts.insert(
            format!("xl/drawings/vmlDrawing{sheet_number}.vml"),
            comments_vml(&sheet.comments).into_bytes(),
        );
        format!("<legacyDrawing r:id=\"rIdVml{sheet_number}\"/>")
    };
    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"><dimension ref=\"{}\"/>{sheet_views}{columns}<sheetData>{row_xml}</sheetData>{auto_filter}{merges}{conditional_formats}{validations}{hyperlinks}{drawing}{comments_parts}{table_parts}</worksheet>",
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
    Ok(BuiltSheet {
        xml,
        relationships,
        parts,
    })
}

pub(super) fn cell_xml(reference: &str, cell: &CellBuild) -> String {
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

pub(super) fn upsert_cell(
    sheet_xml: &str,
    reference: &str,
    replacement: &str,
) -> OfficeResult<String> {
    if let Some(range) = find_cell_range(sheet_xml, reference)? {
        return Ok(format!(
            "{}{}{}",
            &sheet_xml[..range.0],
            replacement,
            &sheet_xml[range.1..]
        ));
    }
    let coordinate = parse_coordinate(reference)?;
    if let Some((opening_start, opening_end, closing_start)) =
        find_row_range(sheet_xml, coordinate.row)?
    {
        let opening = &sheet_xml[opening_start..opening_end];
        if opening.trim_end().ends_with("/>") {
            let opening = opening.trim_end_matches("/>").trim_end();
            return Ok(format!(
                "{}{}>{}</row>{}",
                &sheet_xml[..opening_start],
                opening,
                replacement,
                &sheet_xml[opening_end..]
            ));
        }
        let closing_start = closing_start.ok_or_else(|| malformed_xml("row"))?;
        return Ok(format!(
            "{}{}{}",
            &sheet_xml[..closing_start],
            replacement,
            &sheet_xml[closing_start..]
        ));
    }
    insert_before(
        sheet_xml,
        "</sheetData>",
        &format!("<row r=\"{}\">{replacement}</row>", coordinate.row),
    )
}

pub(super) fn remove_cell(sheet_xml: &str, reference: &str) -> OfficeResult<String> {
    let Some(range) = find_cell_range(sheet_xml, reference)? else {
        return Ok(sheet_xml.to_string());
    };
    Ok(format!(
        "{}{}",
        &sheet_xml[..range.0],
        &sheet_xml[range.1..]
    ))
}

pub(super) fn find_cell_range(xml: &str, reference: &str) -> OfficeResult<Option<(usize, usize)>> {
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

fn find_row_range(xml: &str, row: u32) -> OfficeResult<Option<(usize, usize, Option<usize>)>> {
    let patterns = [format!("r=\"{row}\""), format!("r='{row}'")];
    let mut cursor = 0usize;
    while let Some(relative) = xml[cursor..].find("<row") {
        let start = cursor + relative;
        let boundary = xml.as_bytes().get(start + 4).copied();
        if !matches!(boundary, Some(b' ') | Some(b'>') | Some(b'/')) {
            cursor = start + 4;
            continue;
        }
        let opening_end = xml[start..]
            .find('>')
            .map(|relative| start + relative + 1)
            .ok_or_else(|| malformed_xml("row"))?;
        let opening = &xml[start..opening_end];
        if !patterns.iter().any(|pattern| opening.contains(pattern)) {
            cursor = opening_end;
            continue;
        }
        if opening.trim_end().ends_with("/>") {
            return Ok(Some((start, opening_end, None)));
        }
        let closing_start = xml[opening_end..]
            .find("</row>")
            .map(|relative| opening_end + relative)
            .ok_or_else(|| malformed_xml("row"))?;
        return Ok(Some((start, opening_end, Some(closing_start))));
    }
    Ok(None)
}

pub(super) fn add_merge(source: &str, range: &str) -> OfficeResult<String> {
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
        insert_before_first_of(
            source,
            &[
                "<phoneticPr",
                "<conditionalFormatting",
                "<dataValidations",
                "<hyperlinks",
                "<printOptions",
                "<pageMargins",
                "<drawing",
                "<legacyDrawing",
                "<tableParts",
                "</worksheet>",
            ],
            &format!(
                "<mergeCells count=\"1\"><mergeCell ref=\"{}\"/></mergeCells>",
                xml(range)
            ),
        )
    }
}

pub(super) fn remove_merge(source: &str, range: &str) -> OfficeResult<String> {
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

pub(super) fn set_freeze(source: &str, cell: &str) -> OfficeResult<String> {
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

pub(super) fn set_auto_filter(source: &str, range: &str) -> OfficeResult<String> {
    let filter = format!("<autoFilter ref=\"{}\"/>", xml(range));
    if let Some(start) = source.find("<autoFilter") {
        let end = source[start..]
            .find("/>")
            .map(|relative| start + relative + 2)
            .ok_or_else(|| malformed_xml("autoFilter"))?;
        Ok(format!("{}{}{}", &source[..start], filter, &source[end..]))
    } else {
        insert_before_first_of(
            source,
            &[
                "<mergeCells",
                "<phoneticPr",
                "<conditionalFormatting",
                "<dataValidations",
                "<hyperlinks",
                "<printOptions",
                "<pageMargins",
                "<drawing",
                "<legacyDrawing",
                "<tableParts",
                "</worksheet>",
            ],
            &filter,
        )
    }
}

pub(super) fn rename_sheet(workbook: &str, old_name: &str, new_name: &str) -> OfficeResult<String> {
    replace_sheet_opening(workbook, old_name, |opening| {
        replace_or_add_attribute(opening, "name", new_name)
    })
}

pub(super) fn set_sheet_hidden(workbook: &str, sheet: &str, hidden: bool) -> OfficeResult<String> {
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

pub(super) fn replace_or_add_attribute(opening: &str, name: &str, value: &str) -> String {
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

pub(super) fn validate_sheet_name(name: &str) -> OfficeResult<()> {
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

fn drawing_xml(anchors: &str) -> String {
    format!(
        "<?xml version=\"1.0\"?><xdr:wsDr xmlns:xdr=\"http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing\" xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" xmlns:c=\"http://schemas.openxmlformats.org/drawingml/2006/chart\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\">{anchors}</xdr:wsDr>"
    )
}

fn chart_anchor(id: usize) -> String {
    format!(
        "<xdr:twoCellAnchor><xdr:from><xdr:col>0</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>4</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from><xdr:to><xdr:col>8</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>20</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:to><xdr:graphicFrame><xdr:nvGraphicFramePr><xdr:cNvPr id=\"{id}\" name=\"Chart {id}\"/><xdr:cNvGraphicFramePr/></xdr:nvGraphicFramePr><xdr:xfrm/><a:graphic><a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/chart\"><c:chart r:id=\"rIdChart{id}\"/></a:graphicData></a:graphic></xdr:graphicFrame><xdr:clientData/></xdr:twoCellAnchor>"
    )
}

fn image_anchor(id: usize, image: &ImageBuild) -> OfficeResult<String> {
    let coordinate = parse_coordinate(&image.cell)?;
    let column = coordinate.column.saturating_sub(1);
    let row = coordinate.row.saturating_sub(1);
    Ok(format!(
        "<xdr:oneCellAnchor><xdr:from><xdr:col>{column}</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>{row}</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from><xdr:ext cx=\"2400000\" cy=\"1600000\"/><xdr:pic><xdr:nvPicPr><xdr:cNvPr id=\"{id}\" name=\"Image {id}\" descr=\"{}\"/><xdr:cNvPicPr/></xdr:nvPicPr><xdr:blipFill><a:blip r:embed=\"rIdImage{id}\"/><a:stretch><a:fillRect/></a:stretch></xdr:blipFill><xdr:spPr><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></xdr:spPr></xdr:pic><xdr:clientData/></xdr:oneCellAnchor>",
        xml(&image.alt)
    ))
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

fn data_validations_xml(values: &[Value]) -> OfficeResult<String> {
    if values.is_empty() {
        return Ok(String::new());
    }
    let mut rules = String::new();
    for value in values {
        let object = value
            .as_object()
            .ok_or_else(|| OfficeError::invalid("data validation operation must be an object"))?;
        let range = object
            .get("range")
            .and_then(Value::as_str)
            .ok_or_else(|| OfficeError::invalid("data validation requires range"))?;
        CellRange::parse(range)?;
        let kind = object
            .get("validation_type")
            .and_then(Value::as_str)
            .unwrap_or("list");
        let formula = object
            .get("formula1")
            .or_else(|| object.get("formula"))
            .map(scalar_text)
            .unwrap_or_default();
        let allow_blank = object
            .get("allow_blank")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        rules.push_str(&format!(
            "<dataValidation type=\"{}\" allowBlank=\"{}\" sqref=\"{}\"><formula1>{}</formula1></dataValidation>",
            xml(kind),
            if allow_blank { 1 } else { 0 },
            xml(range),
            xml(&formula)
        ));
    }
    Ok(format!(
        "<dataValidations count=\"{}\">{rules}</dataValidations>",
        values.len()
    ))
}

fn conditional_formats_xml(values: &[Value]) -> OfficeResult<String> {
    if values.is_empty() {
        return Ok(String::new());
    }
    values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let object = value.as_object().ok_or_else(|| {
                OfficeError::invalid("conditional format operation must be an object")
            })?;
            let range = object
                .get("range")
                .and_then(Value::as_str)
                .ok_or_else(|| OfficeError::invalid("conditional format requires range"))?;
            CellRange::parse(range)?;
            let formula = object
                .get("formula")
                .map(scalar_text)
                .unwrap_or_else(|| "TRUE".to_string());
            Ok(format!(
                "<conditionalFormatting sqref=\"{}\"><cfRule type=\"expression\" priority=\"{}\"><formula>{}</formula></cfRule></conditionalFormatting>",
                xml(range),
                index + 1,
                xml(&formula)
            ))
        })
        .collect()
}

fn comments_xml(comments: &BTreeMap<String, String>) -> String {
    let values = comments
        .iter()
        .map(|(cell, text)| {
            format!(
                "<comment ref=\"{}\" authorId=\"0\"><text><r><t xml:space=\"preserve\">{}</t></r></text></comment>",
                xml(cell),
                xml(text)
            )
        })
        .collect::<String>();
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><comments xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><authors><author>RustClaw</author></authors><commentList>{values}</commentList></comments>"
    )
}

fn comments_vml(comments: &BTreeMap<String, String>) -> String {
    let shapes = comments
        .keys()
        .enumerate()
        .filter_map(|(index, cell)| {
            let coordinate = parse_coordinate(cell).ok()?;
            let row = coordinate.row.saturating_sub(1);
            let column = coordinate.column.saturating_sub(1);
            let shape_id = 1025 + index;
            Some(format!(
                "<v:shape id=\"_x0000_s{shape_id}\" type=\"#_x0000_t202\" style=\"position:absolute;visibility:hidden\" fillcolor=\"#ffffe1\" o:insetmode=\"auto\"><v:fill color2=\"#ffffe1\"/><v:shadow on=\"t\" color=\"black\" obscured=\"t\"/><v:path o:connecttype=\"none\"/><v:textbox style=\"mso-direction-alt:auto\"><div style=\"text-align:left\"/></v:textbox><x:ClientData ObjectType=\"Note\"><x:MoveWithCells/><x:SizeWithCells/><x:Anchor>{column}, 15, {row}, 2, {}, 15, {}, 4</x:Anchor><x:AutoFill>False</x:AutoFill><x:Row>{row}</x:Row><x:Column>{column}</x:Column></x:ClientData></v:shape>",
                column + 3,
                row + 4
            ))
        })
        .collect::<String>();
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><xml xmlns:v=\"urn:schemas-microsoft-com:vml\" xmlns:o=\"urn:schemas-microsoft-com:office:office\" xmlns:x=\"urn:schemas-microsoft-com:office:excel\"><o:shapelayout v:ext=\"edit\"><o:idmap v:ext=\"edit\" data=\"1\"/></o:shapelayout><v:shapetype id=\"_x0000_t202\" coordsize=\"21600,21600\" o:spt=\"202\" path=\"m,l,21600r21600,l21600,xe\"><v:stroke joinstyle=\"miter\"/><v:path gradientshapeok=\"t\" o:connecttype=\"rect\"/></v:shapetype>{shapes}</xml>"
    )
}

fn image_extension(path: &std::path::Path) -> OfficeResult<&str> {
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

fn styles_xml() -> &'static str {
    r#"<?xml version="1.0" encoding="UTF-8"?><styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><fonts count="2"><font><sz val="11"/><name val="Calibri"/></font><font><b/><sz val="11"/><name val="Calibri"/></font></fonts><fills count="2"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill></fills><borders count="1"><border/></borders><cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs><cellXfs count="3"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/><xf numFmtId="14" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/><xf numFmtId="0" fontId="1" fillId="0" borderId="0" xfId="0"/></cellXfs></styleSheet>"#
}

pub(super) fn member_text<'a>(
    members: &'a BTreeMap<String, Vec<u8>>,
    name: &str,
) -> OfficeResult<&'a str> {
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

pub(super) fn insert_before(source: &str, needle: &str, content: &str) -> OfficeResult<String> {
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

fn insert_before_first_of(source: &str, anchors: &[&str], content: &str) -> OfficeResult<String> {
    let index = anchors
        .iter()
        .filter_map(|anchor| source.find(anchor))
        .min()
        .ok_or_else(|| malformed_xml("worksheet"))?;
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

pub(super) fn xml(value: &str) -> String {
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

pub(super) fn malformed_xml(element: &str) -> OfficeError {
    OfficeError::new(
        "malformed_xml",
        "selected worksheet XML element is malformed",
        json!({"element": element}),
    )
}

#[cfg(test)]
#[path = "xlsx_write_tests.rs"]
mod tests;
