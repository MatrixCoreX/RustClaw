use super::*;

#[test]
fn render_status_is_structured_on_every_platform() {
    let value = execute("office.render_status", &Map::new()).expect("status");
    assert_eq!(value["schema_version"], 1);
    assert!(value["available"].is_boolean());
    assert_eq!(value["structural_office_support_independent"], true);
}

#[test]
fn unsupported_render_format_is_rejected_before_claiming_success() {
    if detect_renderer().is_none() {
        return;
    }
    let error = render(
        &detect_renderer().expect("renderer"),
        json!({
            "path": "source.docx",
            "output_path": "output.png",
            "format": "png"
        })
        .as_object()
        .expect("object"),
    )
    .expect_err("format");
    assert!(
        matches!(error.code, "source_unavailable" | "unsupported_operation"),
        "source safety may run before output format validation"
    );
}
