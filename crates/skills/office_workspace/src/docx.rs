use crate::error::{OfficeError, OfficeResult};
use crate::model::{DocumentBlock, OfficeTable, TextRun};
use crate::package::OfficePackage;
use crate::xml::{attr_value, attr_value_qualified, local_name};
use quick_xml::events::Event;
use quick_xml::Reader;
use serde_json::json;

pub struct DocxEvidence {
    pub blocks: Vec<DocumentBlock>,
    pub tables: Vec<OfficeTable>,
}

pub fn read_docx(package: &OfficePackage) -> OfficeResult<DocxEvidence> {
    let mut blocks = Vec::new();
    let mut tables = Vec::new();
    let mut parts = package
        .members
        .keys()
        .filter(|name| {
            *name == "word/document.xml"
                || name.starts_with("word/header")
                || name.starts_with("word/footer")
                || matches!(
                    name.as_str(),
                    "word/footnotes.xml" | "word/endnotes.xml" | "word/comments.xml"
                )
        })
        .cloned()
        .collect::<Vec<_>>();
    parts.sort_by_key(|name| {
        if name == "word/document.xml" {
            (0, name.clone())
        } else {
            (1, name.clone())
        }
    });
    for part in parts {
        let xml = package.text(&part)?;
        parse_part(xml, &part, &mut blocks, &mut tables)?;
    }
    Ok(DocxEvidence { blocks, tables })
}

fn parse_part(
    xml: &str,
    part: &str,
    blocks: &mut Vec<DocumentBlock>,
    tables: &mut Vec<OfficeTable>,
) -> OfficeResult<()> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut paragraph_depth = 0usize;
    let mut paragraph_text = String::new();
    let mut runs = Vec::new();
    let mut run_text = String::new();
    let mut run_style = None;
    let mut paragraph_style = None;
    let mut heading_level = None;
    let mut list_level = None;
    let mut hyperlink = None;
    let mut table_depth = 0usize;
    let mut row_depth = 0usize;
    let mut cell_depth = 0usize;
    let mut table_rows: Vec<Vec<String>> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut cell = String::new();
    let mut part_block_index = 0usize;
    let mut part_table_index = 0usize;

    loop {
        match reader.read_event() {
            Ok(Event::Start(element)) => match local_name(element.name().as_ref()) {
                b"p" => {
                    paragraph_depth += 1;
                    if paragraph_depth == 1 {
                        paragraph_text.clear();
                        runs.clear();
                        paragraph_style = None;
                        heading_level = None;
                        list_level = None;
                    }
                }
                b"r" if paragraph_depth > 0 => {
                    run_text.clear();
                    run_style = None;
                }
                b"pStyle" if paragraph_depth > 0 => {
                    paragraph_style = attr_value(&element, b"val");
                    heading_level = paragraph_style.as_deref().and_then(parse_heading_level);
                }
                b"rStyle" if paragraph_depth > 0 => {
                    run_style = attr_value(&element, b"val");
                }
                b"ilvl" if paragraph_depth > 0 => {
                    list_level =
                        attr_value(&element, b"val").and_then(|value| value.parse::<u8>().ok());
                }
                b"hyperlink" if paragraph_depth > 0 => {
                    hyperlink = attr_value_qualified(&element, b"r:id");
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
                b"tab" if paragraph_depth > 0 => {
                    append_text("\t", &mut run_text, &mut paragraph_text, &mut cell);
                }
                b"br" if paragraph_depth > 0 => {
                    append_text("\n", &mut run_text, &mut paragraph_text, &mut cell);
                }
                _ => {}
            },
            Ok(Event::Empty(element)) => match local_name(element.name().as_ref()) {
                b"pStyle" if paragraph_depth > 0 => {
                    paragraph_style = attr_value(&element, b"val");
                    heading_level = paragraph_style.as_deref().and_then(parse_heading_level);
                }
                b"rStyle" if paragraph_depth > 0 => {
                    run_style = attr_value(&element, b"val");
                }
                b"ilvl" if paragraph_depth > 0 => {
                    list_level =
                        attr_value(&element, b"val").and_then(|value| value.parse::<u8>().ok());
                }
                b"tab" if paragraph_depth > 0 => {
                    append_text("\t", &mut run_text, &mut paragraph_text, &mut cell);
                }
                b"br" if paragraph_depth > 0 => {
                    append_text("\n", &mut run_text, &mut paragraph_text, &mut cell);
                }
                _ => {}
            },
            Ok(Event::Text(text)) if paragraph_depth > 0 => {
                let value = text.unescape().map_err(|error| {
                    OfficeError::new(
                        "malformed_xml",
                        format!("invalid DOCX text: {error}"),
                        json!({"part": part}),
                    )
                })?;
                append_text(&value, &mut run_text, &mut paragraph_text, &mut cell);
            }
            Ok(Event::CData(text)) if paragraph_depth > 0 => {
                let value = String::from_utf8_lossy(text.as_ref());
                append_text(&value, &mut run_text, &mut paragraph_text, &mut cell);
            }
            Ok(Event::End(element)) => match local_name(element.name().as_ref()) {
                b"r" if paragraph_depth > 0 => {
                    if !run_text.is_empty() {
                        runs.push(TextRun {
                            text: run_text.clone(),
                            style: run_style.take(),
                            hyperlink: hyperlink.clone(),
                        });
                    }
                    run_text.clear();
                }
                b"hyperlink" => hyperlink = None,
                b"p" if paragraph_depth > 0 => {
                    if paragraph_depth == 1 && !paragraph_text.trim().is_empty() {
                        part_block_index += 1;
                        blocks.push(DocumentBlock {
                            id: stable_id(part, "paragraph", part_block_index),
                            kind: part_kind(part).to_string(),
                            text: paragraph_text.trim().to_string(),
                            runs: runs.clone(),
                            style: paragraph_style.clone(),
                            heading_level,
                            list_level,
                            source_part: part.to_string(),
                            untrusted: true,
                        });
                    }
                    paragraph_depth -= 1;
                }
                b"tc" if cell_depth > 0 => {
                    if cell_depth == 1 {
                        row.push(cell.trim().to_string());
                        cell.clear();
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
                        part_table_index += 1;
                        tables.push(OfficeTable {
                            id: stable_id(part, "table", part_table_index),
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
                    format!("cannot parse DOCX XML: {error}"),
                    json!({"part": part}),
                ))
            }
            _ => {}
        }
    }
    Ok(())
}

fn append_text(value: &str, run: &mut String, paragraph: &mut String, cell: &mut String) {
    run.push_str(value);
    paragraph.push_str(value);
    cell.push_str(value);
}

fn parse_heading_level(style: &str) -> Option<u8> {
    let lower = style.to_ascii_lowercase();
    lower
        .strip_prefix("heading")
        .or_else(|| lower.strip_prefix("title"))
        .and_then(|suffix| {
            suffix
                .trim_matches(|character: char| !character.is_ascii_digit())
                .parse()
                .ok()
        })
}

fn part_kind(part: &str) -> &'static str {
    if part.contains("/header") {
        "header"
    } else if part.contains("/footer") {
        "footer"
    } else if part.ends_with("/footnotes.xml") {
        "footnote"
    } else if part.ends_with("/endnotes.xml") {
        "endnote"
    } else if part.ends_with("/comments.xml") {
        "comment"
    } else {
        "paragraph"
    }
}

fn stable_id(part: &str, kind: &str, index: usize) -> String {
    let part = part.trim_end_matches(".xml").replace(['/', '.'], "_");
    format!("{part}_{kind}_{index}")
}

#[cfg(test)]
#[path = "docx_tests.rs"]
mod tests;
