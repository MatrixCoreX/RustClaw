use crate::error::{OfficeError, OfficeResult};
use crate::model::{CellEvidence, WorkbookEvidence, WorksheetEvidence};
use crate::package::OfficePackage;
use crate::xml::{attr_value, attr_value_qualified, collect_text, local_name, relationship_map};
use quick_xml::events::Event;
use quick_xml::Reader;
use serde_json::{json, Number, Value};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug)]
struct SheetRef {
    id: String,
    name: String,
    state: String,
    path: String,
}

pub fn read_workbook(package: &OfficePackage) -> OfficeResult<WorkbookEvidence> {
    let workbook_xml = package.text("xl/workbook.xml")?;
    let workbook_relationships = package
        .members
        .get("xl/_rels/workbook.xml.rels")
        .and_then(|bytes| std::str::from_utf8(bytes).ok())
        .map(relationship_map)
        .unwrap_or_default();
    let sheets = parse_sheet_refs(workbook_xml, &workbook_relationships)?;
    let shared_strings = package
        .members
        .get("xl/sharedStrings.xml")
        .and_then(|bytes| std::str::from_utf8(bytes).ok())
        .map(parse_shared_strings)
        .unwrap_or_default();
    let date_styles = package
        .members
        .get("xl/styles.xml")
        .and_then(|bytes| std::str::from_utf8(bytes).ok())
        .map(parse_date_styles)
        .unwrap_or_default();
    let mut worksheet_evidence = Vec::with_capacity(sheets.len());
    for sheet in sheets {
        let xml = package.text(&sheet.path)?;
        worksheet_evidence.push(parse_worksheet(
            package,
            &sheet,
            xml,
            &shared_strings,
            &date_styles,
        )?);
    }
    Ok(WorkbookEvidence {
        sheets: worksheet_evidence,
        named_ranges: parse_named_ranges(workbook_xml),
        date_system: parse_date_system(workbook_xml),
    })
}

