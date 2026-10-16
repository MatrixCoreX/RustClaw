use anyhow::{anyhow, Result};
use doc_parse::{extract_pdf_text, PdfExtractionLimits};
use office_workspace::{inspect_for_ingestion, OfficeArtifactEnvelope, OfficeFormat};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

pub(super) const DOCUMENT_EXTRACTION_SCHEMA_VERSION: u32 = 1;
const MAX_OFFICE_BLOCKS: usize = 100_000;
const MAX_EXTRACTED_CHARS: usize = 8_000_000;
const MAX_TEXT_LINES: usize = 1_000_000;
const MAX_OFFICE_TABLE_ROWS: usize = 200_000;
const MAX_SPREADSHEET_CELLS: usize = 1_000_000;

#[derive(Debug, Clone, Serialize)]
pub(super) struct DocumentExtractionResult {
    pub(super) schema_version: u32,
    pub(super) media_type: String,
    pub(super) parser_version: String,
    pub(super) blocks: Vec<DocumentTextBlock>,
    pub(super) warnings: Vec<ExtractionWarning>,
}

impl DocumentExtractionResult {
    #[cfg(test)]
    pub(super) fn text(&self) -> String {
        self.blocks
            .iter()
            .map(|block| block.text.as_str())
            .filter(|text| !text.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct DocumentTextBlock {
    pub(super) kind: &'static str,
    pub(super) text: String,
    pub(super) location: DocumentLocation,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct DocumentLocation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) line_start: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) line_end: Option<usize>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(super) section_path: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) page: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) sheet: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) slide: Option<u32>,
}

