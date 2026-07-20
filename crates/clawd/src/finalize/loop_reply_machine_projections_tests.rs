use super::*;
use crate::pipeline_types::OutputListSelector;

#[test]
fn ranked_inventory_projection_preserves_explicit_size_order_and_limit() {
    let body = serde_json::json!({
        "action": "inventory_dir",
        "sort_by": "size_desc",
        "entries": [
            {"name": "small.txt", "kind": "file", "size_bytes": 7},
            {"name": "large.txt", "kind": "file", "size_bytes": 19},
            {"name": "folder", "kind": "dir", "size_bytes": 100}
        ]
    })
    .to_string();
    let route = crate::IntentOutputContract {
        response_shape: crate::OutputResponseShape::Strict,
        selection: crate::OutputSelectionContract {
            list_selector: OutputListSelector {
                limit: Some(1),
                ..Default::default()
            },
            ..Default::default()
        },
        ..Default::default()
    };

    assert_eq!(
        inventory_ranked_size_list_answer(&body, &route).as_deref(),
        Some("large.txt 19")
    );
}

#[test]
fn compare_paths_metadata_projection_returns_only_machine_fields() {
    let body = serde_json::json!({
        "action": "compare_paths",
        "comparison": {"same_path": false},
        "left": {"exists": true, "kind": "file"},
        "right": {"exists": false}
    })
    .to_string();

    assert_eq!(
        compare_paths_metadata_answer(&body).as_deref(),
        Some("same_path=false\nleft_exists=true\nleft_kind=file\nright_exists=false\nright_kind=-")
    );
}

#[test]
fn compare_paths_metadata_projection_rejects_unrelated_actions() {
    assert!(
        compare_paths_metadata_answer(r#"{"action":"count_inventory","counts":{"total":2}}"#)
            .is_none()
    );
}
