use super::*;
use crate::test_support::{temp_path, write_package};

fn required_docx_parts() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "[Content_Types].xml",
            r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"/>"#,
        ),
        (
            "_rels/.rels",
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"/>"#,
        ),
        (
            "word/document.xml",
            r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body/></w:document>"#,
        ),
    ]
}

#[test]
fn rejects_traversal_members_before_parsing_xml() {
    let path = temp_path("docx");
    let mut parts = required_docx_parts();
    parts.push(("../outside.xml", "unsafe"));
    write_package(&path, &parts);
    let error = OfficePackage::open(&path, None).expect_err("traversal");
    assert_eq!(error.code, "path_traversal");
    std::fs::remove_file(path).ok();
}

#[test]
fn rejects_macro_members_and_never_executes_them() {
    let path = temp_path("docx");
    let mut parts = required_docx_parts();
    parts.push(("word/vbaProject.bin", "not-executed"));
    write_package(&path, &parts);
    let error = OfficePackage::open(&path, None).expect_err("macro");
    assert_eq!(error.code, "macro_enabled_package");
    std::fs::remove_file(path).ok();
}

#[test]
fn reports_external_relationships_as_untrusted() {
    let path = temp_path("docx");
    let mut parts = required_docx_parts();
    parts.push((
        "word/_rels/document.xml.rels",
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId9" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="https://example.invalid/" TargetMode="External"/></Relationships>"#,
    ));
    write_package(&path, &parts);
    let package = OfficePackage::open(&path, None).expect("package");
    assert_eq!(package.evidence.external_relationships.len(), 1);
    assert!(package.evidence.external_relationships[0].untrusted);
    assert!(package
        .warnings
        .iter()
        .any(|warning| warning.code == "external_relationships_present"));
    std::fs::remove_file(path).ok();
}

#[test]
fn media_stays_as_an_untrusted_source_package_artifact_reference() {
    let path = temp_path("docx");
    let mut parts = required_docx_parts();
    parts.push(("word/media/image1.png", "binary-placeholder"));
    write_package(&path, &parts);
    let package = OfficePackage::open(&path, None).expect("package");
    assert_eq!(package.media.len(), 1);
    assert!(package.media[0].untrusted);
    assert!(!package.media[0].content_inline);
    assert_eq!(package.media[0].storage_kind, "source_package_member");
    assert_eq!(package.evidence.artifact_members.len(), 1);
    assert!(package.evidence.artifact_members[0].untrusted);
    assert!(!package.evidence.artifact_members[0].content_inline);
    std::fs::remove_file(path).ok();
}

#[test]
fn large_xml_stays_in_the_source_package_and_is_not_inlined() {
    let path = temp_path("docx");
    let large_xml = format!(
        "<items>{}</items>",
        (0u32..40_000)
            .map(|index| format!("<item id=\"{index}\">{:08x}</item>", index.rotate_left(7)))
            .collect::<String>()
    );
    assert!(large_xml.len() > DEFAULT_LARGE_MEMBER_REF_BYTES as usize);
    let mut parts = required_docx_parts();
    parts.push(("word/large.xml", Box::leak(large_xml.into_boxed_str())));
    write_package(&path, &parts);
    let package = OfficePackage::open(&path, None).expect("package");
    let artifact = package
        .evidence
        .artifact_members
        .iter()
        .find(|artifact| artifact.package_member == "word/large.xml")
        .expect("large XML artifact reference");
    assert!(artifact.untrusted);
    assert!(!artifact.content_inline);
    assert_eq!(artifact.storage_kind, "source_package_member");
    assert!(artifact.size_bytes > DEFAULT_LARGE_MEMBER_REF_BYTES);
    std::fs::remove_file(path).ok();
}

#[test]
fn rejects_expansion_ratio_and_total_size_limits() {
    let path = temp_path("docx");
    let large = "x".repeat(100_000);
    let mut parts = required_docx_parts();
    parts.push(("word/large.xml", Box::leak(large.into_boxed_str())));
    write_package(&path, &parts);
    let error = OfficePackage::open_with_limits(
        &path,
        None,
        &PackageLimits {
            max_entries: 10,
            max_member_bytes: 200_000,
            max_total_bytes: 200_000,
            max_expansion_ratio: 2,
        },
    )
    .expect_err("expansion");
    assert_eq!(error.code, "package_expansion_rejected");
    std::fs::remove_file(path).ok();
}
