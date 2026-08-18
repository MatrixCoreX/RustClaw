use super::{
    bounded_content_excerpt, handle_parse_doc, normalize_action, parse_doc_extra, parse_docx,
    resolve_input_path_against, Metadata, ParsePayload, TableMode, EXTRA_CONTENT_EXCERPT_CHARS,
};
use serde_json::json;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

struct TempOfficeFile(PathBuf);

impl TempOfficeFile {
    fn docx(document_xml: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "agent_doc_parse_{}_{}.docx",
            std::process::id(),
            nonce
        ));
        let file = File::create(&path).expect("create docx fixture");
        let mut archive = ZipWriter::new(file);
        archive
            .start_file("[Content_Types].xml", SimpleFileOptions::default())
            .expect("start content types");
        archive
            .write_all(
                br#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#,
            )
            .expect("write content types");
        archive
            .start_file("_rels/.rels", SimpleFileOptions::default())
            .expect("start root relationships");
        archive
            .write_all(
                br#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#,
            )
            .expect("write root relationships");
        archive
            .start_file("word/document.xml", SimpleFileOptions::default())
            .expect("start document.xml");
        archive
            .write_all(document_xml.as_bytes())
            .expect("write document.xml");
        archive.finish().expect("finish docx fixture");
        Self(path)
    }
}

impl Drop for TempOfficeFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[test]
fn normalize_action_accepts_parse_alias() {
    assert_eq!(normalize_action("parse_doc"), Some("parse_doc"));
    assert_eq!(normalize_action("parse"), Some("parse_doc"));
    assert_eq!(normalize_action("unknown"), None);
}

#[test]
fn relative_input_path_resolves_against_workspace_root() {
    let root = PathBuf::from("/workspace");

    let resolved = resolve_input_path_against(PathBuf::from("docs/report.md"), Some(&root));

    assert_eq!(resolved, root.join("docs/report.md"));
}

#[test]
fn absolute_input_path_is_not_rebased() {
    let absolute = std::env::temp_dir().join("report.md");

    let resolved =
        resolve_input_path_against(absolute.clone(), Some(std::path::Path::new("/workspace")));

    assert_eq!(resolved, absolute);
}

#[test]
fn parse_doc_extra_exposes_path_and_content_excerpt() {
    let req = json!({
        "args": {
            "path": "README.md"
        }
    });
    let payload = ParsePayload {
        text: "Agent Runtime is a local agent runtime.".to_string(),
        tables: vec![],
        sections: vec![],
        metadata: Some(Metadata {
            title: "Agent Runtime".to_string(),
            pages: 1,
            doc_type: "md".to_string(),
            path: "/home/guagua/agent-runtime/README.md".to_string(),
            encoding: "utf-8-or-lossy".to_string(),
            truncated: false,
            truncation_notice: None,
            page_range_applied: None,
            original_chars: 33,
            returned_chars: 33,
            start_char: 0,
            end_char: 33,
            snapshot_sha256: "fixture".to_string(),
            next_continuation: None,
        }),
        status: "ok".to_string(),
        error_code: None,
        error: None,
    };

    let extra = parse_doc_extra(&req, &payload);

    assert_eq!(
        extra.get("path").and_then(|value| value.as_str()),
        Some("/home/guagua/agent-runtime/README.md")
    );
    assert_eq!(
        extra.get("requested_path").and_then(|value| value.as_str()),
        Some("README.md")
    );
    assert_eq!(
        extra
            .get("content_excerpt")
            .and_then(|value| value.as_str()),
        Some("Agent Runtime is a local agent runtime.")
    );
    assert_eq!(
        extra
            .get("content_excerpt_truncated")
            .and_then(|value| value.as_bool()),
        Some(false)
    );
}

