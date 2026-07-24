use super::*;
use crate::package::OfficePackage;
use crate::test_support::temp_path;

#[test]
fn preview_validates_without_writing_output() {
    let output = temp_path("docx");
    let value = execute_mutation(
        "word.preview_create",
        json!({
            "output_path": output,
            "operations": [{"op":"add_paragraph","text":"Preview only"}]
        })
        .as_object()
        .expect("object"),
    )
    .expect("preview");
    assert_eq!(value["preview"], true);
    assert_eq!(value["writes_performed"], false);
    assert!(!output.exists());
}

#[test]
fn create_and_consecutive_hash_bound_edit_are_transactional() {
    let created = temp_path("docx");
    let created_value = execute_mutation(
        "word.create",
        json!({
            "output_path": created,
            "operations": [
                {"op":"add_heading","text":"Version one","level":1},
                {"op":"add_paragraph","text":"Body"}
            ]
        })
        .as_object()
        .expect("object"),
    )
    .expect("create");
    let source_hash = created_value["source"]["sha256"]
        .as_str()
        .expect("source hash")
        .to_string();
    let revised = temp_path("docx");
    let revised_value = execute_mutation(
        "word.edit",
        json!({
            "source_path": created,
            "source_sha256": source_hash,
            "output_path": revised,
            "operations": [
                {"op":"replace_block","block_id":"word_document_paragraph_1","text":"Version two"}
            ]
        })
        .as_object()
        .expect("object"),
    )
    .expect("edit");
    assert_eq!(revised_value["validation"]["valid"], true);
    assert_eq!(
        revised_value["revision_lineage"]["parent_sha256"],
        source_hash
    );
    assert!(created.exists());
    assert!(revised.exists());
    let package = OfficePackage::open(&revised, None).expect("revised package");
    assert_ne!(package.source.sha256, source_hash);
    std::fs::remove_file(created).ok();
    std::fs::remove_file(revised).ok();
}

#[test]
fn stale_hash_rejects_edit_before_output_exists() {
    let source = temp_path("xlsx");
    execute_mutation(
        "spreadsheet.create",
        json!({
            "output_path": source,
            "operations": [
                {"op":"add_sheet","name":"Data"},
                {"op":"set_cell","sheet":"Data","cell":"A1","value":"before"}
            ]
        })
        .as_object()
        .expect("object"),
    )
    .expect("create");
    let output = temp_path("xlsx");
    let error = execute_mutation(
        "spreadsheet.edit",
        json!({
            "source_path": source,
            "source_sha256": "0".repeat(64),
            "output_path": output,
            "operations": [{"op":"set_cell","sheet":"Data","cell":"A1","value":"after"}]
        })
        .as_object()
        .expect("object"),
    )
    .expect_err("stale hash");
    assert_eq!(error.code, "source_conflict");
    assert!(!output.exists());
    std::fs::remove_file(source).ok();
}

#[test]
fn approved_in_place_edit_creates_a_restorable_backup() {
    let source = temp_path("docx");
    let created = execute_mutation(
        "word.create",
        json!({
            "output_path": source,
            "operations": [{"op":"add_paragraph","text":"Before"}]
        })
        .as_object()
        .expect("object"),
    )
    .expect("create");
    let source_hash = created["source"]["sha256"]
        .as_str()
        .expect("hash")
        .to_string();
    let edited = execute_mutation(
        "word.edit",
        json!({
            "source_path": source,
            "source_sha256": source_hash,
            "output_path": source,
            "overwrite": true,
            "in_place": true,
            "operations": [{
                "op":"replace_block",
                "block_id":"word_document_paragraph_1",
                "text":"After"
            }]
        })
        .as_object()
        .expect("object"),
    )
    .expect("edit");
    let backup = edited["revision_lineage"]["backup_path"]
        .as_str()
        .expect("backup");
    let backup_package = OfficePackage::open(Path::new(backup), None).expect("backup package");
    assert_eq!(backup_package.source.sha256, source_hash);
    std::fs::remove_file(source).ok();
    std::fs::remove_file(backup).ok();
}
