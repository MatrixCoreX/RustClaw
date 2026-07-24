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
    assert_eq!(revised_value["revision_lineage"]["source_kind"], "revision");
    assert_eq!(revised_value["continuation"]["status"], "ready");
    assert_eq!(
        revised_value["continuation"]["preview_capability"],
        "word.preview_edit"
    );
    assert_eq!(
        revised_value["continuation"]["source_sha256"],
        package_hash(&revised)
    );
    let revised_hash = revised_value["continuation"]["source_sha256"]
        .as_str()
        .expect("revised hash")
        .to_string();
    let final_revision = temp_path("docx");
    let final_value = execute_mutation(
        "word.edit",
        json!({
            "source_path": revised,
            "source_sha256": revised_hash,
            "output_path": final_revision,
            "operations": [
                {"op":"replace_block","block_id":"word_document_paragraph_2","text":"Body updated later"}
            ]
        })
        .as_object()
        .expect("object"),
    )
    .expect("follow-up edit");
    assert_eq!(
        final_value["revision_lineage"]["parent_sha256"],
        revised_hash
    );
    assert_eq!(
        final_value["continuation"]["source_sha256"],
        package_hash(&final_revision)
    );
    assert!(created.exists());
    assert!(revised.exists());
    assert!(final_revision.exists());
    let package = OfficePackage::open(&revised, None).expect("revised package");
    assert_ne!(package.source.sha256, source_hash);
    std::fs::remove_file(created).ok();
    std::fs::remove_file(revised).ok();
    std::fs::remove_file(final_revision).ok();
}

#[test]
fn template_creation_records_read_only_lineage_and_verified_continuation() {
    let template = temp_path("docx");
    let template_value = execute_mutation(
        "word.create",
        json!({
            "output_path": template,
            "operations": [{"op":"add_paragraph","text":"Template content"}]
        })
        .as_object()
        .expect("object"),
    )
    .expect("create template");
    let template_hash = template_value["source"]["sha256"]
        .as_str()
        .expect("template hash")
        .to_string();
    let output = temp_path("docx");
    let value = execute_mutation(
        "word.create",
        json!({
            "template_path": template,
            "output_path": output,
            "operations": [{"op":"add_paragraph","text":"New revision"}]
        })
        .as_object()
        .expect("object"),
    )
    .expect("create from template");
    assert_eq!(value["revision_lineage"]["source_kind"], "template");
    assert_eq!(value["revision_lineage"]["template_read_only"], true);
    assert_eq!(value["revision_lineage"]["template_sha256"], template_hash);
    assert!(value["revision_lineage"]["parent_sha256"].is_null());
    assert_eq!(value["continuation"]["edit_capability"], "word.edit");
    assert_eq!(
        value["continuation"]["source_path"],
        output.display().to_string()
    );
    assert_eq!(
        value["continuation"]["source_sha256"],
        package_hash(&output)
    );
    assert_eq!(
        package_hash(&template),
        template_hash,
        "template must remain unchanged"
    );
    std::fs::remove_file(template).ok();
    std::fs::remove_file(output).ok();
}

#[test]
fn presentation_template_revision_preserves_source_and_stable_slide_ids() {
    let template = temp_path("pptx");
    let template_value = execute_mutation(
        "presentation.create",
        json!({
            "output_path": template,
            "operations": [
                {"op":"add_slide","title":"Opening","body":"First"},
                {"op":"add_slide","title":"Details","body":"Second"}
            ]
        })
        .as_object()
        .expect("object"),
    )
    .expect("create template");
    let template_hash = template_value["source"]["sha256"]
        .as_str()
        .expect("template hash")
        .to_string();
    let slides = template_value["presentation"]["slides"]
        .as_array()
        .expect("slides");
    let first_id = slides[0]["id"].as_str().expect("first slide ID");
    let second_id = slides[1]["id"].as_str().expect("second slide ID");
    let output = temp_path("pptx");
    let created_from_template = execute_mutation(
        "presentation.create",
        json!({
            "template_path": template,
            "output_path": output,
            "operations": [
                {"op":"add_slide","title":"Appendix","body":"Created from template"}
            ]
        })
        .as_object()
        .expect("object"),
    )
    .expect("create from template");
    assert_eq!(
        created_from_template["revision_lineage"]["source_kind"],
        "template"
    );
    assert_eq!(
        created_from_template["revision_lineage"]["template_sha256"],
        template_hash
    );
    assert_eq!(package_hash(&template), template_hash);
    let output_hash = created_from_template["continuation"]["source_sha256"]
        .as_str()
        .expect("output hash")
        .to_string();
    let final_revision = temp_path("pptx");
    let revised = execute_mutation(
        "presentation.edit",
        json!({
            "source_path": output,
            "source_sha256": output_hash,
            "output_path": final_revision,
            "operations": [
                {"op":"move_slide","slide_id":second_id,"position":1},
                {"op":"replace_slide_text","slide_id":first_id,"match":"Opening","text":"Revised opening"},
                {"op":"add_notes","slide_id":first_id,"text":"Follow-up notes"}
            ]
        })
        .as_object()
        .expect("object"),
    )
    .expect("edit template-derived revision");
    assert_eq!(revised["revision_lineage"]["source_kind"], "revision");
    assert_eq!(revised["revision_lineage"]["parent_sha256"], output_hash);
    let revised_slides = revised["presentation"]["slides"]
        .as_array()
        .expect("revised slides");
    assert_eq!(revised_slides[0]["id"], second_id);
    assert_eq!(revised_slides[1]["id"], first_id);
    assert!(revised_slides[1]["text"]
        .as_array()
        .expect("text")
        .iter()
        .any(|value| value == "Revised opening"));
    assert!(revised_slides[1]["notes"]
        .as_array()
        .expect("notes")
        .iter()
        .any(|value| value == "Follow-up notes"));
    std::fs::remove_file(template).ok();
    std::fs::remove_file(output).ok();
    std::fs::remove_file(final_revision).ok();
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

fn package_hash(path: &Path) -> String {
    OfficePackage::open(path, None)
        .expect("package")
        .source
        .sha256
}
