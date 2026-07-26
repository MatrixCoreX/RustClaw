use super::*;

#[test]
fn image_magic_maps_to_vision_capability_and_workspace_reference() {
    let workspace = Path::new("/workspace");
    let resolved = workspace.join("assets/photo.bin");
    let handoff = content_handoff(
        workspace,
        "assets/photo.bin",
        &resolved,
        Some(b"\x89PNG\r\n\x1a\nrest"),
    );

    assert_eq!(handoff["detected_kind"], "image");
    assert_eq!(handoff["mime_type"], "image/png");
    assert_eq!(handoff["capability_ref"], "image_vision.describe");
    assert_eq!(handoff["argument_name"], "images");
    assert_eq!(handoff["reference"]["kind"], "workspace_path");
    assert_eq!(handoff["reference"]["path"], "assets/photo.bin");
}

#[test]
fn pdf_and_docx_map_to_document_parser() {
    let workspace = Path::new("/workspace");
    let pdf = content_handoff(
        workspace,
        "manual.bin",
        &workspace.join("manual.bin"),
        Some(b"%PDF-1.7"),
    );
    let docx = content_handoff(
        workspace,
        "manual.docx",
        &workspace.join("manual.docx"),
        None,
    );

    assert_eq!(pdf["detected_kind"], "pdf");
    assert_eq!(pdf["capability_ref"], "document.parse");
    assert_eq!(docx["detected_kind"], "document");
    assert_eq!(docx["capability_ref"], "document.parse");
}

#[test]
fn generic_binary_uses_metadata_capability_without_fabricating_a_reader() {
    let workspace = Path::new("/workspace");
    let handoff = content_handoff(
        workspace,
        "/opt/data/archive.bin",
        Path::new("/opt/data/archive.bin"),
        Some(&[0, 1, 2]),
    );

    assert_eq!(handoff["detected_kind"], "binary");
    assert_eq!(handoff["capability_ref"], "filesystem.stat_paths");
    assert_eq!(handoff["reference"]["kind"], "explicit_path");
    assert_eq!(handoff["reference"]["path"], "/opt/data/archive.bin");
}