#[test]
fn parse_doc_extra_falls_back_to_requested_path_without_metadata() {
    let req = json!({
        "args": {
            "path": "AGENTS.md"
        }
    });
    let payload = ParsePayload {
        text: "Agent development rules".to_string(),
        tables: vec![],
        sections: vec![],
        metadata: None,
        status: "ok".to_string(),
        error_code: None,
        error: None,
    };

    let extra = parse_doc_extra(&req, &payload);

    assert_eq!(
        extra.get("path").and_then(|value| value.as_str()),
        Some("AGENTS.md")
    );
}

#[test]
fn bounded_content_excerpt_limits_long_text_without_suffix() {
    let text = "x".repeat(EXTRA_CONTENT_EXCERPT_CHARS + 5);

    let excerpt = bounded_content_excerpt(&text, EXTRA_CONTENT_EXCERPT_CHARS);

    assert_eq!(excerpt.len(), EXTRA_CONTENT_EXCERPT_CHARS);
    assert!(excerpt.chars().all(|ch| ch == 'x'));
}

#[test]
fn document_text_pages_continue_without_losing_the_canonical_range() {
    let path = std::env::temp_dir().join(format!(
        "agent-runtime-doc-parse-page-{}-{}.txt",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos()
    ));
    let source = (0..260)
        .map(|index| char::from(b'a' + (index % 26) as u8))
        .collect::<String>();
    std::fs::write(&path, &source).expect("write fixture");

    let first = handle_parse_doc(&json!({
        "args": {"path": path, "max_chars": 100, "mode": "text_only"}
    }));
    assert_eq!(first.text.chars().count(), 100);
    let metadata = first.metadata.as_ref().expect("metadata");
    assert_eq!(metadata.original_chars, 260);
    assert_eq!(metadata.start_char, 0);
    let continuation = metadata.next_continuation.clone().expect("continuation");

    let second = handle_parse_doc(&json!({
        "args": {
            "path": path,
            "max_chars": 100,
            "mode": "text_only",
            "continuation": continuation,
        }
    }));
    assert_eq!(second.metadata.as_ref().expect("metadata").start_char, 100);
    assert_eq!(
        second.text,
        source.chars().skip(100).take(100).collect::<String>()
    );

    std::fs::write(&path, format!("{source}changed")).expect("change snapshot");
    let stale = handle_parse_doc(&json!({
        "args": {
            "path": path,
            "max_chars": 100,
            "mode": "text_only",
            "continuation": continuation,
        }
    }));
    assert_eq!(stale.error_code.as_deref(), Some("stale_snapshot"));
    let _ = std::fs::remove_file(path);
}

#[test]
fn docx_parser_reads_heading_body_and_table_from_real_package() {
    let fixture = TempOfficeFile::docx(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p>
      <w:pPr><w:pStyle w:val="Heading1"/></w:pPr>
      <w:r><w:t>Quarterly Report</w:t></w:r>
    </w:p>
    <w:p><w:r><w:t>Revenue grew 12%.</w:t></w:r></w:p>
    <w:tbl>
      <w:tr>
        <w:tc><w:p><w:r><w:t>Region</w:t></w:r></w:p></w:tc>
        <w:tc><w:p><w:r><w:t>Revenue</w:t></w:r></w:p></w:tc>
      </w:tr>
      <w:tr>
        <w:tc><w:p><w:r><w:t>APAC</w:t></w:r></w:p></w:tc>
        <w:tc><w:p><w:r><w:t>120</w:t></w:r></w:p></w:tc>
      </w:tr>
    </w:tbl>
  </w:body>
</w:document>"#,
    );

    let parsed = parse_docx(&fixture.0, TableMode::Strict).expect("parse real docx package");

    assert_eq!(parsed.title, "Quarterly Report");
    assert!(parsed.text.contains("Revenue grew 12%."));
    assert_eq!(parsed.sections[0].title, "Quarterly Report");
    assert_eq!(parsed.tables.len(), 1);
    assert_eq!(parsed.tables[0].header, ["Region", "Revenue"]);
    assert_eq!(parsed.tables[0].rows, [["APAC", "120"]]);
    assert_eq!(parsed.encoding, "utf-8");
}