fn parse_sheet_refs(
    xml: &str,
    relationships: &BTreeMap<String, (String, String, bool)>,
) -> OfficeResult<Vec<SheetRef>> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut sheets = Vec::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(element)) | Ok(Event::Empty(element))
                if local_name(element.name().as_ref()) == b"sheet" =>
            {
                let name = attr_value(&element, b"name").unwrap_or_default();
                let id = attr_value(&element, b"sheetId")
                    .unwrap_or_else(|| (sheets.len() + 1).to_string());
                let relationship_id = attr_value_qualified(&element, b"r:id").unwrap_or_default();
                let state = attr_value(&element, b"state").unwrap_or_else(|| "visible".into());
                let path = relationships
                    .get(&relationship_id)
                    .map(|(target, _, _)| normalize_xl_target(target))
                    .unwrap_or_else(|| format!("xl/worksheets/sheet{}.xml", sheets.len() + 1));
                sheets.push(SheetRef {
                    id,
                    name,
                    state,
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
    Ok(sheets)
}

fn parse_worksheet(
    package: &OfficePackage,
    sheet: &SheetRef,
    xml: &str,
    shared_strings: &[String],
    date_styles: &BTreeSet<u32>,
) -> OfficeResult<WorksheetEvidence> {
    let relationships_path = worksheet_relationships_path(&sheet.path);
    let relationships = package
        .members
        .get(&relationships_path)
        .and_then(|bytes| std::str::from_utf8(bytes).ok())
        .map(relationship_map)
        .unwrap_or_default();
    let comments = load_comments(package, &sheet.path, &relationships);
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut cells = Vec::new();
    let mut merged_ranges = Vec::new();
    let mut tables = Vec::new();
    let mut charts = Vec::new();
    let mut images = Vec::new();
    let mut freeze_panes = Vec::new();
    let mut auto_filter = None;
    let mut dimension = None;
    let mut hyperlinks: BTreeMap<String, String> = BTreeMap::new();
    let mut current_cell: Option<CellBuilder> = None;
    let mut capture_formula = false;
    let mut capture_value = false;
    let mut capture_inline = false;

    loop {
        match reader.read_event() {
            Ok(Event::Start(element)) => match local_name(element.name().as_ref()) {
                b"c" => current_cell = Some(CellBuilder::from_element(&element)),
                b"f" if current_cell.is_some() => capture_formula = true,
                b"v" if current_cell.is_some() => capture_value = true,
                b"t" if current_cell.is_some() => capture_inline = true,
                b"hyperlink" => record_hyperlink(&element, &relationships, &mut hyperlinks),
                _ => {}
            },
            Ok(Event::Empty(element)) => match local_name(element.name().as_ref()) {
                b"dimension" => dimension = attr_value(&element, b"ref"),
                b"mergeCell" => {
                    if let Some(reference) = attr_value(&element, b"ref") {
                        merged_ranges.push(reference);
                    }
                }
                b"pane" => {
                    if let Some(reference) = attr_value(&element, b"topLeftCell") {
                        freeze_panes.push(reference);
                    }
                }
                b"autoFilter" => auto_filter = attr_value(&element, b"ref"),
                b"hyperlink" => record_hyperlink(&element, &relationships, &mut hyperlinks),
                _ => {}
            },
            Ok(Event::Text(text)) if current_cell.is_some() => {
                let value = text.unescape().map_err(|error| {
                    OfficeError::new(
                        "malformed_xml",
                        format!("invalid worksheet text: {error}"),
                        json!({"part": sheet.path}),
                    )
                })?;
                if capture_formula {
                    current_cell.as_mut().unwrap().formula.push_str(&value);
                } else if capture_value {
                    current_cell.as_mut().unwrap().value.push_str(&value);
                } else if capture_inline {
                    current_cell.as_mut().unwrap().inline_text.push_str(&value);
                }
            }
            Ok(Event::End(element)) => match local_name(element.name().as_ref()) {
                b"f" => capture_formula = false,
                b"v" => capture_value = false,
                b"t" => capture_inline = false,
                b"c" => {
                    if let Some(cell) = current_cell.take() {
                        cells.push(cell.finish(shared_strings, date_styles, &comments));
                    }
                }
                _ => {}
            },
            Ok(Event::Eof) => break,
            Err(error) => {
                return Err(OfficeError::new(
                    "malformed_xml",
                    format!("cannot parse worksheet XML: {error}"),
                    json!({"part": sheet.path}),
                ))
            }
            _ => {}
        }
    }

    for cell in &mut cells {
        cell.hyperlink = hyperlinks.get(&cell.reference).cloned();
    }
    for (_, (target, relation_type, external)) in relationships {
        if external {
            continue;
        }
        let normalized = normalize_part_target(&sheet.path, &target);
        if relation_type.ends_with("/table") {
            tables.push(normalized);
        } else if relation_type.ends_with("/drawing") {
            collect_drawing_refs(package, &normalized, &mut charts, &mut images);
        }
    }
    Ok(WorksheetEvidence {
        id: format!("sheet_{}", sheet.id),
        name: sheet.name.clone(),
        state: sheet.state.clone(),
        dimension,
        cells,
        merged_ranges,
        tables,
        charts,
        images,
        freeze_panes,
        auto_filter,
        untrusted: true,
    })
}

struct CellBuilder {
    reference: String,
    cell_type: String,
    style_id: Option<u32>,
    value: String,
    inline_text: String,
    formula: String,
}

impl CellBuilder {
    fn from_element(element: &quick_xml::events::BytesStart<'_>) -> Self {
        Self {
            reference: attr_value(element, b"r").unwrap_or_default(),
            cell_type: attr_value(element, b"t").unwrap_or_else(|| "n".to_string()),
            style_id: attr_value(element, b"s").and_then(|value| value.parse().ok()),
            value: String::new(),
            inline_text: String::new(),
            formula: String::new(),
        }
    }

    fn finish(
        self,
        shared_strings: &[String],
        date_styles: &BTreeSet<u32>,
        comments: &BTreeMap<String, String>,
    ) -> CellEvidence {
        let displayed = if self.cell_type == "s" {
            self.value
                .parse::<usize>()
                .ok()
                .and_then(|index| shared_strings.get(index).cloned())
        } else if self.cell_type == "inlineStr" || self.cell_type == "str" {
            Some(if self.inline_text.is_empty() {
                self.value.clone()
            } else {
                self.inline_text.clone()
            })
        } else {
            Some(self.value.clone())
        };
        let is_date = self.style_id.is_some_and(|id| date_styles.contains(&id));
        let cell_type = if is_date {
            "date_serial".to_string()
        } else {
            match self.cell_type.as_str() {
                "s" | "inlineStr" | "str" => "string",
                "b" => "boolean",
                "e" => "error",
                "d" => "date",
                _ => "number",
            }
            .to_string()
        };
        let value = match cell_type.as_str() {
            "boolean" => Some(Value::Bool(self.value == "1" || self.value == "true")),
            "number" | "date_serial" => self
                .value
                .parse::<f64>()
                .ok()
                .and_then(Number::from_f64)
                .map(Value::Number),
            _ => displayed.clone().map(Value::String),
        };
        CellEvidence {
            reference: self.reference.clone(),
            cell_type,
            value,
            displayed_value: displayed,
            formula: (!self.formula.is_empty()).then_some(self.formula),
            style_id: self.style_id,
            hyperlink: None,
            comment: comments.get(&self.reference).cloned(),
            untrusted: true,
        }
    }
}

fn parse_shared_strings(xml: &str) -> Vec<String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut strings = Vec::new();
    let mut current = String::new();
    let mut in_item = false;
    loop {
        match reader.read_event() {
            Ok(Event::Start(element)) if local_name(element.name().as_ref()) == b"si" => {
                in_item = true;
                current.clear();
            }
            Ok(Event::Text(text)) if in_item => {
                if let Ok(value) = text.unescape() {
                    current.push_str(&value);
                }
            }
            Ok(Event::End(element)) if local_name(element.name().as_ref()) == b"si" => {
                strings.push(std::mem::take(&mut current));
                in_item = false;
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    strings
}

fn parse_date_styles(xml: &str) -> BTreeSet<u32> {
    let builtin_dates = [14u32, 15, 16, 17, 18, 19, 20, 21, 22, 45, 46, 47]
        .into_iter()
        .collect::<BTreeSet<_>>();
    let mut custom_date_formats = BTreeSet::new();
    let mut style_formats = Vec::new();
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut in_cell_formats = false;
    loop {
        match reader.read_event() {
            Ok(Event::Start(element)) if local_name(element.name().as_ref()) == b"cellXfs" => {
                in_cell_formats = true;
            }
            Ok(Event::End(element)) if local_name(element.name().as_ref()) == b"cellXfs" => {
                in_cell_formats = false;
            }
            Ok(Event::Start(element)) | Ok(Event::Empty(element))
                if local_name(element.name().as_ref()) == b"numFmt" =>
            {
                if let (Some(id), Some(code)) = (
                    attr_value(&element, b"numFmtId").and_then(|value| value.parse::<u32>().ok()),
                    attr_value(&element, b"formatCode"),
                ) {
                    if looks_like_date_format(&code) {
                        custom_date_formats.insert(id);
                    }
                }
            }
            Ok(Event::Start(element)) | Ok(Event::Empty(element))
                if in_cell_formats && local_name(element.name().as_ref()) == b"xf" =>
            {
                style_formats.push(
                    attr_value(&element, b"numFmtId")
                        .and_then(|value| value.parse::<u32>().ok())
                        .unwrap_or(0),
                );
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    style_formats
        .iter()
        .enumerate()
        .filter_map(|(index, format)| {
            (builtin_dates.contains(format) || custom_date_formats.contains(format))
                .then_some(index as u32)
        })
        .collect()
}

fn looks_like_date_format(format: &str) -> bool {
    let normalized = format
        .chars()
        .filter(|character| !matches!(character, '"' | '\\' | '[' | ']'))
        .collect::<String>()
        .to_ascii_lowercase();
    normalized.contains('y')
        || normalized.contains("dd")
        || normalized.contains("mm")
        || normalized.contains("hh")
}

fn parse_named_ranges(xml: &str) -> Vec<String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut ranges = Vec::new();
    let mut current_name = None;
    loop {
        match reader.read_event() {
            Ok(Event::Start(element)) if local_name(element.name().as_ref()) == b"definedName" => {
                current_name = attr_value(&element, b"name");
            }
            Ok(Event::Text(text)) if current_name.is_some() => {
                let value = text
                    .unescape()
                    .map(|value| value.into_owned())
                    .unwrap_or_default();
                ranges.push(format!(
                    "{}={}",
                    current_name.take().unwrap_or_default(),
                    value
                ));
            }
            Ok(Event::End(element)) if local_name(element.name().as_ref()) == b"definedName" => {
                current_name = None;
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    ranges
}

fn parse_date_system(xml: &str) -> String {
    if xml.contains("date1904=\"1\"") || xml.contains("date1904=\"true\"") {
        "1904".to_string()
    } else {
        "1900".to_string()
    }
}

fn record_hyperlink(
    element: &quick_xml::events::BytesStart<'_>,
    relationships: &BTreeMap<String, (String, String, bool)>,
    hyperlinks: &mut BTreeMap<String, String>,
) {
    let Some(reference) = attr_value(element, b"ref") else {
        return;
    };
    let target = attr_value(element, b"location").or_else(|| {
        attr_value_qualified(element, b"r:id")
            .and_then(|id| relationships.get(&id).map(|(target, _, _)| target.clone()))
    });
    if let Some(target) = target {
        hyperlinks.insert(reference, target);
    }
}

fn load_comments(
    package: &OfficePackage,
    sheet_path: &str,
    relationships: &BTreeMap<String, (String, String, bool)>,
) -> BTreeMap<String, String> {
    let comments_part = relationships
        .values()
        .find(|(_, kind, external)| !*external && kind.ends_with("/comments"))
        .map(|(target, _, _)| normalize_part_target(sheet_path, target));
    let Some(bytes) = comments_part.and_then(|part| package.members.get(&part)) else {
        return BTreeMap::new();
    };
    let Ok(xml) = std::str::from_utf8(bytes) else {
        return BTreeMap::new();
    };
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut comments = BTreeMap::new();
    let mut reference = None;
    let mut text = String::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(element)) if local_name(element.name().as_ref()) == b"comment" => {
                reference = attr_value(&element, b"ref");
                text.clear();
            }
            Ok(Event::Text(value)) if reference.is_some() => {
                if let Ok(value) = value.unescape() {
                    text.push_str(&value);
                }
            }
            Ok(Event::End(element)) if local_name(element.name().as_ref()) == b"comment" => {
                if let Some(reference) = reference.take() {
                    comments.insert(reference, text.clone());
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    comments
}

fn collect_drawing_refs(
    package: &OfficePackage,
    drawing_part: &str,
    charts: &mut Vec<String>,
    images: &mut Vec<String>,
) {
    let Some(bytes) = package.members.get(drawing_part) else {
        return;
    };
    let Ok(xml) = std::str::from_utf8(bytes) else {
        return;
    };
    let refs = collect_text(xml);
    let rels_path = drawing_relationships_path(drawing_part);
    if let Some(rels) = package
        .members
        .get(&rels_path)
        .and_then(|bytes| std::str::from_utf8(bytes).ok())
    {
        for (_, (target, kind, external)) in relationship_map(rels) {
            if external {
                continue;
            }
            let target = normalize_part_target(drawing_part, &target);
            if kind.ends_with("/chart") {
                charts.push(target);
            } else if kind.ends_with("/image") {
                images.push(target);
            }
        }
    }
    if refs.is_empty() {
        return;
    }
}

fn normalize_xl_target(target: &str) -> String {
    if target.starts_with('/') {
        target.trim_start_matches('/').to_string()
    } else if target.starts_with("xl/") {
        target.to_string()
    } else {
        normalize_segments(&format!("xl/{target}"))
    }
}

fn normalize_part_target(source: &str, target: &str) -> String {
    if target.starts_with('/') {
        return target.trim_start_matches('/').to_string();
    }
    let base = source.rsplit_once('/').map(|(base, _)| base).unwrap_or("");
    normalize_segments(&format!("{base}/{target}"))
}

fn normalize_segments(path: &str) -> String {
    let mut parts = Vec::new();
    for part in path.split('/') {
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

fn worksheet_relationships_path(sheet_path: &str) -> String {
    let (directory, file) = sheet_path.rsplit_once('/').unwrap_or(("", sheet_path));
    format!("{directory}/_rels/{file}.rels")
}

fn drawing_relationships_path(drawing_path: &str) -> String {
    let (directory, file) = drawing_path.rsplit_once('/').unwrap_or(("", drawing_path));
    format!("{directory}/_rels/{file}.rels")
}

#[cfg(test)]
#[path = "xlsx_tests.rs"]
mod tests;
