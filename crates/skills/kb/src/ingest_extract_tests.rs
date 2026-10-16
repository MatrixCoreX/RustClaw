use super::*;
use serde_json::json;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn extracted(bytes: &[u8], file_type: &str) -> DocumentExtractionResult {
    match extract_document(Path::new("fixture"), bytes, file_type).expect("extract document") {
        ExtractOutcome::Extracted(result) => result,
        ExtractOutcome::Skip { error_code, .. } => panic!("unexpected skip: {error_code}"),
    }
}

fn temp_office_path(extension: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "kb-typed-office-{}-{}.{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos(),
        extension
    ))
}

fn minimal_pdf(text: &str) -> Vec<u8> {
    let escaped = text
        .replace('\\', "\\\\")
        .replace('(', "\\(")
        .replace(')', "\\)");
    let stream = format!("BT /F1 12 Tf 72 720 Td ({escaped}) Tj ET");
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>".to_string(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
        format!("<< /Length {} >>\nstream\n{stream}\nendstream", stream.len()),
    ];
    let mut pdf = String::from("%PDF-1.4\n");
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        writeln!(&mut pdf, "{} 0 obj\n{}\nendobj", index + 1, object).expect("pdf object");
    }
    let xref = pdf.len();
    writeln!(&mut pdf, "xref\n0 {}", objects.len() + 1).expect("xref header");
    pdf.push_str("0000000000 65535 f \n");
    for offset in offsets {
        writeln!(&mut pdf, "{offset:010} 00000 n ").expect("xref row");
    }
    write!(
        &mut pdf,
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
        objects.len() + 1
    )
    .expect("pdf trailer");
    pdf.into_bytes()
}

fn create_and_extract_office(
    extension: &str,
    action: &str,
    operations: serde_json::Value,
) -> DocumentExtractionResult {
    let path = temp_office_path(extension);
    office_workspace::execute(&json!({
        "action": action,
        "output_path": path,
        "operations": operations,
    }))
    .expect("create office fixture");
    let bytes = std::fs::read(&path).expect("read fixture");
    let result = match extract_document(&path, &bytes, extension).expect("extract office") {
        ExtractOutcome::Extracted(result) => result,
        ExtractOutcome::Skip { error_code, .. } => panic!("unexpected skip: {error_code}"),
    };
    std::fs::remove_file(path).ok();
    result
}

