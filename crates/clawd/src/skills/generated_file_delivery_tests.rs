use super::*;

#[test]
fn delivery_validation_rejects_cross_task_refs_append_and_implicit_replacement() {
    for args in [
        json!({"deliver_to_user":"true"}),
        json!({"replaces_artifact_refs":["artifact:task/task-a/old"]}),
        json!({"deliver_to_user":true,"replaces_artifact_refs":["artifact:task/task-b/old"]}),
    ] {
        assert!(validate(args.as_object().unwrap(), "task-a", false).is_err());
    }
    assert!(validate(
        json!({"deliver_to_user":true}).as_object().unwrap(),
        "task-a",
        true
    )
    .is_err());
    assert!(validate(
        json!({"deliver_to_user":true,"replaces_artifact_refs":["artifact:task/task-a/old"]})
            .as_object()
            .unwrap(),
        "task-a",
        false
    )
    .is_ok());
}

#[test]
fn ordinary_writes_do_not_publish_and_delivery_snapshots_are_immutable() {
    let root = std::env::temp_dir().join(format!("file-delivery-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let mut extra = None;
    publish(
        &root,
        "task-a",
        &json!({"path":"result.txt","content":"not delivered"}),
        &mut extra,
    )
    .unwrap();
    assert!(extra.is_none());
    let mut outputs = Vec::new();
    for content in ["old version", "corrected version"] {
        let mut extra = Some(json!({"action":"write_text"}));
        publish(
            &root,
            "task-a",
            &json!({"path":"验收结果.txt","content":content,"deliver_to_user":true}),
            &mut extra,
        )
        .unwrap();
        outputs.push(extra.unwrap()["artifacts"][0].clone());
    }
    assert_ne!(outputs[0]["path"], outputs[1]["path"]);
    for (artifact, expected) in outputs.iter().zip(["old version", "corrected version"]) {
        assert_eq!(artifact["filename"], "验收结果.txt");
        assert_eq!(artifact["owner_task_id"], "task-a");
        assert_eq!(
            std::fs::read_to_string(root.join(artifact["path"].as_str().unwrap())).unwrap(),
            expected
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}