#[derive(Debug, Clone)]
pub(super) struct ExtractedDocumentChunk {
    pub(super) content_kind: String,
    pub(super) text: String,
    pub(super) location: DocumentLocation,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct ExtractionWarning {
    pub(super) warning_code: String,
    pub(super) message_key: String,
}

#[derive(Debug)]
pub(super) enum ExtractOutcome {
    Extracted(DocumentExtractionResult),
    Skip {
        error_code: String,
        message_key: String,
    },
}

pub(super) fn extract_document(
    path: &Path,
    bytes: &[u8],
    file_type: &str,
) -> Result<ExtractOutcome> {
    match file_type {
        "pdf" => return Ok(extract_pdf(path, bytes.len() as u64)),
        "docx" | "xlsx" | "pptx" => return Ok(extract_office(path)),
        _ => {}
    }
    if bytes.iter().any(|byte| *byte == 0) {
        return Ok(skip("binary_content_unsupported"));
    }
    let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
    let raw = match std::str::from_utf8(bytes) {
        Ok(text) => text.replace("\r\n", "\n"),
        Err(_) => {
            return Ok(skip("text_encoding_unsupported"));
        }
    };
    if raw.lines().count() > MAX_TEXT_LINES {
        return Ok(skip("document_line_limit_exceeded"));
    }
    let (blocks, media_type, parser_version) = match file_type {
        "json" => {
            let value: serde_json::Value =
                serde_json::from_str(&raw).map_err(|error| anyhow!("invalid JSON: {error}"))?;
            (
                single_block("structured_data", serde_json::to_string_pretty(&value)?),
                "application/json",
                "json-structured-v2",
            )
        }
        "jsonl" | "ndjson" => {
            let normalized = normalize_json_lines(&raw)?;
            (
                line_blocks(&normalized, "structured_record"),
                "application/x-ndjson",
                "jsonl-records-v2",
            )
        }
        "csv" => (line_blocks(&raw, "table_row"), "text/csv", "csv-rows-v3"),
        "tsv" => (
            line_blocks(&raw, "table_row"),
            "text/tab-separated-values",
            "tsv-rows-v3",
        ),
        "html" | "htm" => (
            extract_visible_html_blocks(&raw)?,
            "text/html",
            "html-visible-structure-v3",
        ),
        "md" | "markdown" => (
            extract_markdown_blocks(&raw),
            "text/markdown",
            "markdown-structure-v2",
        ),
        extension if is_source_code_extension(extension) => (
            single_block("source_code", raw),
            "text/plain",
            "source-code-v1",
        ),
        _ => (
            single_block("plain_text", raw),
            "text/plain",
            "utf8-text-v3",
        ),
    };
    if blocks.is_empty() {
        return Ok(skip("document_content_empty"));
    }
    Ok(ExtractOutcome::Extracted(DocumentExtractionResult {
        schema_version: DOCUMENT_EXTRACTION_SCHEMA_VERSION,
        media_type: media_type.to_string(),
        parser_version: parser_version.to_string(),
        blocks,
        warnings: Vec::new(),
    }))
}

pub(super) fn chunk_extraction(
    extraction: &DocumentExtractionResult,
    chunk_size: usize,
    chunk_overlap: usize,
) -> Vec<ExtractedDocumentChunk> {
    extraction
        .blocks
        .iter()
        .flat_map(|block| {
            super::split_chunks(&block.text, chunk_size, chunk_overlap)
                .into_iter()
                .map(|text| ExtractedDocumentChunk {
                    content_kind: block.kind.to_string(),
                    text,
                    location: block.location.clone(),
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn extract_pdf(path: &Path, source_bytes: u64) -> ExtractOutcome {
    let limits = PdfExtractionLimits {
        max_source_bytes: source_bytes.max(1),
        max_pages: 2_000,
        max_text_chars: MAX_EXTRACTED_CHARS,
    };
    let extraction = match extract_pdf_text(path, None, None, limits) {
        Ok(extraction) => extraction,
        Err(error) => return skip(error.code),
    };
    let blocks = extraction
        .pages
        .into_iter()
        .filter(|page| !page.text.trim().is_empty())
        .map(|page| DocumentTextBlock {
            kind: "pdf_text_layer",
            text: page.text,
            location: DocumentLocation {
                page: Some(page.page),
                ..DocumentLocation::default()
            },
        })
        .collect::<Vec<_>>();
    if blocks.is_empty() {
        return skip("document_content_empty");
    }
    let warnings = extraction
        .lossy_utf8
        .then(|| extraction_warning("text_encoding_lossy"))
        .into_iter()
        .collect();
    ExtractOutcome::Extracted(DocumentExtractionResult {
        schema_version: DOCUMENT_EXTRACTION_SCHEMA_VERSION,
        media_type: "application/pdf".to_string(),
        parser_version: extraction.parser_version.to_string(),
        blocks,
        warnings,
    })
}

fn extract_office(path: &Path) -> ExtractOutcome {
    let envelope = match inspect_for_ingestion(path) {
        Ok(envelope) => envelope,
        Err(error) => return skip(error.code),
    };
    let format = envelope.format;
    let blocks = match office_blocks(&envelope) {
        Ok(blocks) => blocks,
        Err(code) => return skip(code),
    };
    if blocks.is_empty() {
        return skip("document_content_empty");
    }
    let warnings = envelope
        .warnings
        .iter()
        .map(|warning| extraction_warning(&warning.code))
        .collect::<Vec<_>>();
    ExtractOutcome::Extracted(DocumentExtractionResult {
        schema_version: DOCUMENT_EXTRACTION_SCHEMA_VERSION,
        media_type: office_media_type(format).to_string(),
        parser_version: "office-pure-parser-v1".to_string(),
        blocks,
        warnings,
    })
}

fn office_blocks(
    envelope: &OfficeArtifactEnvelope,
) -> std::result::Result<Vec<DocumentTextBlock>, &'static str> {
    let mut blocks = Vec::new();
    match envelope.format {
        OfficeFormat::Docx => {
            let mut section_path = Vec::<String>::new();
            for block in &envelope.document_blocks {
                if let Some(level) = block.heading_level {
                    let keep = usize::from(level.saturating_sub(1));
                    section_path.truncate(keep.min(section_path.len()));
                    section_path.push(block.text.clone());
                }
                if !block.text.trim().is_empty() {
                    blocks.push(DocumentTextBlock {
                        kind: if block.heading_level.is_some() {
                            "heading"
                        } else if block.list_level.is_some() {
                            "list_item"
                        } else {
                            "word_paragraph"
                        },
                        text: block.text.clone(),
                        location: DocumentLocation {
                            section_path: section_path.clone(),
                            ..DocumentLocation::default()
                        },
                    });
                }
                enforce_block_limits(&blocks)?;
            }
            for table in &envelope.tables {
                if table.rows.len() > MAX_OFFICE_TABLE_ROWS {
                    return Err("document_row_limit_exceeded");
                }
                for (row_index, row) in table.rows.iter().enumerate() {
                    let text = row.join("\t");
                    if text.trim().is_empty() {
                        continue;
                    }
                    blocks.push(DocumentTextBlock {
                        kind: "word_table_row",
                        text,
                        location: DocumentLocation {
                            section_path: vec![table.id.clone()],
                            line_start: Some(row_index + 1),
                            line_end: Some(row_index + 1),
                            ..DocumentLocation::default()
                        },
                    });
                    enforce_block_limits(&blocks)?;
                }
            }
        }
        OfficeFormat::Xlsx => {
            let Some(workbook) = &envelope.workbook else {
                return Err("office_structure_missing");
            };
            for sheet in &workbook.sheets {
                if sheet.cells.len() > MAX_SPREADSHEET_CELLS {
                    return Err("document_cell_limit_exceeded");
                }
                let mut rows = BTreeMap::<u32, Vec<String>>::new();
                for cell in &sheet.cells {
                    let Some(value) = cell.displayed_value.clone().or_else(|| {
                        cell.value.as_ref().map(|value| {
                            value
                                .as_str()
                                .map(str::to_string)
                                .unwrap_or_else(|| value.to_string())
                        })
                    }) else {
                        continue;
                    };
                    let row = spreadsheet_row(&cell.reference).unwrap_or(0);
                    rows.entry(row)
                        .or_default()
                        .push(format!("{}\t{}", cell.reference, value));
                }
                if rows.len() > MAX_OFFICE_TABLE_ROWS {
                    return Err("document_row_limit_exceeded");
                }
                for (row, cells) in rows {
                    blocks.push(DocumentTextBlock {
                        kind: "worksheet_row",
                        text: cells.join("\t"),
                        location: DocumentLocation {
                            sheet: Some(sheet.name.clone()),
                            section_path: vec![sheet.name.clone()],
                            line_start: (row > 0).then_some(row as usize),
                            line_end: (row > 0).then_some(row as usize),
                            ..DocumentLocation::default()
                        },
                    });
                    enforce_block_limits(&blocks)?;
                }
            }
        }
        OfficeFormat::Pptx => {
            let Some(presentation) = &envelope.presentation else {
                return Err("office_structure_missing");
            };
            for slide in &presentation.slides {
                let mut parts = Vec::new();
                if let Some(title) = &slide.title {
                    parts.push(title.clone());
                }
                parts.extend(slide.text.iter().cloned());
                parts.extend(slide.notes.iter().cloned());
                let text = parts
                    .into_iter()
                    .filter(|part| !part.trim().is_empty())
                    .collect::<Vec<_>>()
                    .join("\n");
                if !text.trim().is_empty() {
                    blocks.push(DocumentTextBlock {
                        kind: "presentation_slide",
                        text,
                        location: DocumentLocation {
                            slide: Some(slide.index as u32),
                            section_path: slide.title.clone().into_iter().collect(),
                            ..DocumentLocation::default()
                        },
                    });
                }
                enforce_block_limits(&blocks)?;
                for table in &slide.tables {
                    if table.rows.len() > MAX_OFFICE_TABLE_ROWS {
                        return Err("document_row_limit_exceeded");
                    }
                    for (row_index, row) in table.rows.iter().enumerate() {
                        let text = row.join("\t");
                        if text.trim().is_empty() {
                            continue;
                        }
                        blocks.push(DocumentTextBlock {
                            kind: "presentation_table_row",
                            text,
                            location: DocumentLocation {
                                slide: Some(slide.index as u32),
                                section_path: slide.title.clone().into_iter().collect(),
                                line_start: Some(row_index + 1),
                                line_end: Some(row_index + 1),
                                ..DocumentLocation::default()
                            },
                        });
                        enforce_block_limits(&blocks)?;
                    }
                }
            }
        }
    }
    if blocks
        .iter()
        .map(|block| block.text.chars().count())
        .sum::<usize>()
        > MAX_EXTRACTED_CHARS
    {
        return Err("document_text_limit_exceeded");
    }
    Ok(blocks)
}

fn enforce_block_limits(blocks: &[DocumentTextBlock]) -> std::result::Result<(), &'static str> {
    if blocks.len() > MAX_OFFICE_BLOCKS {
        return Err("document_block_limit_exceeded");
    }
    Ok(())
}

fn office_media_type(format: OfficeFormat) -> &'static str {
    match format {
        OfficeFormat::Docx => {
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
        }
        OfficeFormat::Xlsx => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        OfficeFormat::Pptx => {
            "application/vnd.openxmlformats-officedocument.presentationml.presentation"
        }
    }
}

fn skip(error_code: impl Into<String>) -> ExtractOutcome {
    let error_code = error_code.into();
    ExtractOutcome::Skip {
        message_key: format!("skill.kb.ingest.{error_code}"),
        error_code,
    }
}

fn extraction_warning(warning_code: impl Into<String>) -> ExtractionWarning {
    let warning_code = warning_code.into();
    ExtractionWarning {
        message_key: format!("skill.kb.ingest.{warning_code}"),
        warning_code,
    }
}

fn normalize_json_lines(raw: &str) -> Result<String> {
    raw.lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(index, line)| {
            serde_json::from_str::<serde_json::Value>(line)
                .and_then(|value| serde_json::to_string(&value))
                .map_err(|error| anyhow!("invalid JSONL at line {}: {error}", index + 1))
        })
        .collect::<Result<Vec<_>>>()
        .map(|lines| lines.join("\n"))
}

fn single_block(kind: &'static str, text: String) -> Vec<DocumentTextBlock> {
    let text = text.trim().to_string();
    if text.is_empty() {
        return Vec::new();
    }
    let line_count = text.lines().count().max(1);
    vec![DocumentTextBlock {
        kind,
        text,
        location: DocumentLocation {
            line_start: Some(1),
            line_end: Some(line_count),
            ..DocumentLocation::default()
        },
    }]
}

fn line_blocks(raw: &str, kind: &'static str) -> Vec<DocumentTextBlock> {
    raw.lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let text = line.trim_end().to_string();
            (!text.trim().is_empty()).then_some(DocumentTextBlock {
                kind,
                text,
                location: DocumentLocation {
                    line_start: Some(index + 1),
                    line_end: Some(index + 1),
                    ..DocumentLocation::default()
                },
            })
        })
        .collect()
}

