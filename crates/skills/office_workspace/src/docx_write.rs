use crate::error::{OfficeError, OfficeResult};
use crate::operations::NormalizedOperation;
use crate::package::resolve_input_path;
use crate::range::format_column;
use quick_xml::escape::escape;
use serde_json::json;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

pub struct DocxWriteResult {
    pub members: BTreeMap<String, Vec<u8>>,
    pub changed_refs: Vec<String>,
    pub preservation: Vec<String>,
}

#[derive(Default)]
struct DocxParts {
    body: String,
    header: Option<String>,
    footer: Option<String>,
    relationships: Vec<(String, String, String, bool)>,
    media: Vec<(String, Vec<u8>)>,
    footnotes: Vec<String>,
    endnotes: Vec<String>,
    comments: Vec<String>,
    title: String,
    subject: String,
    creator: String,
    landscape: bool,
}

pub fn create_docx(operations: &[NormalizedOperation]) -> OfficeResult<DocxWriteResult> {
    let mut parts = DocxParts::default();
    apply_create_operations(&mut parts, operations)?;
    let members = build_docx_package(parts);
    Ok(DocxWriteResult {
        members,
        changed_refs: operations
            .iter()
            .flat_map(NormalizedOperation::object_refs)
            .collect(),
        preservation: vec!["new_package".to_string()],
    })
}

pub fn edit_docx(
    source_members: &BTreeMap<String, Vec<u8>>,
    operations: &[NormalizedOperation],
) -> OfficeResult<DocxWriteResult> {
    let mut members = source_members.clone();
    let document = member_text(&members, "word/document.xml")?.to_string();
    if !document.contains("<w:document") {
        return Err(OfficeError::unsupported(
            "DOCX mutation currently requires the standard w namespace prefix",
            json!({"part": "word/document.xml"}),
        ));
    }
    let mut document = document;
    let mut changed_refs = Vec::new();
    let mut append_parts = DocxParts::default();
    for operation in operations {
        match operation.kind.as_str() {
            "replace_block" => {
                let block_id = operation.string("block_id")?;
                let text = operation.string("text")?;
                let (part, index) = parse_word_object_id(block_id, "paragraph")?;
                let member = member_name_from_id_part(part)?;
                let xml = if member == "word/document.xml" {
                    &mut document
                } else {
                    let current = member_text(&members, &member)?.to_string();
                    members.insert(
                        member.clone(),
                        replace_element_text(&current, "w:p", index, text)?.into_bytes(),
                    );
                    changed_refs.push(block_id.to_string());
                    continue;
                };
                *xml = replace_element_text(xml, "w:p", index, text)?;
                changed_refs.push(block_id.to_string());
            }
            "delete_block" => {
                let block_id = operation.string("block_id")?;
                let (part, index) = parse_word_object_id(block_id, "paragraph")?;
                let member = member_name_from_id_part(part)?;
                if member == "word/document.xml" {
                    document = delete_element(&document, "w:p", index)?;
                } else {
                    let current = member_text(&members, &member)?.to_string();
                    members.insert(member, delete_element(&current, "w:p", index)?.into_bytes());
                }
                changed_refs.push(block_id.to_string());
            }
            "set_block_style" => {
                let block_id = operation.string("block_id")?;
                let style = operation.string("style")?;
                let (part, index) = parse_word_object_id(block_id, "paragraph")?;
                let member = member_name_from_id_part(part)?;
                if member == "word/document.xml" {
                    document = set_paragraph_style(&document, index, style)?;
                } else {
                    let current = member_text(&members, &member)?.to_string();
                    members.insert(
                        member,
                        set_paragraph_style(&current, index, style)?.into_bytes(),
                    );
                }
                changed_refs.push(block_id.to_string());
            }
            "replace_match" => {
                let block_id = operation.string("block_id")?;
                let expected = operation.string("expected_text")?;
                let replacement = operation.string("text")?;
                let (part, index) = parse_word_object_id(block_id, "paragraph")?;
                let member = member_name_from_id_part(part)?;
                let current = if member == "word/document.xml" {
                    document.clone()
                } else {
                    member_text(&members, &member)?.to_string()
                };
                let updated =
                    replace_text_in_element(&current, "w:p", index, expected, replacement)?;
                if member == "word/document.xml" {
                    document = updated;
                } else {
                    members.insert(member, updated.into_bytes());
                }
                changed_refs.push(block_id.to_string());
            }
            "table_set_cell" => {
                let table_id = operation.string("table_id")?;
                let row = operation.usize("row")?;
                let column = operation.usize("column")?;
                let value = operation.string("text")?;
                let (part, index) = parse_word_object_id(table_id, "table")?;
                if part != "word_document" {
                    return Err(OfficeError::unsupported(
                        "table cell editing is limited to word/document.xml",
                        json!({"table_id": table_id}),
                    ));
                }
                document = set_table_cell(&document, index, row, column, value)?;
                changed_refs.push(format!(
                    "{table_id}:{}{}",
                    format_column((column + 1) as u32),
                    row + 1
                ));
            }
            "replace_image" => {
                let media_id = operation.string("media_id")?;
                let image_path = resolve_input_path(operation.string("path")?)?;
                replace_image(&mut members, media_id, &image_path)?;
                changed_refs.push(media_id.to_string());
            }
            "set_properties" => update_properties(&mut members, operation)?,
            "set_header" => {
                let text = operation.string("text")?;
                members.insert("word/header1.xml".into(), header_xml(text).into_bytes());
                ensure_document_relation(
                    &mut members,
                    "header",
                    "header1.xml",
                    "rIdRustClawHeader",
                )?;
                changed_refs.push("word_header1".to_string());
            }
            "set_footer" => {
                let text = operation.string("text")?;
                members.insert("word/footer1.xml".into(), footer_xml(text).into_bytes());
                ensure_document_relation(
                    &mut members,
                    "footer",
                    "footer1.xml",
                    "rIdRustClawFooter",
                )?;
                changed_refs.push("word_footer1".to_string());
            }
            "set_section" => {
                document = set_section(&document, operation)?;
                changed_refs.push("word_document_section".to_string());
            }
            kind if is_append_operation(kind) => {
                apply_create_operation(&mut append_parts, operation)?;
            }
            _ => {
                return Err(OfficeError::unsupported(
                    "DOCX edit operation is not implemented without potential format loss",
                    json!({"operation_id": operation.id, "op": operation.kind}),
                ))
            }
        }
    }
    if !append_parts.body.is_empty() {
        document = append_to_document(&document, &append_parts.body)?;
        changed_refs.push("word_document_body".to_string());
    }
    merge_auxiliary_parts(&mut members, append_parts)?;
    members.insert("word/document.xml".to_string(), document.into_bytes());
    Ok(DocxWriteResult {
        members,
        changed_refs,
        preservation: vec![
            "unknown_package_parts_preserved".to_string(),
            "untouched_xml_parts_preserved".to_string(),
        ],
    })
}

