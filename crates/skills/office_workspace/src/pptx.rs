use crate::error::{OfficeError, OfficeResult};
use crate::model::{OfficeTable, PresentationEvidence, SlideEvidence};
use crate::package::OfficePackage;
use crate::xml::{attr_value, attr_value_qualified, local_name, relationship_map};
use quick_xml::events::Event;
use quick_xml::Reader;
use serde_json::json;

pub fn read_presentation(package: &OfficePackage) -> OfficeResult<PresentationEvidence> {
    let presentation_xml = package.text("ppt/presentation.xml")?;
    let relationships = package
        .members
        .get("ppt/_rels/presentation.xml.rels")
        .and_then(|bytes| std::str::from_utf8(bytes).ok())
        .map(relationship_map)
        .unwrap_or_default();
    let slide_refs = parse_slide_refs(presentation_xml);
    let mut slides = Vec::new();
    for (index, relationship_id) in slide_refs.into_iter().enumerate() {
        let Some((target, _, false)) = relationships.get(&relationship_id) else {
            continue;
        };
        let part = normalize_ppt_target(target);
        let xml = package.text(&part)?;
        slides.push(parse_slide(
            package,
            index + 1,
            &relationship_id,
            &part,
            xml,
        )?);
    }
    Ok(PresentationEvidence {
        slides,
        masters: matching_parts(package, "ppt/slideMasters/", ".xml"),
        layouts: matching_parts(package, "ppt/slideLayouts/", ".xml"),
        themes: matching_parts(package, "ppt/theme/", ".xml"),
    })
}