fn extract_markdown_blocks(raw: &str) -> Vec<DocumentTextBlock> {
    let heading = Regex::new(r"^(#{1,6})\s+(.+)$").expect("static markdown heading regex");
    let list_item = Regex::new(r"^(?:[-*+]\s+|[0-9]+[.)]\s+)").expect("static markdown list regex");
    let mut blocks = Vec::new();
    let mut section_path = Vec::<String>::new();
    let mut paragraph = Vec::<String>::new();
    let mut paragraph_start = 1usize;
    let mut code = Vec::<String>::new();
    let mut code_start = 1usize;
    let mut fence: Option<&str> = None;

    for (index, line) in raw.lines().enumerate() {
        let line_number = index + 1;
        let trimmed = line.trim();
        if let Some(active_fence) = fence {
            code.push(line.to_string());
            if trimmed.starts_with(active_fence) && code.len() > 1 {
                push_block(
                    &mut blocks,
                    "code_block",
                    &mut code,
                    code_start,
                    line_number,
                    &section_path,
                );
                fence = None;
            }
            continue;
        }
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            flush_paragraph(
                &mut blocks,
                &mut paragraph,
                paragraph_start,
                line_number.saturating_sub(1),
                &section_path,
            );
            code_start = line_number;
            code.push(line.to_string());
            fence = Some(if trimmed.starts_with("```") {
                "```"
            } else {
                "~~~"
            });
            continue;
        }
        if let Some(captures) = heading.captures(trimmed) {
            flush_paragraph(
                &mut blocks,
                &mut paragraph,
                paragraph_start,
                line_number.saturating_sub(1),
                &section_path,
            );
            let level = captures[1].len();
            section_path.truncate(level.saturating_sub(1).min(section_path.len()));
            section_path.push(captures[2].trim().to_string());
            blocks.push(DocumentTextBlock {
                kind: "heading",
                text: trimmed.to_string(),
                location: DocumentLocation {
                    line_start: Some(line_number),
                    line_end: Some(line_number),
                    section_path: section_path.clone(),
                    ..DocumentLocation::default()
                },
            });
            continue;
        }
        let is_table_row = trimmed.starts_with('|') && trimmed.ends_with('|');
        if list_item.is_match(trimmed) || is_table_row {
            flush_paragraph(
                &mut blocks,
                &mut paragraph,
                paragraph_start,
                line_number.saturating_sub(1),
                &section_path,
            );
            blocks.push(DocumentTextBlock {
                kind: if is_table_row {
                    "table_row"
                } else {
                    "list_item"
                },
                text: trimmed.to_string(),
                location: DocumentLocation {
                    line_start: Some(line_number),
                    line_end: Some(line_number),
                    section_path: section_path.clone(),
                    ..DocumentLocation::default()
                },
            });
            continue;
        }
        if trimmed.is_empty() {
            flush_paragraph(
                &mut blocks,
                &mut paragraph,
                paragraph_start,
                line_number.saturating_sub(1),
                &section_path,
            );
        } else {
            if paragraph.is_empty() {
                paragraph_start = line_number;
            }
            paragraph.push(line.to_string());
        }
    }
    let last_line = raw.lines().count().max(1);
    flush_paragraph(
        &mut blocks,
        &mut paragraph,
        paragraph_start,
        last_line,
        &section_path,
    );
    if !code.is_empty() {
        push_block(
            &mut blocks,
            "code_block",
            &mut code,
            code_start,
            last_line,
            &section_path,
        );
    }
    blocks
}