fn apply_create_operations(
    parts: &mut DocxParts,
    operations: &[NormalizedOperation],
) -> OfficeResult<()> {
    for operation in operations {
        apply_create_operation(parts, operation)?;
    }
    Ok(())
}

fn apply_create_operation(
    parts: &mut DocxParts,
    operation: &NormalizedOperation,
) -> OfficeResult<()> {
    match operation.kind.as_str() {
        "set_properties" => {
            parts.title = operation.optional_string("title").unwrap_or("").to_string();
            parts.subject = operation
                .optional_string("subject")
                .unwrap_or("")
                .to_string();
            parts.creator = operation
                .optional_string("creator")
                .unwrap_or("")
                .to_string();
        }
        "set_section" => {
            parts.landscape = operation
                .optional_string("orientation")
                .is_some_and(|value| value == "landscape");
        }
        "set_header" => parts.header = Some(operation.string("text")?.to_string()),
        "set_footer" => parts.footer = Some(operation.string("text")?.to_string()),
        "add_heading" => {
            let text = operation.string("text")?;
            let level = operation.optional_usize("level").unwrap_or(1).clamp(1, 9);
            parts
                .body
                .push_str(&paragraph_xml(text, Some(&format!("Heading{level}")), None));
        }
        "add_paragraph" => {
            parts.body.push_str(&paragraph_xml(
                operation.string("text")?,
                operation.optional_string("style"),
                None,
            ));
        }
        "add_list_item" => {
            let level = operation.optional_usize("level").unwrap_or(0).min(8);
            parts.body.push_str(&paragraph_xml(
                operation.string("text")?,
                operation.optional_string("style"),
                Some(level),
            ));
        }
        "add_table" => {
            let rows = operation
                .value("rows")
                .and_then(ValueRows::new)
                .ok_or_else(|| invalid_rows(operation))?;
            parts.body.push_str(&table_xml(rows));
        }
        "add_image" => add_image(parts, operation)?,
        "add_hyperlink" => {
            let text = operation.string("text")?;
            let url = operation.string("url")?;
            let id = format!("rIdRustClawLink{}", parts.relationships.len() + 1);
            parts
                .relationships
                .push((id.clone(), "hyperlink".to_string(), url.to_string(), true));
            parts.body.push_str(&format!(
                "<w:p><w:hyperlink r:id=\"{}\"><w:r><w:rPr><w:color w:val=\"0563C1\"/><w:u w:val=\"single\"/></w:rPr><w:t>{}</w:t></w:r></w:hyperlink></w:p>",
                xml(&id),
                xml(text)
            ));
        }
        "add_bookmark" => {
            let name = operation.string("name")?;
            let text = operation.string("text")?;
            let id = operation.optional_usize("bookmark_id").unwrap_or(1);
            parts.body.push_str(&format!(
                "<w:p><w:bookmarkStart w:id=\"{id}\" w:name=\"{}\"/><w:r><w:t>{}</w:t></w:r><w:bookmarkEnd w:id=\"{id}\"/></w:p>",
                xml(name),
                xml(text)
            ));
        }
        "add_footnote" => {
            let id = parts.footnotes.len() + 1;
            parts.footnotes.push(operation.string("text")?.to_string());
            parts.body.push_str(&format!(
                "<w:p><w:r><w:footnoteReference w:id=\"{id}\"/></w:r></w:p>"
            ));
        }
        "add_endnote" => {
            let id = parts.endnotes.len() + 1;
            parts.endnotes.push(operation.string("text")?.to_string());
            parts.body.push_str(&format!(
                "<w:p><w:r><w:endnoteReference w:id=\"{id}\"/></w:r></w:p>"
            ));
        }
        "add_comment" => {
            let id = parts.comments.len();
            let comment = operation.string("comment")?;
            let text = operation.string("text")?;
            parts.comments.push(comment.to_string());
            parts.body.push_str(&format!(
                "<w:p><w:commentRangeStart w:id=\"{id}\"/><w:r><w:t>{}</w:t></w:r><w:commentRangeEnd w:id=\"{id}\"/><w:r><w:commentReference w:id=\"{id}\"/></w:r></w:p>",
                xml(text)
            ));
        }
        "add_page_break" => {
            parts
                .body
                .push_str("<w:p><w:r><w:br w:type=\"page\"/></w:r></w:p>");
        }
        "add_section_break" => {
            parts.body.push_str(
                "<w:p><w:pPr><w:sectPr><w:type w:val=\"nextPage\"/></w:sectPr></w:pPr></w:p>",
            );
        }
        _ => {
            return Err(OfficeError::unsupported(
                "DOCX create operation is not implemented",
                json!({"operation_id": operation.id, "op": operation.kind}),
            ))
        }
    }
    Ok(())
}