#[test]
fn extraction_result_is_versioned_and_machine_structured() {
    let result = extracted(br#"{"name":"runtime","enabled":true}"#, "json");

    assert_eq!(result.schema_version, DOCUMENT_EXTRACTION_SCHEMA_VERSION);
    assert_eq!(result.media_type, "application/json");
    assert_eq!(result.parser_version, "json-structured-v2");
    assert_eq!(result.blocks.len(), 1);
    assert_eq!(result.blocks[0].kind, "structured_data");
    assert_eq!(result.blocks[0].location.line_start, Some(1));
    assert!(result.text().contains("runtime"));
    assert!(result.warnings.is_empty());
}

#[test]
fn html_extraction_keeps_visible_structure_and_drops_inert_content() {
    let result = extracted(
        br#"<!doctype html><html><head><style>.secret{display:none}</style><script>window.token='SCRIPT-HIDDEN';</script></head><body><h1>Title &amp; Guide</h1><ul><li>First</li><li>Second &#x2713;</li></ul></body></html>"#,
        "html",
    );
    let text = result.text();

    assert_eq!(result.parser_version, "html-visible-structure-v3");
    assert!(text.contains("Title & Guide"));
    assert!(text.contains("First\nSecond ✓"));
    assert!(!text.contains("SCRIPT-HIDDEN"));
    assert!(!text.contains("display:none"));
    assert!(result.blocks.iter().any(|block| {
        block.kind == "heading" && block.location.section_path == ["Title & Guide"]
    }));
    assert_eq!(
        result
            .blocks
            .iter()
            .filter(|block| block.kind == "list_item")
            .count(),
        2
    );
}

#[test]
fn jsonl_is_validated_per_line_and_canonicalized() {
    let result = extracted(b"{\"id\":1}\r\n{\"id\":2}\n", "jsonl");
    assert_eq!(result.parser_version, "jsonl-records-v2");
    assert_eq!(result.text(), "{\"id\":1}\n{\"id\":2}");
    assert_eq!(result.blocks.len(), 2);
    assert_eq!(result.blocks[1].location.line_start, Some(2));

    let error = extract_document(
        Path::new("fixture.jsonl"),
        b"{\"id\":1}\nnot-json\n",
        "jsonl",
    )
    .expect_err("invalid JSONL must fail extraction");
    assert!(error.to_string().contains("line 2"));
}

#[test]
fn markdown_and_tabular_extractors_preserve_atomic_structure() {
    let markdown = extracted(
        b"# Install\nIntro paragraph.\n\n- Linux\n- macOS\n\n| OS | Command |\n| --- | --- |\n| Linux | run |\n\n```sh\necho ready\n```\n",
        "md",
    );
    assert_eq!(markdown.parser_version, "markdown-structure-v2");
    assert!(markdown
        .blocks
        .iter()
        .any(|block| { block.kind == "heading" && block.location.section_path == ["Install"] }));
    assert_eq!(
        markdown
            .blocks
            .iter()
            .filter(|block| block.kind == "list_item")
            .count(),
        2
    );
    assert_eq!(
        markdown
            .blocks
            .iter()
            .filter(|block| block.kind == "table_row")
            .count(),
        3
    );
    assert!(markdown
        .blocks
        .iter()
        .any(|block| block.kind == "code_block" && block.text.contains("echo ready")));

    let csv = extracted(b"name,value\nlatency,42\n", "csv");
    assert_eq!(csv.parser_version, "csv-rows-v3");
    assert_eq!(csv.blocks.len(), 2);
    assert!(csv.blocks.iter().all(|block| block.kind == "table_row"));
    assert_eq!(csv.blocks[1].location.line_start, Some(2));
}

#[test]
fn binary_and_invalid_utf8_have_stable_skip_codes() {
    let binary = extract_document(Path::new("fixture.bin"), &[0xff, 0x00, 0x10], "bin")
        .expect("classify binary");
    assert!(matches!(
        binary,
        ExtractOutcome::Skip {
            error_code,
            ..
        } if error_code == "binary_content_unsupported"
    ));

    let invalid = extract_document(Path::new("fixture.txt"), &[0xff, 0xfe], "txt")
        .expect("classify invalid text");
    assert!(matches!(
        invalid,
        ExtractOutcome::Skip {
            error_code,
            ..
        } if error_code == "text_encoding_unsupported"
    ));
}

#[test]
fn office_ingestion_reuses_typed_pure_parser_with_source_locations() {
    let docx = create_and_extract_office(
        "docx",
        "word.create",
        json!([
            {"op":"add_heading","text":"Architecture","level":1},
            {"op":"add_paragraph","text":"Runtime evidence"}
        ]),
    );
    assert_eq!(docx.parser_version, "office-pure-parser-v1");
    assert!(docx
        .blocks
        .iter()
        .any(|block| block.location.section_path == ["Architecture"]));

    let xlsx = create_and_extract_office(
        "xlsx",
        "spreadsheet.create",
        json!([
            {"op":"rename_sheet","sheet":"Sheet1","new_name":"Metrics"},
            {"op":"set_cell","sheet":"Metrics","cell":"A1","value":"latency"},
            {"op":"set_cell","sheet":"Metrics","cell":"B1","value":42}
        ]),
    );
    assert!(xlsx.blocks.iter().any(|block| {
        block.location.sheet.as_deref() == Some("Metrics") && block.text.contains("B1\t42")
    }));

    let pptx = create_and_extract_office(
        "pptx",
        "presentation.create",
        json!([
            {"op":"add_slide","title":"Roadmap","body":["Phase one"],"notes":["Evidence note"]}
        ]),
    );
    assert!(pptx.blocks.iter().any(|block| {
        block.location.slide == Some(1)
            && block.text.contains("Roadmap")
            && block.text.contains("Evidence note")
    }));
}

#[test]
fn pdf_ingestion_preserves_page_location_or_reports_missing_optional_parser() {
    let path = temp_office_path("pdf");
    let bytes = minimal_pdf("typed PDF evidence");
    std::fs::write(&path, &bytes).expect("write PDF fixture");

    let outcome = extract_document(&path, &bytes, "pdf").expect("classify PDF");
    std::fs::remove_file(path).ok();
    match outcome {
        ExtractOutcome::Extracted(result) => {
            assert_eq!(result.parser_version, "poppler-text-layer-v1");
            assert!(result.blocks.iter().any(|block| {
                block.location.page == Some(1) && block.text.contains("typed PDF evidence")
            }));
        }
        ExtractOutcome::Skip { error_code, .. } => {
            assert_eq!(error_code, "parser_dependency_unavailable");
        }
    }
}