fn parse_slide_refs(xml: &str) -> Vec<String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut ids = Vec::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(element)) | Ok(Event::Empty(element))
                if local_name(element.name().as_ref()) == b"sldId" =>
            {
                if let Some(id) = attr_value_qualified(&element, b"r:id") {
                    ids.push(id);
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    ids
}

fn parse_slide(
    package: &OfficePackage,
    index: usize,
    relationship_id: &str,
    part: &str,
    xml: &str,
) -> OfficeResult<SlideEvidence> {
    let stable_slide_id = stable_slide_id(part);
    let rels_path = slide_relationships_path(part);
    let relationships = package
        .members
        .get(&rels_path)
        .and_then(|bytes| std::str::from_utf8(bytes).ok())
        .map(relationship_map)
        .unwrap_or_default();
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut text = Vec::new();
    let mut paragraph = String::new();
    let mut in_paragraph = false;
    let mut table_depth = 0usize;
    let mut row_depth = 0usize;
    let mut cell_depth = 0usize;
    let mut table_rows = Vec::new();
    let mut row = Vec::new();
    let mut cell = String::new();
    let mut tables = Vec::new();
    let mut shapes = Vec::new();
    let mut hidden = false;

    loop {
        match reader.read_event() {
            Ok(Event::Start(element)) => match local_name(element.name().as_ref()) {
                b"sld" => {
                    hidden = attr_value(&element, b"show").as_deref() == Some("0");
                }
                b"p" => {
                    in_paragraph = true;
                    paragraph.clear();
                }
                b"tbl" => {
                    table_depth += 1;
                    if table_depth == 1 {
                        table_rows.clear();
                    }
                }
                b"tr" if table_depth > 0 => {
                    row_depth += 1;
                    if row_depth == 1 {
                        row.clear();
                    }
                }
                b"tc" if row_depth > 0 => {
                    cell_depth += 1;
                    if cell_depth == 1 {
                        cell.clear();
                    }
                }
                b"cNvPr" => {
                    let name = attr_value(&element, b"name").unwrap_or_default();
                    let id = attr_value(&element, b"id").unwrap_or_default();
                    shapes.push(format!("{id}:{name}"));
                }
                _ => {}
            },
            Ok(Event::Empty(element)) => match local_name(element.name().as_ref()) {
                b"sld" => {
                    hidden = attr_value(&element, b"show").as_deref() == Some("0");
                }
                b"cNvPr" => {
                    let name = attr_value(&element, b"name").unwrap_or_default();
                    let id = attr_value(&element, b"id").unwrap_or_default();
                    shapes.push(format!("{id}:{name}"));
                }
                _ => {}
            },
            Ok(Event::Text(value)) if in_paragraph => {
                let value = value.unescape().map_err(|error| {
                    OfficeError::new(
                        "malformed_xml",
                        format!("invalid slide text: {error}"),
                        json!({"part": part}),
                    )
                })?;
                paragraph.push_str(&value);
                if cell_depth > 0 {
                    cell.push_str(&value);
                }
            }
            Ok(Event::End(element)) => match local_name(element.name().as_ref()) {
                b"p" => {
                    in_paragraph = false;
                    if !paragraph.trim().is_empty() {
                        text.push(paragraph.trim().to_string());
                    }
                }
                b"tc" if cell_depth > 0 => {
                    if cell_depth == 1 {
                        row.push(cell.trim().to_string());
                    }
                    cell_depth -= 1;
                }
                b"tr" if row_depth > 0 => {
                    if row_depth == 1 {
                        table_rows.push(std::mem::take(&mut row));
                    }
                    row_depth -= 1;
                }
                b"tbl" if table_depth > 0 => {
                    if table_depth == 1 {
                        tables.push(OfficeTable {
                            id: format!("{stable_slide_id}_table_{}", tables.len() + 1),
                            source_part: part.to_string(),
                            rows: std::mem::take(&mut table_rows),
                            untrusted: true,
                        });
                    }
                    table_depth -= 1;
                }
                _ => {}
            },
            Ok(Event::Eof) => break,
            Err(error) => {
                return Err(OfficeError::new(
                    "malformed_xml",
                    format!("cannot parse slide XML: {error}"),
                    json!({"part": part}),
                ))
            }
            _ => {}
        }
    }

    let mut notes = Vec::new();
    let mut charts = Vec::new();
    let mut images = Vec::new();
    let mut layout = None;
    for (_, (target, kind, external)) in relationships {
        if external {
            continue;
        }
        let normalized = normalize_part_target(part, &target);
        if kind.ends_with("/notesSlide") {
            if let Some(xml) = package
                .members
                .get(&normalized)
                .and_then(|bytes| std::str::from_utf8(bytes).ok())
            {
                notes.extend(plain_text(xml));
            }
        } else if kind.ends_with("/chart") {
            charts.push(normalized);
        } else if kind.ends_with("/image") {
            images.push(normalized);
        } else if kind.ends_with("/slideLayout") {
            layout = Some(normalized);
        }
    }
    Ok(SlideEvidence {
        id: stable_slide_id,
        index,
        relationship_id: Some(relationship_id.to_string()),
        layout,
        hidden,
        title: text.first().cloned(),
        text,
        notes,
        tables,
        charts,
        shapes,
        images,
        untrusted: true,
    })
}

fn stable_slide_id(part: &str) -> String {
    part.rsplit_once('/')
        .map(|(_, file)| file)
        .unwrap_or(part)
        .strip_prefix("slide")
        .and_then(|value| value.strip_suffix(".xml"))
        .filter(|value| {
            !value.is_empty() && value.chars().all(|character| character.is_ascii_digit())
        })
        .map(|value| format!("slide_{value}"))
        .unwrap_or_else(|| format!("slide_part_{}", crate::package::hash_bytes(part.as_bytes())))
}

fn plain_text(xml: &str) -> Vec<String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut values = Vec::new();
    loop {
        match reader.read_event() {
            Ok(Event::Text(text)) => {
                if let Ok(value) = text.unescape() {
                    let value = value.trim();
                    if !value.is_empty() {
                        values.push(value.to_string());
                    }
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    values
}

fn matching_parts(package: &OfficePackage, prefix: &str, suffix: &str) -> Vec<String> {
    package
        .members
        .keys()
        .filter(|name| name.starts_with(prefix) && name.ends_with(suffix))
        .cloned()
        .collect()
}

fn normalize_ppt_target(target: &str) -> String {
    if target.starts_with('/') {
        target.trim_start_matches('/').to_string()
    } else if target.starts_with("ppt/") {
        target.to_string()
    } else {
        normalize_segments(&format!("ppt/{target}"))
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

fn slide_relationships_path(slide_path: &str) -> String {
    let (directory, file) = slide_path.rsplit_once('/').unwrap_or(("", slide_path));
    format!("{directory}/_rels/{file}.rels")
}

#[cfg(test)]
#[path = "pptx_tests.rs"]
mod tests;