struct ValueRows<'a>(&'a [serde_json::Value]);

impl<'a> ValueRows<'a> {
    fn new(value: &'a serde_json::Value) -> Option<Self> {
        value.as_array().map(|rows| Self(rows.as_slice()))
    }
}

fn invalid_rows(operation: &NormalizedOperation) -> OfficeError {
    OfficeError::new(
        "invalid_operation",
        "add_table requires rows as an array of arrays",
        json!({"operation_id": operation.id}),
    )
}

fn table_xml(rows: ValueRows<'_>) -> String {
    let mut output = String::from(
        "<w:tbl><w:tblPr><w:tblW w:w=\"0\" w:type=\"auto\"/><w:tblBorders><w:top w:val=\"single\" w:sz=\"4\"/><w:left w:val=\"single\" w:sz=\"4\"/><w:bottom w:val=\"single\" w:sz=\"4\"/><w:right w:val=\"single\" w:sz=\"4\"/><w:insideH w:val=\"single\" w:sz=\"4\"/><w:insideV w:val=\"single\" w:sz=\"4\"/></w:tblBorders></w:tblPr>",
    );
    for row in rows.0 {
        output.push_str("<w:tr>");
        if let Some(cells) = row.as_array() {
            for cell in cells {
                let value = scalar_text(cell);
                output.push_str(&format!(
                    "<w:tc><w:tcPr/><w:p><w:r><w:t>{}</w:t></w:r></w:p></w:tc>",
                    xml(&value)
                ));
            }
        }
        output.push_str("</w:tr>");
    }
    output.push_str("</w:tbl>");
    output
}

fn add_image(parts: &mut DocxParts, operation: &NormalizedOperation) -> OfficeResult<()> {
    let path = resolve_input_path(operation.string("path")?)?;
    let bytes = fs::read(&path).map_err(|error| {
        OfficeError::new(
            "source_unavailable",
            format!("cannot read image source: {error}"),
            json!({"path": path.display().to_string()}),
        )
    })?;
    let extension = image_extension(&path)?;
    let index = parts.media.len() + 1;
    let member = format!("word/media/image{index}.{extension}");
    let relation_id = format!("rIdRustClawImage{index}");
    parts.media.push((member.clone(), bytes));
    parts.relationships.push((
        relation_id.clone(),
        "image".to_string(),
        format!("media/image{index}.{extension}"),
        false,
    ));
    let alt = operation.optional_string("alt").unwrap_or("image");
    parts.body.push_str(&drawing_paragraph(
        &relation_id,
        index,
        alt,
        operation.optional_usize("width_emu").unwrap_or(3_600_000),
        operation.optional_usize("height_emu").unwrap_or(2_400_000),
    ));
    if let Some(caption) = operation.optional_string("caption") {
        parts
            .body
            .push_str(&paragraph_xml(caption, Some("Caption"), None));
    }
    Ok(())
}

