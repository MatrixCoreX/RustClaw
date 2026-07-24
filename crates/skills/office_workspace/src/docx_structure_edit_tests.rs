use super::*;
use crate::docx::read_docx;
use crate::docx_write::{create_docx, edit_docx};
use crate::model::OfficeFormat;
use crate::operations::normalize_operations;
use crate::package::OfficePackage;
use crate::package_write::publish_package;
use crate::test_support::temp_path;

fn create_members(operations: Value) -> std::collections::BTreeMap<String, Vec<u8>> {
    let operations = normalize_operations(Some(&operations), OfficeFormat::Docx, false)
        .expect("create operations");
    create_docx(&operations).expect("create").members
}

fn apply_edit(
    members: &std::collections::BTreeMap<String, Vec<u8>>,
    operations: Value,
) -> std::collections::BTreeMap<String, Vec<u8>> {
    let operations =
        normalize_operations(Some(&operations), OfficeFormat::Docx, true).expect("edit operations");
    edit_docx(members, &operations).expect("edit").members
}

#[test]
fn inserts_and_moves_revision_bound_paragraphs() {
    let members = create_members(json!([
        {"op":"add_paragraph","text":"Alpha"},
        {"op":"add_paragraph","text":"Beta"}
    ]));
    let members = apply_edit(
        &members,
        json!([
            {"op":"insert_block_after","block_id":"word_document_paragraph_2","text":"Gamma","style":"Heading2"}
        ]),
    );
    let members = apply_edit(
        &members,
        json!([
            {"op":"move_block","block_id":"word_document_paragraph_3","target_block_id":"word_document_paragraph_1","position":"before"}
        ]),
    );
    let document = std::str::from_utf8(&members["word/document.xml"]).expect("document");
    assert!(document.find("Gamma").expect("Gamma") < document.find("Alpha").expect("Alpha"));
    assert!(document.contains("<w:pStyle w:val=\"Heading2\"/>"));
}

#[test]
fn inserts_replaces_moves_and_deletes_runs() {
    let members = create_members(json!([
        {"op":"add_paragraph","text":"one"}
    ]));
    let members = apply_edit(
        &members,
        json!([
            {"op":"insert_run","block_id":"word_document_paragraph_1","index":1,"text":" two","style":"Emphasis"}
        ]),
    );
    let members = apply_edit(
        &members,
        json!([
            {"op":"replace_run","block_id":"word_document_paragraph_1","run":2,"expected_text":" two","text":" second"}
        ]),
    );
    let members = apply_edit(
        &members,
        json!([
            {"op":"move_run","block_id":"word_document_paragraph_1","run":2,"target_index":0}
        ]),
    );
    let members = apply_edit(
        &members,
        json!([
            {"op":"delete_run","block_id":"word_document_paragraph_1","run":2}
        ]),
    );
    let document = std::str::from_utf8(&members["word/document.xml"]).expect("document");
    assert!(document.contains(" second"));
    assert!(!document.contains(">one<"));
}

#[test]
fn resizes_table_rows_and_columns_without_rebuilding_other_parts() {
    let mut members = create_members(json!([
        {"op":"add_table","rows":[["a","b"],["c","d"]]}
    ]));
    members.insert("custom/preserve.bin".into(), b"keep".to_vec());
    let members = apply_edit(
        &members,
        json!([
            {"op":"table_add_row","table_id":"word_document_table_1","index":1,"values":["x","y"]},
            {"op":"table_add_column","table_id":"word_document_table_1","column":2,"values":["h","i","j"]}
        ]),
    );
    let members = apply_edit(
        &members,
        json!([
            {"op":"table_delete_row","table_id":"word_document_table_1","row":2},
            {"op":"table_delete_column","table_id":"word_document_table_1","column":0}
        ]),
    );
    assert_eq!(
        members.get("custom/preserve.bin").map(Vec::as_slice),
        Some(b"keep".as_slice())
    );
    let output = temp_path("docx");
    publish_package(&members, &output, OfficeFormat::Docx, false, None, None).expect("publish");
    let package = OfficePackage::open(&output, Some(OfficeFormat::Docx)).expect("package");
    let evidence = read_docx(&package).expect("read");
    assert_eq!(
        evidence.tables[0].rows,
        vec![vec!["b", "h"], vec!["y", "i"]]
    );
    std::fs::remove_file(output).ok();
}
