use super::*;
use crate::model::OfficeFormat;
use crate::operations::normalize_operations;
use crate::package::OfficePackage;
use crate::package_write::publish_package;
use crate::pptx::read_presentation;
use crate::pptx_write::create_pptx;
use crate::test_support::temp_path;

fn two_slide_package() -> (std::path::PathBuf, OfficePackage) {
    let operations = normalize_operations(
        Some(&json!([
            {"op":"add_slide","title":"One"},
            {"op":"add_slide","title":"Two"}
        ])),
        OfficeFormat::Pptx,
        false,
    )
    .expect("create operations");
    let members = create_pptx(&operations).expect("create").members;
    let path = temp_path("pptx");
    publish_package(&members, &path, OfficeFormat::Pptx, false, None, None).expect("publish");
    let package = OfficePackage::open(&path, Some(OfficeFormat::Pptx)).expect("package");
    (path, package)
}

fn publish_edit(package: &OfficePackage, operations: Value) -> (std::path::PathBuf, OfficePackage) {
    let operations =
        normalize_operations(Some(&operations), OfficeFormat::Pptx, true).expect("edit operations");
    let result = edit_pptx(package, &operations).expect("edit");
    let path = temp_path("pptx");
    publish_package(
        &result.members,
        &path,
        OfficeFormat::Pptx,
        false,
        None,
        None,
    )
    .expect("publish edit");
    let package = OfficePackage::open(&path, Some(OfficeFormat::Pptx)).expect("edited package");
    (path, package)
}

#[test]
fn stable_slide_ids_survive_reorder_and_followup_edit() {
    let (source, package) = two_slide_package();
    let (reordered_path, reordered) = publish_edit(
        &package,
        json!([{"op":"move_slide","slide_id":"slide_2","position":1}]),
    );
    let evidence = read_presentation(&reordered).expect("read reordered");
    assert_eq!(
        evidence
            .slides
            .iter()
            .map(|slide| slide.id.as_str())
            .collect::<Vec<_>>(),
        vec!["slide_2", "slide_1"]
    );

    let (revised_path, revised) = publish_edit(
        &reordered,
        json!([{
            "op":"replace_slide_text",
            "slide_id":"slide_2",
            "match":"Two",
            "text":"Revised"
        }]),
    );
    let evidence = read_presentation(&revised).expect("read revised");
    assert_eq!(evidence.slides[0].id, "slide_2");
    assert_eq!(evidence.slides[0].title.as_deref(), Some("Revised"));
    assert_eq!(evidence.slides[1].title.as_deref(), Some("One"));

    for path in [source, reordered_path, revised_path] {
        std::fs::remove_file(path).ok();
    }
}

#[test]
fn slide_lifecycle_preserves_part_identity_and_hidden_state() {
    let (source, package) = two_slide_package();
    let (output, edited) = publish_edit(
        &package,
        json!([
            {"op":"add_slide","title":"Three","position":2,"hidden":true},
            {"op":"duplicate_slide","slide_id":"slide_1","position":4},
            {"op":"set_slide_layout","slide_id":"slide_3","layout":"slideLayout1"},
            {"op":"move_slide","slide_id":"slide_2","position":1},
            {"op":"move_slide","slide_id":"slide_3","position":2},
            {"op":"delete_slide","slide_id":"slide_4"}
        ]),
    );
    let evidence = read_presentation(&edited).expect("read lifecycle");
    assert_eq!(
        evidence
            .slides
            .iter()
            .map(|slide| slide.id.as_str())
            .collect::<Vec<_>>(),
        vec!["slide_2", "slide_3", "slide_1"]
    );
    assert!(evidence.slides[1].hidden);
    assert!(evidence.slides[1]
        .layout
        .as_deref()
        .is_some_and(|value| value.ends_with("slideLayout1.xml")));
    assert!(!edited.members.contains_key("ppt/slides/slide4.xml"));

    for path in [source, output] {
        std::fs::remove_file(path).ok();
    }
}

#[test]
fn relationship_backed_objects_round_trip_without_dropping_unknown_parts() {
    let (source, mut package) = two_slide_package();
    package.members.insert(
        "customXml/item1.xml".into(),
        b"<custom>keep</custom>".to_vec(),
    );
    let image = temp_path("png");
    std::fs::write(&image, b"replacement-image-bytes").expect("image");
    let (output, edited) = publish_edit(
        &package,
        json!([
            {"op":"add_text","slide_id":"slide_1","text":"Added text"},
            {"op":"add_table","slide_id":"slide_1","rows":[["Name","Value"],["A","1"]]},
            {"op":"add_chart","slide_id":"slide_1","title":"Trend","chart_type":"line","categories":["A","B"],"values":[1,2]},
            {"op":"add_shape","slide_id":"slide_1","shape":"roundRect","text":"Decision"},
            {"op":"add_link","slide_id":"slide_1","text":"Reference","url":"https://example.test/reference"},
            {"op":"add_image","slide_id":"slide_1","path":image.to_string_lossy(),"alt":"Evidence"},
            {"op":"replace_image","media_id":"media_1","path":image.to_string_lossy()},
            {"op":"add_notes","slide_id":"slide_1","notes":["Speaker note"]},
            {"op":"set_transition","slide_id":"slide_1","transition":"wipe"}
        ]),
    );
    let evidence = read_presentation(&edited).expect("read objects");
    let slide = &evidence.slides[0];
    assert!(slide.text.iter().any(|value| value == "Added text"));
    assert_eq!(slide.tables[0].rows[1], vec!["A", "1"]);
    assert_eq!(slide.charts.len(), 1);
    assert_eq!(slide.images.len(), 1);
    assert!(slide.notes.iter().any(|value| value == "Speaker note"));
    assert!(edited.members.contains_key("customXml/item1.xml"));
    let relationships = std::str::from_utf8(
        edited
            .members
            .get("ppt/slides/_rels/slide1.xml.rels")
            .expect("slide relationships"),
    )
    .expect("relationships text");
    assert!(relationships.contains("TargetMode=\"External\""));
    assert!(relationships.contains("https://example.test/reference"));
    let slide_xml = std::str::from_utf8(
        edited
            .members
            .get("ppt/slides/slide1.xml")
            .expect("slide part"),
    )
    .expect("slide text");
    assert!(slide_xml.contains("<p:transition><p:wipe/></p:transition>"));

    for path in [source, output, image] {
        std::fs::remove_file(path).ok();
    }
}

#[test]
fn deleting_the_last_slide_returns_a_structured_error() {
    let operations = normalize_operations(
        Some(&json!([{"op":"add_slide","title":"Only"}])),
        OfficeFormat::Pptx,
        false,
    )
    .expect("create operations");
    let members = create_pptx(&operations).expect("create").members;
    let source = temp_path("pptx");
    publish_package(&members, &source, OfficeFormat::Pptx, false, None, None).expect("publish");
    let package = OfficePackage::open(&source, Some(OfficeFormat::Pptx)).expect("package");
    let operations = normalize_operations(
        Some(&json!([{"op":"delete_slide","slide_id":"slide_1"}])),
        OfficeFormat::Pptx,
        true,
    )
    .expect("edit operations");
    let error = match edit_pptx(&package, &operations) {
        Ok(_) => panic!("must reject last slide deletion"),
        Err(error) => error,
    };
    assert_eq!(error.code, "last_slide");
    std::fs::remove_file(source).ok();
}