fn build_docx_package(parts: DocxParts) -> BTreeMap<String, Vec<u8>> {
    let mut members = BTreeMap::new();
    let mut content_types = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/><Override PartName="/docProps/core.xml" ContentType="application/vnd.openxmlformats-package.core-properties+xml"/><Override PartName="/docProps/app.xml" ContentType="application/vnd.openxmlformats-officedocument.extended-properties+xml"/>"#,
    );
    if parts.header.is_some() {
        content_types.push_str(r#"<Override PartName="/word/header1.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml"/>"#);
    }
    if parts.footer.is_some() {
        content_types.push_str(r#"<Override PartName="/word/footer1.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml"/>"#);
    }
    if !parts.footnotes.is_empty() {
        content_types.push_str(r#"<Override PartName="/word/footnotes.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.footnotes+xml"/>"#);
    }
    if !parts.endnotes.is_empty() {
        content_types.push_str(r#"<Override PartName="/word/endnotes.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.endnotes+xml"/>"#);
    }
    if !parts.comments.is_empty() {
        content_types.push_str(r#"<Override PartName="/word/comments.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.comments+xml"/>"#);
    }
    for (name, _) in &parts.media {
        let extension = Path::new(name)
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("png");
        content_types.push_str(&format!(
            "<Default Extension=\"{}\" ContentType=\"{}\"/>",
            xml(extension),
            image_content_type(extension)
        ));
    }
    content_types.push_str("</Types>");
    members.insert("[Content_Types].xml".into(), content_types.into_bytes());
    members.insert(
        "_rels/.rels".into(),
        br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/><Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties" Target="docProps/app.xml"/></Relationships>"#.to_vec(),
    );
    members.insert(
        "docProps/core.xml".into(),
        core_properties_xml(&parts).into_bytes(),
    );
    members.insert(
        "docProps/app.xml".into(),
        br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties"><Application>RustClaw</Application></Properties>"#.to_vec(),
    );
    members.insert(
        "word/styles.xml".into(),
        default_styles_xml().as_bytes().to_vec(),
    );
    let (section, header_ref, footer_ref) = section_xml(
        parts.landscape,
        parts.header.is_some(),
        parts.footer.is_some(),
    );
    members.insert(
        "word/document.xml".into(),
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" xmlns:wp=\"http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing\" xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" xmlns:pic=\"http://schemas.openxmlformats.org/drawingml/2006/picture\"><w:body>{}{}{}{}</w:body></w:document>",
            parts.body, header_ref, footer_ref, section
        )
        .into_bytes(),
    );
    let mut relationships = vec![(
        "rIdStyles".to_string(),
        "styles".to_string(),
        "styles.xml".to_string(),
        false,
    )];
    relationships.extend(parts.relationships.clone());
    if parts.header.is_some() {
        relationships.push((
            "rIdRustClawHeader".into(),
            "header".into(),
            "header1.xml".into(),
            false,
        ));
    }
    if parts.footer.is_some() {
        relationships.push((
            "rIdRustClawFooter".into(),
            "footer".into(),
            "footer1.xml".into(),
            false,
        ));
    }
    add_note_relationships(&parts, &mut relationships);
    members.insert(
        "word/_rels/document.xml.rels".into(),
        relationships_xml(&relationships).into_bytes(),
    );
    if let Some(header) = parts.header {
        members.insert("word/header1.xml".into(), header_xml(&header).into_bytes());
    }
    if let Some(footer) = parts.footer {
        members.insert("word/footer1.xml".into(), footer_xml(&footer).into_bytes());
    }
    if !parts.footnotes.is_empty() {
        members.insert(
            "word/footnotes.xml".into(),
            notes_xml("footnotes", "footnote", &parts.footnotes).into_bytes(),
        );
    }
    if !parts.endnotes.is_empty() {
        members.insert(
            "word/endnotes.xml".into(),
            notes_xml("endnotes", "endnote", &parts.endnotes).into_bytes(),
        );
    }
    if !parts.comments.is_empty() {
        members.insert(
            "word/comments.xml".into(),
            comments_xml(&parts.comments).into_bytes(),
        );
    }
    for (name, bytes) in parts.media {
        members.insert(name, bytes);
    }
    members
}

fn add_note_relationships(
    parts: &DocxParts,
    relationships: &mut Vec<(String, String, String, bool)>,
) {
    if !parts.footnotes.is_empty() {
        relationships.push((
            "rIdRustClawFootnotes".into(),
            "footnotes".into(),
            "footnotes.xml".into(),
            false,
        ));
    }
    if !parts.endnotes.is_empty() {
        relationships.push((
            "rIdRustClawEndnotes".into(),
            "endnotes".into(),
            "endnotes.xml".into(),
            false,
        ));
    }
    if !parts.comments.is_empty() {
        relationships.push((
            "rIdRustClawComments".into(),
            "comments".into(),
            "comments.xml".into(),
            false,
        ));
    }
}

fn merge_auxiliary_parts(
    members: &mut BTreeMap<String, Vec<u8>>,
    parts: DocxParts,
) -> OfficeResult<()> {
    if let Some(header) = parts.header {
        members.insert("word/header1.xml".into(), header_xml(&header).into_bytes());
        ensure_document_relation(members, "header", "header1.xml", "rIdRustClawHeader")?;
    }
    if let Some(footer) = parts.footer {
        members.insert("word/footer1.xml".into(), footer_xml(&footer).into_bytes());
        ensure_document_relation(members, "footer", "footer1.xml", "rIdRustClawFooter")?;
    }
    for (name, bytes) in parts.media {
        members.insert(name, bytes);
    }
    if !parts.relationships.is_empty() {
        let rels_name = "word/_rels/document.xml.rels";
        let mut rels = members
            .get(rels_name)
            .and_then(|bytes| std::str::from_utf8(bytes).ok())
            .unwrap_or(r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"></Relationships>"#)
            .to_string();
        for (id, kind, target, external) in parts.relationships {
            let relation = relationship_xml(&id, &kind, &target, external);
            rels = insert_before(&rels, "</Relationships>", &relation)?;
        }
        members.insert(rels_name.into(), rels.into_bytes());
    }
    Ok(())
}

fn replace_element_text(
    xml_text: &str,
    tag: &str,
    one_based_index: usize,
    replacement: &str,
) -> OfficeResult<String> {
    let range = nth_element_range(xml_text, tag, one_based_index)?;
    let element = &xml_text[range.0..range.1];
    let replaced = replace_all_text_nodes(element, replacement)?;
    Ok(format!(
        "{}{}{}",
        &xml_text[..range.0],
        replaced,
        &xml_text[range.1..]
    ))
}

fn replace_text_in_element(
    xml_text: &str,
    tag: &str,
    one_based_index: usize,
    expected: &str,
    replacement: &str,
) -> OfficeResult<String> {
    let range = nth_element_range(xml_text, tag, one_based_index)?;
    let element = &xml_text[range.0..range.1];
    let plain = text_nodes(element).join("");
    if !plain.contains(expected) {
        return Err(OfficeError::new(
            "source_conflict",
            "expected text is absent from the selected object revision",
            json!({"expected_text": expected, "object_index": one_based_index}),
        ));
    }
    let replaced_plain = plain.replacen(expected, replacement, 1);
    let replaced = replace_all_text_nodes(element, &replaced_plain)?;
    Ok(format!(
        "{}{}{}",
        &xml_text[..range.0],
        replaced,
        &xml_text[range.1..]
    ))
}

fn replace_all_text_nodes(element: &str, replacement: &str) -> OfficeResult<String> {
    let mut output = String::with_capacity(element.len() + replacement.len());
    let mut cursor = 0usize;
    let mut wrote = false;
    while let Some(relative) = element[cursor..].find("<w:t") {
        let start = cursor + relative;
        let Some(open_end_relative) = element[start..].find('>') else {
            break;
        };
        let open_end = start + open_end_relative + 1;
        let Some(close_relative) = element[open_end..].find("</w:t>") else {
            break;
        };
        let close = open_end + close_relative;
        output.push_str(&element[cursor..open_end]);
        if !wrote {
            output.push_str(&xml(replacement));
            wrote = true;
        }
        output.push_str("</w:t>");
        cursor = close + "</w:t>".len();
    }
    output.push_str(&element[cursor..]);
    if wrote {
        return Ok(output);
    }
    let insertion = format!("<w:r><w:t>{}</w:t></w:r>", xml(replacement));
    insert_before(element, "</w:p>", &insertion)
}

fn delete_element(xml_text: &str, tag: &str, one_based_index: usize) -> OfficeResult<String> {
    let range = nth_element_range(xml_text, tag, one_based_index)?;
    Ok(format!("{}{}", &xml_text[..range.0], &xml_text[range.1..]))
}

fn set_paragraph_style(xml_text: &str, index: usize, style: &str) -> OfficeResult<String> {
    let range = nth_element_range(xml_text, "w:p", index)?;
    let element = &xml_text[range.0..range.1];
    let style_xml = format!("<w:pStyle w:val=\"{}\"/>", xml(style));
    let updated = if let Some(start) = element.find("<w:pStyle") {
        let end = element[start..]
            .find("/>")
            .map(|relative| start + relative + 2)
            .ok_or_else(|| malformed_selector("w:pStyle"))?;
        format!("{}{}{}", &element[..start], style_xml, &element[end..])
    } else if element.contains("<w:pPr>") {
        element.replacen("<w:pPr>", &format!("<w:pPr>{style_xml}"), 1)
    } else {
        element.replacen("<w:p", &format!("<w:p><w:pPr>{style_xml}</w:pPr>"), 1)
    };
    Ok(format!(
        "{}{}{}",
        &xml_text[..range.0],
        updated,
        &xml_text[range.1..]
    ))
}

fn set_table_cell(
    document: &str,
    table_index: usize,
    row: usize,
    column: usize,
    text: &str,
) -> OfficeResult<String> {
    let table_range = nth_element_range(document, "w:tbl", table_index)?;
    let table = &document[table_range.0..table_range.1];
    let row_range = nth_element_range(table, "w:tr", row + 1)?;
    let row_xml = &table[row_range.0..row_range.1];
    let cell_range = nth_element_range(row_xml, "w:tc", column + 1)?;
    let cell = &row_xml[cell_range.0..cell_range.1];
    let cell = replace_all_text_nodes(cell, text)?;
    let row_xml = format!(
        "{}{}{}",
        &row_xml[..cell_range.0],
        cell,
        &row_xml[cell_range.1..]
    );
    let table = format!(
        "{}{}{}",
        &table[..row_range.0],
        row_xml,
        &table[row_range.1..]
    );
    Ok(format!(
        "{}{}{}",
        &document[..table_range.0],
        table,
        &document[table_range.1..]
    ))
}

fn nth_element_range(xml: &str, tag: &str, one_based_index: usize) -> OfficeResult<(usize, usize)> {
    if one_based_index == 0 {
        return Err(OfficeError::invalid("object identifiers are one-based"));
    }
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut cursor = 0usize;
    let mut current = 0usize;
    while let Some(relative) = xml[cursor..].find(&open) {
        let start = cursor + relative;
        let boundary = xml.as_bytes().get(start + open.len()).copied();
        if !matches!(boundary, Some(b'>') | Some(b' ') | Some(b'/')) {
            cursor = start + open.len();
            continue;
        }
        current += 1;
        let open_end = xml[start..]
            .find('>')
            .map(|relative| start + relative + 1)
            .ok_or_else(|| malformed_selector(tag))?;
        let end = if xml[start..open_end].trim_end().ends_with("/>") {
            open_end
        } else {
            xml[open_end..]
                .find(&close)
                .map(|relative| open_end + relative + close.len())
                .ok_or_else(|| malformed_selector(tag))?
        };
        if current == one_based_index {
            return Ok((start, end));
        }
        cursor = end;
    }
    Err(OfficeError::new(
        "object_not_found",
        "selected OOXML object does not exist",
        json!({"tag": tag, "index": one_based_index}),
    ))
}

fn parse_word_object_id<'a>(object_id: &'a str, kind: &str) -> OfficeResult<(&'a str, usize)> {
    let marker = format!("_{kind}_");
    let (part, index) = object_id.rsplit_once(&marker).ok_or_else(|| {
        OfficeError::new(
            "invalid_selector",
            "object identifier does not match the expected Word object kind",
            json!({"object_id": object_id, "kind": kind}),
        )
    })?;
    let index = index.parse::<usize>().map_err(|_| {
        OfficeError::new(
            "invalid_selector",
            "object identifier has an invalid index",
            json!({"object_id": object_id}),
        )
    })?;
    Ok((part, index))
}

fn member_name_from_id_part(part: &str) -> OfficeResult<String> {
    if part == "word_document" {
        Ok("word/document.xml".to_string())
    } else if let Some(name) = part.strip_prefix("word_header") {
        Ok(format!("word/header{name}.xml"))
    } else if let Some(name) = part.strip_prefix("word_footer") {
        Ok(format!("word/footer{name}.xml"))
    } else if part == "word_footnotes" {
        Ok("word/footnotes.xml".to_string())
    } else if part == "word_endnotes" {
        Ok("word/endnotes.xml".to_string())
    } else if part == "word_comments" {
        Ok("word/comments.xml".to_string())
    } else {
        Err(OfficeError::new(
            "invalid_selector",
            "object identifier refers to an unsupported Word package part",
            json!({"part": part}),
        ))
    }
}

fn append_to_document(document: &str, body: &str) -> OfficeResult<String> {
    if let Some(section) = document.find("<w:sectPr") {
        Ok(format!(
            "{}{}{}",
            &document[..section],
            body,
            &document[section..]
        ))
    } else {
        insert_before(document, "</w:body>", body)
    }
}

fn replace_image(
    members: &mut BTreeMap<String, Vec<u8>>,
    media_id: &str,
    source: &Path,
) -> OfficeResult<()> {
    let index = media_id
        .strip_prefix("media_")
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|index| *index > 0)
        .ok_or_else(|| OfficeError::invalid("media_id must use media_<index> format"))?;
    let member = members
        .keys()
        .filter(|name| name.starts_with("word/media/"))
        .nth(index - 1)
        .cloned()
        .ok_or_else(|| {
            OfficeError::new(
                "object_not_found",
                "selected image does not exist",
                json!({"media_id": media_id}),
            )
        })?;
    let source_extension = image_extension(source)?;
    let member_extension = Path::new(&member)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    if source_extension != member_extension {
        return Err(OfficeError::unsupported(
            "image replacement must preserve the package media type",
            json!({
                "media_id": media_id,
                "existing_extension": member_extension,
                "source_extension": source_extension
            }),
        ));
    }
    let bytes = fs::read(source).map_err(|error| {
        OfficeError::new(
            "source_unavailable",
            format!("cannot read replacement image: {error}"),
            json!({"path": source.display().to_string()}),
        )
    })?;
    members.insert(member, bytes);
    Ok(())
}

fn update_properties(
    members: &mut BTreeMap<String, Vec<u8>>,
    operation: &NormalizedOperation,
) -> OfficeResult<()> {
    let mut properties = member_text(members, "docProps/core.xml")
        .unwrap_or(
            r#"<?xml version="1.0"?><cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/"></cp:coreProperties>"#,
        )
        .to_string();
    for (field, tag) in [
        ("title", "dc:title"),
        ("subject", "dc:subject"),
        ("creator", "dc:creator"),
    ] {
        if let Some(value) = operation.optional_string(field) {
            properties =
                set_or_insert_simple_element(&properties, tag, value, "</cp:coreProperties>")?;
        }
    }
    members.insert("docProps/core.xml".into(), properties.into_bytes());
    Ok(())
}

fn set_section(document: &str, operation: &NormalizedOperation) -> OfficeResult<String> {
    let landscape = operation
        .optional_string("orientation")
        .is_some_and(|value| value == "landscape");
    let (section, _, _) = section_xml(landscape, false, false);
    if let Some(start) = document.find("<w:sectPr") {
        let end = document[start..]
            .find("</w:sectPr>")
            .map(|relative| start + relative + "</w:sectPr>".len())
            .ok_or_else(|| malformed_selector("w:sectPr"))?;
        Ok(format!(
            "{}{}{}",
            &document[..start],
            section,
            &document[end..]
        ))
    } else {
        insert_before(document, "</w:body>", &section)
    }
}

fn ensure_document_relation(
    members: &mut BTreeMap<String, Vec<u8>>,
    kind: &str,
    target: &str,
    id: &str,
) -> OfficeResult<()> {
    let name = "word/_rels/document.xml.rels";
    let current = members
        .get(name)
        .and_then(|bytes| std::str::from_utf8(bytes).ok())
        .unwrap_or(r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"></Relationships>"#);
    if current.contains(&format!("Id=\"{}\"", xml(id))) {
        return Ok(());
    }
    let updated = insert_before(
        current,
        "</Relationships>",
        &relationship_xml(id, kind, target, false),
    )?;
    members.insert(name.into(), updated.into_bytes());
    Ok(())
}

fn set_or_insert_simple_element(
    xml_text: &str,
    tag: &str,
    value: &str,
    parent_close: &str,
) -> OfficeResult<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    if let Some(start) = xml_text.find(&open) {
        let content_start = start + open.len();
        let end = xml_text[content_start..]
            .find(&close)
            .map(|relative| content_start + relative)
            .ok_or_else(|| malformed_selector(tag))?;
        return Ok(format!(
            "{}{}{}",
            &xml_text[..content_start],
            xml(value),
            &xml_text[end..]
        ));
    }
    insert_before(
        xml_text,
        parent_close,
        &format!("<{tag}>{}</{tag}>", xml(value)),
    )
}

fn insert_before(source: &str, needle: &str, content: &str) -> OfficeResult<String> {
    let index = source.rfind(needle).ok_or_else(|| {
        OfficeError::new(
            "malformed_xml",
            "required closing element is missing",
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

fn text_nodes(element: &str) -> Vec<String> {
    let mut output = Vec::new();
    let mut cursor = 0usize;
    while let Some(relative) = element[cursor..].find("<w:t") {
        let start = cursor + relative;
        let Some(open_end_relative) = element[start..].find('>') else {
            break;
        };
        let open_end = start + open_end_relative + 1;
        let Some(close_relative) = element[open_end..].find("</w:t>") else {
            break;
        };
        let close = open_end + close_relative;
        output.push(element[open_end..close].to_string());
        cursor = close + "</w:t>".len();
    }
    output
}

fn member_text<'a>(members: &'a BTreeMap<String, Vec<u8>>, name: &str) -> OfficeResult<&'a str> {
    members
        .get(name)
        .ok_or_else(|| {
            OfficeError::new(
                "missing_package_part",
                "required DOCX package part is missing",
                json!({"member": name}),
            )
        })
        .and_then(|bytes| {
            std::str::from_utf8(bytes).map_err(|error| {
                OfficeError::new(
                    "malformed_xml",
                    format!("DOCX package part is not UTF-8 XML: {error}"),
                    json!({"member": name}),
                )
            })
        })
}

fn is_append_operation(kind: &str) -> bool {
    matches!(
        kind,
        "add_heading"
            | "add_paragraph"
            | "add_list_item"
            | "add_table"
            | "add_image"
            | "add_hyperlink"
            | "add_bookmark"
            | "add_footnote"
            | "add_endnote"
            | "add_comment"
            | "add_page_break"
            | "add_section_break"
    )
}

fn paragraph_xml(text: &str, style: Option<&str>, list_level: Option<usize>) -> String {
    let mut properties = String::new();
    if let Some(style) = style {
        properties.push_str(&format!("<w:pStyle w:val=\"{}\"/>", xml(style)));
    }
    if let Some(level) = list_level {
        properties.push_str(&format!(
            "<w:numPr><w:ilvl w:val=\"{level}\"/><w:numId w:val=\"1\"/></w:numPr>"
        ));
    }
    let properties = (!properties.is_empty())
        .then(|| format!("<w:pPr>{properties}</w:pPr>"))
        .unwrap_or_default();
    format!(
        "<w:p>{properties}<w:r><w:t xml:space=\"preserve\">{}</w:t></w:r></w:p>",
        xml(text)
    )
}

fn drawing_paragraph(
    relation_id: &str,
    index: usize,
    alt: &str,
    width: usize,
    height: usize,
) -> String {
    format!(
        "<w:p><w:r><w:drawing><wp:inline><wp:extent cx=\"{width}\" cy=\"{height}\"/><wp:docPr id=\"{index}\" name=\"Image {index}\" descr=\"{}\"/><a:graphic><a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/picture\"><pic:pic><pic:nvPicPr><pic:cNvPr id=\"{index}\" name=\"Image {index}\"/></pic:nvPicPr><pic:blipFill><a:blip r:embed=\"{}\"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{width}\" cy=\"{height}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>",
        xml(alt),
        xml(relation_id)
    )
}

fn section_xml(landscape: bool, header: bool, footer: bool) -> (String, String, String) {
    let (width, height, orient) = if landscape {
        (15_840, 12_240, " w:orient=\"landscape\"")
    } else {
        (12_240, 15_840, "")
    };
    let header_ref = if header {
        "<w:headerReference w:type=\"default\" r:id=\"rIdRustClawHeader\"/>".to_string()
    } else {
        String::new()
    };
    let footer_ref = if footer {
        "<w:footerReference w:type=\"default\" r:id=\"rIdRustClawFooter\"/>".to_string()
    } else {
        String::new()
    };
    (
        format!(
            "<w:sectPr>{header_ref}{footer_ref}<w:pgSz w:w=\"{width}\" w:h=\"{height}\"{orient}/><w:pgMar w:top=\"1440\" w:right=\"1440\" w:bottom=\"1440\" w:left=\"1440\"/></w:sectPr>"
        ),
        String::new(),
        String::new(),
    )
}

fn header_xml(text: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><w:hdr xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:p><w:r><w:t>{}</w:t></w:r></w:p></w:hdr>",
        xml(text)
    )
}

fn footer_xml(text: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><w:ftr xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:p><w:r><w:t>{}</w:t></w:r></w:p></w:ftr>",
        xml(text)
    )
}

fn notes_xml(root: &str, item: &str, values: &[String]) -> String {
    let mut output = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><w:{root} xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">"
    );
    for (index, value) in values.iter().enumerate() {
        output.push_str(&format!(
            "<w:{item} w:id=\"{}\"><w:p><w:r><w:t>{}</w:t></w:r></w:p></w:{item}>",
            index + 1,
            xml(value)
        ));
    }
    output.push_str(&format!("</w:{root}>"));
    output
}

fn comments_xml(values: &[String]) -> String {
    let mut output = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><w:comments xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">",
    );
    for (index, value) in values.iter().enumerate() {
        output.push_str(&format!(
            "<w:comment w:id=\"{index}\" w:author=\"RustClaw\"><w:p><w:r><w:t>{}</w:t></w:r></w:p></w:comment>",
            xml(value)
        ));
    }
    output.push_str("</w:comments>");
    output
}