fn flush_paragraph(
    blocks: &mut Vec<DocumentTextBlock>,
    lines: &mut Vec<String>,
    line_start: usize,
    line_end: usize,
    section_path: &[String],
) {
    push_block(
        blocks,
        "paragraph",
        lines,
        line_start,
        line_end,
        section_path,
    );
}

fn push_block(
    blocks: &mut Vec<DocumentTextBlock>,
    kind: &'static str,
    lines: &mut Vec<String>,
    line_start: usize,
    line_end: usize,
    section_path: &[String],
) {
    if lines.is_empty() {
        return;
    }
    let text = std::mem::take(lines).join("\n").trim().to_string();
    if !text.is_empty() {
        blocks.push(DocumentTextBlock {
            kind,
            text,
            location: DocumentLocation {
                line_start: Some(line_start),
                line_end: Some(line_end.max(line_start)),
                section_path: section_path.to_vec(),
                ..DocumentLocation::default()
            },
        });
    }
}

fn extract_visible_html_blocks(raw: &str) -> Result<Vec<DocumentTextBlock>> {
    const HEADING_MARKER: &str = "\u{e000}heading:";
    const LIST_MARKER: &str = "\u{e000}list:";
    const TABLE_MARKER: &str = "\u{e000}table:";
    const CODE_MARKER: &str = "\u{e000}code:";
    let without_inert =
        Regex::new(r"(?is)<!--.*?-->|<script\b[^>]*>.*?</script\s*>|<style\b[^>]*>.*?</style\s*>")?
            .replace_all(raw, " ");
    let structural = Regex::new(r"(?is)<h([1-6])\b[^>]*>")?
        .replace_all(&without_inert, |captures: &regex::Captures<'_>| {
            format!("\n{HEADING_MARKER}{}:", &captures[1])
        });
    let structural =
        Regex::new(r"(?is)<li\b[^>]*>")?.replace_all(&structural, format!("\n{LIST_MARKER}"));
    let structural =
        Regex::new(r"(?is)<tr\b[^>]*>")?.replace_all(&structural, format!("\n{TABLE_MARKER}"));
    let structural = Regex::new(r"(?is)<(?:pre|code)\b[^>]*>")?
        .replace_all(&structural, format!("\n{CODE_MARKER}"));
    let structural = Regex::new(r"(?is)</(?:td|th)\s*>")?.replace_all(&structural, "\t");
    let structural = Regex::new(
        r"(?is)</?(?:h[1-6]|p|div|section|article|main|header|footer|li|ul|ol|tr|table|blockquote|pre|code|br)\b[^>]*>",
    )?
    .replace_all(&structural, "\n");
    let without_tags = Regex::new(r"(?is)<[^>]+>")?.replace_all(&structural, " ");
    let decoded = decode_html_entities(&without_tags)?;
    let mut blocks = Vec::new();
    let mut section_path = Vec::<String>::new();
    for (index, line) in decoded.lines().enumerate() {
        let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if line.is_empty() {
            continue;
        }
        let (kind, text) = if let Some(rest) = line.strip_prefix(HEADING_MARKER) {
            let (level, text) = rest.split_once(':').unwrap_or(("1", rest));
            let level = level.parse::<usize>().unwrap_or(1).clamp(1, 6);
            section_path.truncate(level.saturating_sub(1).min(section_path.len()));
            section_path.push(text.trim().to_string());
            ("heading", text.trim())
        } else if let Some(text) = line.strip_prefix(LIST_MARKER) {
            ("list_item", text.trim())
        } else if let Some(text) = line.strip_prefix(TABLE_MARKER) {
            ("table_row", text.trim())
        } else if let Some(text) = line.strip_prefix(CODE_MARKER) {
            ("code_block", text.trim())
        } else {
            ("visible_html", line.as_str())
        };
        if text.is_empty() {
            continue;
        }
        blocks.push(DocumentTextBlock {
            kind,
            text: text.to_string(),
            location: DocumentLocation {
                line_start: Some(index + 1),
                line_end: Some(index + 1),
                section_path: section_path.clone(),
                ..DocumentLocation::default()
            },
        });
    }
    Ok(blocks)
}

