use super::*;
use crate::model::OfficeFormat;
use crate::operations::normalize_operations;
use crate::package::OfficePackage;
use crate::package_write::publish_package;
use crate::pptx::read_presentation;
use crate::test_support::temp_path;

#[test]
fn creates_and_reopens_structured_multilingual_deck() {
    let operations = normalize_operations(
        Some(&json!([
            {"op":"add_slide","title":"产品路线图","body":["第一阶段","Second phase"],"notes":["讲解重点"]},
            {"op":"add_table","slide_id":"slide_1","rows":[["里程碑","状态"],["M1","完成"]]},
            {"op":"add_chart","slide_id":"slide_1","title":"进度","categories":["M1","M2"],"values":[70,90]},
            {"op":"add_shape","slide_id":"slide_1","shape":"roundRect","text":"Verified"},
            {"op":"set_transition","slide_id":"slide_1","transition":"fade"},
            {"op":"add_slide","title":"Résumé","body":"Next steps"}
        ])),
        OfficeFormat::Pptx,
        false,
    )
    .expect("operations");
    let result = create_pptx(&operations).expect("create");
    let path = temp_path("pptx");
    publish_package(
        &result.members,
        &path,
        OfficeFormat::Pptx,
        false,
        None,
        None,
    )
    .expect("publish");
    let package = OfficePackage::open(&path, Some(OfficeFormat::Pptx)).expect("package");
    let deck = read_presentation(&package).expect("read");
    assert_eq!(deck.slides.len(), 2);
    assert_eq!(deck.slides[0].title.as_deref(), Some("产品路线图"));
    assert!(deck.slides[0].text.iter().any(|text| text == "Verified"));
    assert!(!deck.slides[0].tables.is_empty());
    assert!(!deck.slides[0].charts.is_empty());
    assert!(deck.slides[0].notes.iter().any(|text| text == "讲解重点"));
    std::fs::remove_file(path).ok();
}

#[test]
fn edits_text_notes_and_order_while_preserving_theme() {
    let create = normalize_operations(
        Some(&json!([
            {"op":"add_slide","title":"One"},
            {"op":"add_slide","title":"Two"}
        ])),
        OfficeFormat::Pptx,
        false,
    )
    .expect("create operations");
    let members = create_pptx(&create).expect("create").members;
    let source = temp_path("pptx");
    publish_package(&members, &source, OfficeFormat::Pptx, false, None, None).expect("publish");
    let package = OfficePackage::open(&source, Some(OfficeFormat::Pptx)).expect("package");
    let edit = normalize_operations(
        Some(&json!([
            {"op":"replace_slide_text","slide_id":"slide_2","match":"Two","text":"Revised"},
            {"op":"add_notes","slide_id":"slide_2","text":"Reviewer note"},
            {"op":"move_slide","slide_id":"slide_2","position":1}
        ])),
        OfficeFormat::Pptx,
        true,
    )
    .expect("edit operations");
    let result = edit_pptx(&package, &edit).expect("edit");
    assert!(result.members.contains_key("ppt/theme/theme1.xml"));
    assert!(
        std::str::from_utf8(&result.members["ppt/slides/slide2.xml"])
            .expect("slide")
            .contains("Revised")
    );
    assert!(result
        .members
        .contains_key("ppt/notesSlides/notesSlide2.xml"));
    let presentation =
        std::str::from_utf8(&result.members["ppt/presentation.xml"]).expect("presentation");
    assert!(
        presentation.find("rIdSlide2").expect("slide2")
            < presentation.find("rIdSlide1").expect("slide1")
    );
    std::fs::remove_file(source).ok();
}