fn relationships_xml(values: &[(String, String, String, bool)]) -> String {
    let mut output = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">",
    );
    for (id, kind, target, external) in values {
        output.push_str(&relationship_xml(id, kind, target, *external));
    }
    output.push_str("</Relationships>");
    output
}

fn relationship_xml(id: &str, kind: &str, target: &str, external: bool) -> String {
    format!(
        "<Relationship Id=\"{}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/{}\" Target=\"{}\"{}/>",
        xml(id),
        xml(kind),
        xml(target),
        if external {
            " TargetMode=\"External\""
        } else {
            ""
        }
    )
}

fn core_properties_xml(parts: &DocxParts) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><cp:coreProperties xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\"><dc:title>{}</dc:title><dc:subject>{}</dc:subject><dc:creator>{}</dc:creator></cp:coreProperties>",
        xml(&parts.title),
        xml(&parts.subject),
        xml(&parts.creator)
    )
}

fn default_styles_xml() -> &'static str {
    r#"<?xml version="1.0" encoding="UTF-8"?><w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style><w:style w:type="paragraph" w:styleId="Title"><w:name w:val="Title"/><w:basedOn w:val="Normal"/><w:rPr><w:b/><w:sz w:val="36"/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="Caption"><w:name w:val="Caption"/><w:basedOn w:val="Normal"/><w:rPr><w:i/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/><w:rPr><w:b/><w:sz w:val="32"/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="Heading2"><w:name w:val="heading 2"/><w:basedOn w:val="Normal"/><w:rPr><w:b/><w:sz w:val="28"/></w:rPr></w:style></w:styles>"#
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
            "supported Office image inputs are PNG, JPEG, and GIF",
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

fn scalar_text(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => String::new(),
        serde_json::Value::String(value) => value.clone(),
        serde_json::Value::Bool(value) => value.to_string(),
        serde_json::Value::Number(value) => value.to_string(),
        other => other.to_string(),
    }
}

fn xml(value: &str) -> String {
    escape(value).into_owned()
}

fn malformed_selector(element: &str) -> OfficeError {
    OfficeError::new(
        "malformed_xml",
        "selected OOXML element is malformed",
        json!({"element": element}),
    )
}

#[cfg(test)]
#[path = "docx_write_tests.rs"]
mod tests;