fn spreadsheet_row(reference: &str) -> Option<u32> {
    let first_digit = reference.find(|character: char| character.is_ascii_digit())?;
    reference[first_digit..].parse().ok()
}

fn is_source_code_extension(extension: &str) -> bool {
    matches!(
        extension,
        "c" | "cc"
            | "cpp"
            | "cs"
            | "go"
            | "h"
            | "hpp"
            | "java"
            | "js"
            | "jsx"
            | "kt"
            | "kts"
            | "lua"
            | "php"
            | "py"
            | "rb"
            | "rs"
            | "sh"
            | "swift"
            | "ts"
            | "tsx"
    )
}

fn decode_html_entities(raw: &str) -> Result<String> {
    let numeric = Regex::new(r"&#(?:(?P<hex>[xX][0-9a-fA-F]+)|(?P<dec>[0-9]+));")?;
    let decoded = numeric.replace_all(raw, |captures: &regex::Captures<'_>| {
        let value = captures
            .name("hex")
            .and_then(|value| u32::from_str_radix(&value.as_str()[1..], 16).ok())
            .or_else(|| {
                captures
                    .name("dec")
                    .and_then(|value| value.as_str().parse::<u32>().ok())
            });
        value
            .and_then(char::from_u32)
            .map(|character| character.to_string())
            .unwrap_or_else(|| captures[0].to_string())
    });
    Ok(decoded
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'"))
}

#[cfg(test)]
#[path = "ingest_extract_tests.rs"]
mod tests;
