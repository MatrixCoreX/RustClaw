fn read_task_result_json_for_debug(
    state: &AppState,
    task_id: &str,
) -> anyhow::Result<Option<(String, Value)>> {
    let db = state
        .core
        .db
        .get()
        .map_err(|err| anyhow::anyhow!("db pool: {err}"))?;
    let row = db
        .query_row(
            "SELECT status, result_json FROM tasks WHERE task_id = ?1 LIMIT 1",
            [task_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
        )
        .optional()?;
    let Some((db_status, raw_result_json)) = row else {
        return Ok(None);
    };
    let Some(raw_result_json) = raw_result_json else {
        return Ok(None);
    };
    let Ok(result_json) = serde_json::from_str::<Value>(&raw_result_json) else {
        return Ok(None);
    };
    Ok(Some((db_status, result_json)))
}

fn build_model_catalog_trace_for_debug(state: &AppState, entries: &[TaskDebugEntry]) -> Value {
    let catalog = match claw_core::model_catalog::build_model_catalog_from_workspace(
        &state.skill_rt.workspace_root,
    ) {
        Ok(catalog) => catalog,
        Err(error) => {
            return json!({
                "trace_kind": "model_catalog_decision",
                "status": "catalog_unavailable",
                "error_code": "model_catalog_unavailable",
                "error_detail": error.to_string(),
            });
        }
    };
    let observed_providers = entries
        .iter()
        .filter_map(task_debug_entry_provider_token)
        .collect::<BTreeSet<_>>();
    let observed_models = entries
        .iter()
        .filter_map(|entry| {
            entry
                .model
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
        })
        .collect::<BTreeSet<_>>();
    let catalog_entries = catalog
        .entries
        .iter()
        .filter(|entry| {
            entry.active_text_provider
                || observed_providers.contains(&entry.provider)
                || observed_models.contains(&entry.model)
        })
        .map(|entry| {
            json!({
                "schema_version": entry.schema_version,
                "provider": entry.provider,
                "model": entry.model,
                "models": entry.models,
                "api_style": entry.api_style,
                "base_url_kind": entry.base_url_kind,
                "credential_state": entry.credential_state,
                "context_window_tokens": entry.context_window_tokens,
                "timeout_seconds": entry.timeout_seconds,
                "input_modalities": entry.input_modalities,
                "output_modalities": entry.output_modalities,
                "supports_text": entry.supports_text,
                "supports_image_input": entry.supports_image_input,
                "supports_video_input": entry.supports_video_input,
                "supports_audio_input": entry.supports_audio_input,
                "supports_image_understanding": entry.supports_image_understanding,
                "supports_audio_transcription": entry.supports_audio_transcription,
                "supports_image_generation": entry.supports_image_generation,
                "supports_audio_generation": entry.supports_audio_generation,
                "supports_video_generation": entry.supports_video_generation,
                "supports_music_generation": entry.supports_music_generation,
                "async_required": entry.async_required,
                "dry_run_supported": entry.dry_run_supported,
                "active_text_provider": entry.active_text_provider,
            })
        })
        .collect::<Vec<_>>();
    let vendor_patch_names = catalog
        .entries
        .iter()
        .flat_map(|entry| entry.config_source.iter())
        .filter_map(|source| source.strip_prefix("prompts/layers/vendor_patches/"))
        .filter_map(|suffix| suffix.split('/').next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect::<BTreeSet<_>>();
    json!({
        "trace_kind": "model_catalog_decision",
        "status": "ok",
        "selected_provider": catalog.selected_provider,
        "selected_model": catalog.selected_model,
        "readiness": build_model_readiness_trace_for_debug(&catalog),
        "observed_provider_count": observed_providers.len(),
        "observed_providers": observed_providers.into_iter().collect::<Vec<_>>(),
        "observed_models": observed_models.into_iter().collect::<Vec<_>>(),
        "entry_count": catalog_entries.len(),
        "entries": catalog_entries,
        "vendor_patch_names": vendor_patch_names.into_iter().collect::<Vec<_>>(),
        "catalog_guard_status": read_model_catalog_guard_status(&state.skill_rt.workspace_root),
    })
}

fn build_model_readiness_trace_for_debug(
    catalog: &claw_core::model_catalog::ModelCatalog,
) -> Value {
    let matched_entry_count = catalog
        .entries
        .iter()
        .filter(|entry| {
            entry.provider == catalog.selected_provider && entry.model == catalog.selected_model
        })
        .count();
    let selected_entry = catalog.entries.iter().find(|entry| {
        entry.provider == catalog.selected_provider && entry.model == catalog.selected_model
    });
    let selected_entry_status = if selected_entry.is_some() {
        "found"
    } else {
        "missing"
    };
    let credential_state = selected_entry
        .map(|entry| entry.credential_state.as_str())
        .unwrap_or("null");
    let text_generation = selected_entry
        .map(|entry| entry.supports_text)
        .unwrap_or(false);
    let ready = selected_entry.is_some()
        && text_generation
        && !matches!(credential_state, "missing" | "null" | "");
    json!({
        "schema_version": catalog.schema_version,
        "selected_provider": catalog.selected_provider,
        "selected_model": catalog.selected_model,
        "selected_entry_status": selected_entry_status,
        "entry_count": catalog.entries.len(),
        "matched_entry_count": matched_entry_count,
        "credential_state": credential_state,
        "ready": ready,
        "text_generation": text_generation,
        "image_input": selected_entry.map(|entry| entry.supports_image_input).unwrap_or(false),
        "image_understanding": selected_entry.map(|entry| entry.supports_image_understanding).unwrap_or(false),
        "image_generation": selected_entry.map(|entry| entry.supports_image_generation).unwrap_or(false),
        "audio_input": selected_entry.map(|entry| entry.supports_audio_input).unwrap_or(false),
        "audio_transcription": selected_entry.map(|entry| entry.supports_audio_transcription).unwrap_or(false),
        "audio_generation": selected_entry.map(|entry| entry.supports_audio_generation).unwrap_or(false),
        "video_input": selected_entry.map(|entry| entry.supports_video_input).unwrap_or(false),
        "video_generation": selected_entry.map(|entry| entry.supports_video_generation).unwrap_or(false),
        "music_generation": selected_entry.map(|entry| entry.supports_music_generation).unwrap_or(false),
        "async_required": selected_entry.map(|entry| entry.async_required).unwrap_or(false),
        "dry_run": selected_entry.map(|entry| entry.dry_run_supported).unwrap_or(false),
    })
}

fn task_debug_entry_provider_token(entry: &TaskDebugEntry) -> Option<String> {
    entry
        .vendor
        .as_deref()
        .or(entry.provider.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.strip_prefix("vendor-").unwrap_or(value))
        .map(|value| value.to_ascii_lowercase())
}

fn extract_resume_trace_for_debug(db_status: &str, result_json: &Value) -> Option<Value> {
    let lifecycle =
        crate::task_lifecycle::task_query_lifecycle_projection(db_status, Some(result_json), None);
    if !resume_trace_has_signal(&lifecycle) && extract_task_checkpoint_for_debug(result_json).is_none()
    {
        return None;
    }
    let mut trace = serde_json::Map::new();
    trace.insert("trace_kind".to_string(), json!("task_resume_decision"));
    copy_lifecycle_field(&mut trace, &lifecycle, "state");
    copy_lifecycle_field(&mut trace, &lifecycle, "execution_state");
    copy_lifecycle_field(&mut trace, &lifecycle, "state_source");
    copy_lifecycle_field(&mut trace, &lifecycle, "reason_code");
    copy_lifecycle_field(&mut trace, &lifecycle, "waiting_reason_code");
    copy_lifecycle_field(&mut trace, &lifecycle, "checkpoint_id");
    copy_lifecycle_field(&mut trace, &lifecycle, "resume_due");
    copy_lifecycle_field(&mut trace, &lifecycle, "resume_wait_seconds");
    copy_lifecycle_field(&mut trace, &lifecycle, "resume_entrypoint");
    copy_lifecycle_field(&mut trace, &lifecycle, "next_action_kind");
    copy_lifecycle_field(&mut trace, &lifecycle, "recommended_user_action_kind");
    copy_lifecycle_field(&mut trace, &lifecycle, "completed_side_effect_count");
    copy_lifecycle_field(&mut trace, &lifecycle, "completed_side_effect_refs");
    copy_lifecycle_field(
        &mut trace,
        &lifecycle,
        "completed_side_effect_refs_truncated",
    );
    copy_lifecycle_field(&mut trace, &lifecycle, "requires_idempotency_guard");
    copy_lifecycle_field(&mut trace, &lifecycle, "provider_blocker_active");
    copy_lifecycle_field(&mut trace, &lifecycle, "provider_blocker_status_code");
    copy_lifecycle_field(
        &mut trace,
        &lifecycle,
        "provider_blocker_retry_after_seconds",
    );
    copy_lifecycle_field(
        &mut trace,
        &lifecycle,
        "provider_blocker_next_recovery_kind",
    );
    copy_lifecycle_field(&mut trace, &lifecycle, "open_issue_count");
    copy_lifecycle_field(&mut trace, &lifecycle, "open_issue_codes");
    if let Some(checkpoint) = extract_task_checkpoint_for_debug(result_json) {
        copy_checkpoint_fallback_fields(&mut trace, &checkpoint);
        trace.insert("task_checkpoint".to_string(), checkpoint);
    }
    trace.insert("lifecycle".to_string(), lifecycle);
    Some(Value::Object(trace))
}

fn copy_checkpoint_fallback_fields(trace: &mut serde_json::Map<String, Value>, checkpoint: &Value) {
    if !trace.contains_key("checkpoint_id") {
        copy_lifecycle_field(trace, checkpoint, "checkpoint_id");
    }
    if !trace.contains_key("resume_entrypoint") {
        copy_lifecycle_field(trace, checkpoint, "resume_entrypoint");
    }
    if !trace.contains_key("completed_side_effect_count") {
        if let Some(refs) = checkpoint
            .get("completed_side_effect_refs")
            .and_then(Value::as_array)
        {
            trace.insert("completed_side_effect_count".to_string(), json!(refs.len()));
            trace.insert("requires_idempotency_guard".to_string(), json!(!refs.is_empty()));
        }
    }
    let Some(signal) = checkpoint.get("repair_signal").filter(|value| value.is_object()) else {
        return;
    };
    let external_provider_blocked = signal
        .get("external_provider_blocked")
        .or_else(|| signal.get("external_blocked"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if external_provider_blocked && !trace.contains_key("provider_blocker_active") {
        trace.insert("provider_blocker_active".to_string(), json!(true));
    }
    if !trace.contains_key("provider_blocker_status_code") {
        if let Some(status_code) = signal
            .get("status_code")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            trace.insert("provider_blocker_status_code".to_string(), json!(status_code));
        }
    }
    if !trace.contains_key("provider_blocker_retry_after_seconds") {
        if let Some(retry_after) = signal.get("retry_after_seconds").and_then(Value::as_u64) {
            trace.insert(
                "provider_blocker_retry_after_seconds".to_string(),
                json!(retry_after),
            );
        }
    }
    if !trace.contains_key("provider_blocker_next_recovery_kind") {
        if let Some(next_recovery_kind) = signal
            .get("next_recovery_kind")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            trace.insert(
                "provider_blocker_next_recovery_kind".to_string(),
                json!(next_recovery_kind),
            );
        }
    }
}

fn resume_trace_has_signal(lifecycle: &Value) -> bool {
    let state = lifecycle
        .get("state")
        .and_then(Value::as_str)
        .unwrap_or_default();
    matches!(state, "waiting" | "background" | "needs_user")
        || lifecycle.get("checkpoint_id").is_some()
        || lifecycle
            .get("completed_side_effect_count")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            > 0
        || lifecycle
            .get("provider_blocker_active")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        || lifecycle
            .get("open_issue_count")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            > 0
}

fn copy_lifecycle_field(
    trace: &mut serde_json::Map<String, Value>,
    lifecycle: &Value,
    key: &str,
) {
    if let Some(value) = lifecycle.get(key).filter(|value| !value.is_null()) {
        trace.insert(key.to_string(), value.clone());
    }
}

fn extract_task_checkpoint_for_debug(result_json: &Value) -> Option<Value> {
    [
        "/task_journal/trace/task_checkpoint",
        "/task_journal/summary/task_checkpoint",
        "/task_checkpoint",
    ]
    .iter()
    .find_map(|pointer| {
        result_json
            .pointer(pointer)
            .filter(|value| value.is_object())
            .cloned()
    })
}

#[cfg(test)]
mod task_debug_trace_tests {
    use super::{
        build_model_catalog_trace_for_debug, extract_resume_trace_for_debug, TaskDebugEntry,
    };
    use serde_json::json;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_debug_workspace_root() -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("rustclaw-debug-trace-{unique}"));
        std::fs::create_dir_all(&path).expect("create temp workspace");
        path
    }

    fn write_minimax_debug_catalog(root: &PathBuf, selected_model: &str) {
        std::fs::create_dir_all(root.join("configs")).expect("configs dir");
        std::fs::write(
            root.join("configs/config.toml"),
            format!(
                r#"
[llm]
selected_vendor = "minimax"
selected_model = "{selected_model}"

[llm.minimax]
api_format = "openai_compat"
base_url = "https://api.minimaxi.com/v1"
api_key = "catalog-secret"
model = "MiniMax-M3"
models = ["MiniMax-M3"]
input_modalities = ["text", "image", "video"]
output_modalities = ["text"]
context_window_tokens = 1000000
"#
            ),
        )
        .expect("write config");
        std::fs::write(
            root.join("configs/image.toml"),
            r#"
[image_vision]
minimax_models = ["MiniMax-M3"]
"#,
        )
        .expect("write image config");
    }

    #[test]
    fn task_debug_model_catalog_trace_projects_secret_free_capabilities() {
        let root = temp_debug_workspace_root();
        write_minimax_debug_catalog(&root, "MiniMax-M3");
        let mut state = crate::AppState::test_default_with_fixture_provider();
        state.skill_rt.workspace_root = root;
        let entries = vec![TaskDebugEntry {
            ts: Some(10),
            task_id: Some("task-1".to_string()),
            call_id: Some("task-1:planner".to_string()),
            vendor: Some("minimax".to_string()),
            provider: Some("vendor-minimax".to_string()),
            provider_type: Some("openai_compat".to_string()),
            model: Some("MiniMax-M3".to_string()),
            model_kind: None,
            status: Some("ok".to_string()),
            mode: None,
            prompt_source: Some("layered:prompts/lightweight_execution_prompt.md".to_string()),
            prompt_hash: None,
            prompt_file: None,
            prompt: None,
            request_payload: Some(json!({"messages": []})),
            response: None,
            raw_response: Some("{}".to_string()),
            clean_response: None,
            sanitized: None,
            error: None,
            usage: None,
        }];

        let trace = build_model_catalog_trace_for_debug(&state, &entries);

        assert_eq!(trace["trace_kind"], "model_catalog_decision");
        assert_eq!(trace["status"], "ok");
        assert_eq!(trace["selected_provider"], "minimax");
        assert_eq!(trace["selected_model"], "MiniMax-M3");
        assert_eq!(trace["observed_providers"][0], "minimax");
        assert_eq!(trace["entries"][0]["schema_version"], 1);
        assert_eq!(trace["entries"][0]["models"], json!(["MiniMax-M3"]));
        assert!(trace["entries"][0]
            .as_object()
            .is_some_and(|entry| entry.contains_key("timeout_seconds")));
        assert_eq!(trace["entries"][0]["timeout_seconds"], json!(null));
        assert_eq!(
            trace["entries"][0]["input_modalities"],
            json!(["text", "image", "video"])
        );
        assert_eq!(trace["entries"][0]["output_modalities"], json!(["text"]));
        assert_eq!(trace["entries"][0]["supports_image_input"], true);
        assert_eq!(trace["entries"][0]["supports_image_understanding"], true);
        assert_eq!(trace["entries"][0]["supports_audio_transcription"], false);
        assert_eq!(trace["entries"][0]["active_text_provider"], true);
        assert_eq!(trace["entries"][0]["credential_state"], "configured_inline");
        assert_eq!(trace["readiness"]["schema_version"], 1);
        assert_eq!(trace["readiness"]["selected_provider"], "minimax");
        assert_eq!(trace["readiness"]["selected_model"], "MiniMax-M3");
        assert_eq!(trace["readiness"]["selected_entry_status"], "found");
        assert_eq!(trace["readiness"]["entry_count"], 1);
        assert_eq!(trace["readiness"]["matched_entry_count"], 1);
        assert_eq!(trace["readiness"]["credential_state"], "configured_inline");
        assert_eq!(trace["readiness"]["ready"], true);
        assert_eq!(trace["readiness"]["text_generation"], true);
        assert_eq!(trace["readiness"]["image_input"], true);
        assert_eq!(trace["readiness"]["image_understanding"], true);
        assert_eq!(trace["readiness"]["video_input"], true);
        assert_eq!(trace["readiness"]["async_required"], false);
        assert_eq!(trace["readiness"]["dry_run"], false);
        assert_eq!(trace["vendor_patch_names"][0], "minimax");
        assert!(!trace.to_string().contains("catalog-secret"));
    }

    #[test]
    fn task_debug_model_catalog_trace_marks_missing_selected_model_not_ready() {
        let root = temp_debug_workspace_root();
        write_minimax_debug_catalog(&root, "missing-model");
        let mut state = crate::AppState::test_default_with_fixture_provider();
        state.skill_rt.workspace_root = root;
        let entries = vec![TaskDebugEntry {
            ts: Some(10),
            task_id: Some("task-1".to_string()),
            call_id: Some("task-1:planner".to_string()),
            vendor: Some("minimax".to_string()),
            provider: Some("vendor-minimax".to_string()),
            provider_type: Some("openai_compat".to_string()),
            model: Some("MiniMax-M3".to_string()),
            model_kind: None,
            status: Some("ok".to_string()),
            mode: None,
            prompt_source: Some("layered:prompts/lightweight_execution_prompt.md".to_string()),
            prompt_hash: None,
            prompt_file: None,
            prompt: None,
            request_payload: Some(json!({"messages": []})),
            response: None,
            raw_response: Some("{}".to_string()),
            clean_response: None,
            sanitized: None,
            error: None,
            usage: None,
        }];

        let trace = build_model_catalog_trace_for_debug(&state, &entries);

        assert_eq!(trace["selected_model"], "missing-model");
        assert_eq!(trace["readiness"]["selected_model"], "missing-model");
        assert_eq!(trace["readiness"]["selected_entry_status"], "missing");
        assert_eq!(trace["readiness"]["matched_entry_count"], 0);
        assert_eq!(trace["readiness"]["credential_state"], "null");
        assert_eq!(trace["readiness"]["ready"], false);
        assert_eq!(trace["readiness"]["text_generation"], false);
        assert_eq!(trace["entries"][0]["model"], "MiniMax-M3");
        assert!(!trace.to_string().contains("catalog-secret"));
    }

    #[test]
    fn task_debug_resume_trace_projects_checkpoint_machine_fields() {
        let result_json = json!({
            "task_lifecycle": {
                "state": "waiting",
                "resume_reason": "provider_gap_retry_window",
                "next_check_after": 1781800300,
                "checkpoint_id": "ckpt-1"
            },
            "task_checkpoint": {
                "schema_version": 1,
                "checkpoint_id": "ckpt-1",
                "resume_entrypoint": "next_planner_round",
                "budget": {
                    "round": 2,
                    "step": 3,
                    "llm_calls": 4,
                    "tool_calls": 5,
                    "elapsed_ms": 6000,
                    "llm_elapsed_ms": 3000,
                    "tool_elapsed_ms": 2000
                },
                "completed_side_effect_refs": ["write_file:tmp/a.txt"],
                "evidence_refs": ["step:evidence:1"],
                "artifact_refs": ["changed_file:tmp/a.txt"],
                "last_successful_round": 2,
                "repair_signal": {
                    "status_code": "provider_rate_limited",
                    "external_provider_blocked": true,
                    "provider": "minimax",
                    "retry_after_seconds": 30,
                    "next_recovery_kind": "background_wait"
                }
            }
        });

        let trace = extract_resume_trace_for_debug("running", &result_json).expect("resume trace");

        assert_eq!(trace["trace_kind"], "task_resume_decision");
        assert_eq!(trace["state"], "waiting");
        assert_eq!(trace["checkpoint_id"], "ckpt-1");
        assert_eq!(trace["resume_entrypoint"], "next_planner_round");
        assert_eq!(trace["completed_side_effect_count"], 1);
        assert_eq!(trace["requires_idempotency_guard"], true);
        assert_eq!(trace["provider_blocker_active"], true);
        assert_eq!(
            trace["provider_blocker_status_code"],
            "provider_rate_limited"
        );
        assert_eq!(trace["task_checkpoint"]["checkpoint_id"], "ckpt-1");
    }
}
