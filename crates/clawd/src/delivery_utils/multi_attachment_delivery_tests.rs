use super::*;
use serde_json::Value;

fn fixture() -> (TempDirGuard, AppState, Vec<String>) {
    let root = TempDirGuard::new("multiple_delivery_records");
    let mut state = test_state_with_i18n(&[]);
    state.skill_rt.workspace_root = root.path().to_path_buf();
    state.skill_rt.default_locator_search_dir = root.path().to_path_buf();
    let tokens = ["image_bundle.zip", "background_audio.mp3"]
        .into_iter()
        .map(|name| {
            let path = root.path().join(name);
            fs::write(&path, name).unwrap();
            format!("FILE:{}", path.canonicalize().unwrap().display())
        })
        .collect();
    (root, state, tokens)
}

#[test]
fn token_only_multiline_reply_preserves_every_attachment_in_order() {
    let (_root, state, tokens) = fixture();
    let text = tokens.join("\n");
    assert_eq!(
        super::super::normalize_delivery_message(&state, &text),
        Some(text.clone())
    );
    let (answer, messages) = intercept_response_payload_for_delivery(
        &state,
        "Deliver both existing attachments.",
        false,
        &IntentOutputContract::default(),
        text.clone(),
        vec![text.clone()],
    );
    assert_eq!(answer, text);
    assert_eq!(messages, vec![text]);
    assert_eq!(super::super::extract_delivery_file_tokens(&answer), tokens);
}

#[test]
fn token_lines_preserve_surrounding_prose_and_paragraph_breaks() {
    let (_root, state, tokens) = fixture();
    for text in [
        format!("{}\n\nAudio follows.\n{}\nEnd.", tokens[0], tokens[1]),
        format!("Two attachments:\n{}\n\n{}", tokens[0], tokens[1]),
    ] {
        assert_eq!(
            super::super::normalize_delivery_message(&state, &text),
            Some(text)
        );
    }
}

#[test]
fn missing_token_does_not_remove_other_existing_attachments() {
    let (root, state, tokens) = fixture();
    let missing = format!("FILE:{}", root.path().join("missing.txt").display());
    let text = format!("{}\n{}\n{}", tokens[0], missing, tokens[1]);
    assert_eq!(
        super::super::normalize_delivery_message(&state, &text),
        Some(tokens.join("\n"))
    );
    assert!(super::super::normalize_delivery_message(&state, &missing).is_none());
}

#[test]
fn normalized_reply_materializes_both_files_without_relaxing_artifact_validation() {
    let (root, state, tokens) = fixture();
    let text = super::super::normalize_delivery_message(&state, &tokens.join("\n")).unwrap();
    // The broker accepts typed artifact/output evidence, not display-text parsing.
    let outputs = super::super::extract_delivery_file_tokens(&text)
        .iter()
        .map(|token| {
            serde_json::json!({
                "path": super::super::extract_file_path_from_delivery_token(token).unwrap()
            })
        })
        .collect::<Vec<_>>();
    let result = serde_json::json!({"text":text,"messages":[text],"outputs":outputs});
    let result = crate::task_artifacts::materialize_task_result_artifacts(
        root.path(),
        "multiple-delivery-records",
        &result.to_string(),
    )
    .unwrap();
    let result: Value = serde_json::from_str(&result).unwrap();
    let artifacts = result["artifacts"].as_array().unwrap();
    assert_eq!(artifacts.len(), 2);
    for (artifact, filename) in artifacts
        .iter()
        .zip(["image_bundle.zip", "background_audio.mp3"])
    {
        assert_eq!(artifact["filename"], filename);
        assert_eq!(artifact["schema_version"], 2);
        assert_eq!(artifact["sha256"].as_str().unwrap().len(), 64);
    }
}
