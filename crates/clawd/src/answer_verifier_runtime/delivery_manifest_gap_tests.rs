use super::*;

#[test]
fn path_only_delivery_requires_an_actual_published_manifest() {
    let root = std::env::temp_dir().join(format!("manifest-gap-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let task_id = "manifest-task";
    let mut output = crate::IntentOutputContract::default();
    output.delivery_required = true;
    output.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    let contract = AnswerContract::new("deliver a file", output);
    let mut journal = crate::task_journal::TaskJournal::for_task(task_id, "ask", "deliver a file");
    let file = root.join("result.txt");
    std::fs::write(&file, "actual content").unwrap();
    let candidate = format!("FILE:{}", file.display());
    assert!(
        missing_delivery_manifest_gap(&root, task_id, &contract, &journal, &candidate).is_some()
    );
    journal
        .capability_results
        .push(claw_core::capability_result::CapabilityResultEnvelope::ok(
            "test.produce",
            None,
            serde_json::json!({"extra":{"artifacts":[{"path":file}]}}),
        ));
    assert!(
        missing_delivery_manifest_gap(&root, task_id, &contract, &journal, &candidate).is_none()
    );
    std::fs::remove_dir_all(root).unwrap();
}
