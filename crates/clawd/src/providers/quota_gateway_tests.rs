use super::*;

fn task(state: &crate::AppState, name: &str, input: &str) -> crate::ClaimedTask {
    state.seed_ask_task_row(name, 7, 11, &json!({"text":input}).to_string());
    crate::repo::claim_next_task(state).unwrap().unwrap()
}

async fn call(
    state: &crate::AppState,
    task: &crate::ClaimedTask,
    native: bool,
) -> Result<(), String> {
    if native {
        crate::llm_gateway::run_native_model_turn_with_fallback(
            state,
            task,
            "fixture",
            "test:quota_terminal",
            &request(false),
        )
        .await
        .map(|_| ())
    } else {
        crate::llm_gateway::run_with_fallback_with_prompt_source(
            state,
            task,
            "fixture",
            "test:quota_terminal",
        )
        .await
        .map(|_| ())
    }
}

#[tokio::test]
async fn gateway_quota_terminates_with_english_notice_without_composer_or_model_retry() {
    for (native, input, completed_before_quota) in [
        (false, "请继续任务", false),
        (true, "続けてください", false),
        (false, "Continúa la tarea", true),
        (true, "Continue the task", true),
    ] {
        let server = server(
            StatusCode::TOO_MANY_REQUESTS,
            HeaderMap::new(),
            json!({"error":{"type":"rate_limit_error","message":"opaque (2056)"}}).to_string(),
        )
        .await;
        let mut state =
            crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
        state.core.llm_providers = vec![runtime("vendor-minimax", "openai_compat", &server.url)];
        let task = task(&state, "quota-terminal-task", input);
        let error = call(&state, &task, native).await.unwrap_err();
        assert_eq!(error, crate::llm_gateway::PROVIDER_QUOTA_TERMINATED);
        assert!(state.task_provider_quota_exhausted(&task.task_id).is_some());
        assert!(state.task_provider_blocker(&task.task_id).is_none());
        assert_eq!(state.task_llm_call_count(&task.task_id), 1);
        assert!(call(&state, &task, !native).await.is_err());
        assert_eq!(server.calls.load(Ordering::SeqCst), 1);
        let outcome = if completed_before_quota {
            let mut journal =
                crate::task_journal::TaskJournal::for_task(&task.task_id, "ask", input);
            journal
                .step_results
                .push(crate::task_journal::TaskJournalStepTrace::ok(
                    "completed-write",
                    "write_file",
                    r#"{"path":"fixture.txt","content_bytes":18}"#,
                ));
            Ok(crate::AskReply::non_llm("candidate".into()).with_task_journal(journal))
        } else {
            Err(error)
        };
        crate::finalize::finalize_ask_result(
            &state,
            &task,
            &json!({"text":input}),
            input,
            "",
            None,
            input,
            None,
            &[],
            None,
            outcome,
        )
        .await
        .unwrap();
        let (status, raw): (String, String) = state
            .core
            .db
            .get()
            .unwrap()
            .query_row(
                "SELECT status,result_json FROM tasks WHERE task_id=?1",
                [&task.task_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        let result: Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(status, "failed");
        assert_eq!(result["text"], "API quota exhausted. Task terminated.");
        assert_eq!(result["task_lifecycle"]["state"], "failed");
        assert_eq!(result["task_lifecycle"]["retryable"], false);
        assert!(result["task_lifecycle"].get("next_check_after").is_none());
        assert!(result["task_journal"]["summary"]["task_checkpoint"].is_null());
        if completed_before_quota {
            assert_eq!(
                result["task_journal"]["trace"]["step_results"][0]["step_id"],
                "completed-write"
            );
        }
        assert_eq!(server.calls.load(Ordering::SeqCst), 1);
        assert!(state.task_provider_quota_exhausted(&task.task_id).is_none());
        for channel in ["wechat", "telegram", "feishu", "lark", "whatsapp"] {
            let mut channel_task = task.clone();
            channel_task.channel = channel.into();
            channel_task.external_user_id = Some("fixture-user".into());
            channel_task.external_chat_id = Some("12345".into());
            let envelope = crate::delivery_service::build_daemon_delivery_envelope(
                &state,
                &channel_task,
                &json!({"text":input,"context_token":"fixture-context"}),
                result["text"].as_str().unwrap(),
                claw_core::channel_delivery::ChannelDeliverySource::ImmediateDaemon,
                claw_core::channel_delivery::ChannelTaskDeliveryContent::Full,
                None,
            )
            .unwrap();
            assert_eq!(envelope.text_segments.len(), 1);
            assert_eq!(
                envelope.text_segments[0].text,
                "API quota exhausted. Task terminated."
            );
            assert!(envelope.artifacts.is_empty());
        }
    }
}

#[tokio::test]
async fn quota_in_one_provider_does_not_terminate_a_successful_configured_fallback() {
    for native in [false, true] {
        let quota = server(
            StatusCode::TOO_MANY_REQUESTS,
            HeaderMap::new(),
            json!({"error":{"type":"rate_limit_error","message":"opaque (2056)"}}).to_string(),
        )
        .await;
        let success = server(StatusCode::OK, HeaderMap::new(), json!({
            "choices":[{"message":{"role":"assistant","content":"fixture-success"},"finish_reason":"stop"}]
        }).to_string()).await;
        let mut state =
            crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
        let mut fallback = runtime("fixture-fallback", "openai_compat", &success.url);
        Arc::get_mut(&mut fallback).unwrap().config.priority = 2;
        state.core.llm_providers = vec![
            runtime("vendor-minimax", "openai_compat", &quota.url),
            fallback,
        ];
        let task = task(&state, "quota-fallback-task", "fixture");
        assert!(call(&state, &task, native).await.is_ok());
        assert_eq!(quota.calls.load(Ordering::SeqCst), 1);
        assert_eq!(success.calls.load(Ordering::SeqCst), 1);
        assert!(state.task_provider_quota_exhausted(&task.task_id).is_none());
    }
}

#[tokio::test]
async fn temporary_rate_limit_remains_waiting_without_a_quota_notice() {
    for native in [false, true] {
        let mut headers = HeaderMap::new();
        headers.insert("retry-after", "60".parse().unwrap());
        let server = server(StatusCode::TOO_MANY_REQUESTS, headers,
            json!({"error":{"type":"rate_limit_error","code":"rate_limit_exceeded","message":"fixture"}}).to_string()).await;
        let mut state =
            crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
        state.core.llm_providers = vec![runtime("vendor-minimax", "openai_compat", &server.url)];
        let task = task(&state, "rate-limit-task", "fixture");
        assert!(call(&state, &task, native).await.is_err());
        assert!(state.task_provider_quota_exhausted(&task.task_id).is_none());
        let blocker = state.task_provider_blocker(&task.task_id).unwrap();
        assert_eq!(blocker.status_code, "rate_limited");
        assert!(blocker.retry_after_seconds > 0);
    }
}

#[tokio::test]
async fn mixed_quota_and_temporary_provider_failure_do_not_select_the_fixed_notice() {
    for native in [false, true] {
        for quota_first in [false, true] {
            let quota = server(
                StatusCode::TOO_MANY_REQUESTS,
                HeaderMap::new(),
                json!({"error":{"type":"rate_limit_error","message":"opaque (2056)"}}).to_string(),
            )
            .await;
            let mut headers = HeaderMap::new();
            headers.insert("retry-after", "60".parse().unwrap());
            let temporary = server(
                StatusCode::SERVICE_UNAVAILABLE,
                headers,
                json!({"error":{"type":"server_error","message":"fixture"}}).to_string(),
            )
            .await;
            let mut state =
                crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
            let mut quota_provider = runtime("vendor-minimax", "openai_compat", &quota.url);
            let mut temporary_provider =
                runtime("fixture-temporary", "openai_compat", &temporary.url);
            Arc::get_mut(&mut quota_provider).unwrap().config.priority =
                if quota_first { 1 } else { 2 };
            Arc::get_mut(&mut temporary_provider)
                .unwrap()
                .config
                .priority = if quota_first { 2 } else { 1 };
            state.core.llm_providers = vec![quota_provider, temporary_provider];
            let task = task(&state, "mixed-quota-task", "fixture");
            let error = call(&state, &task, native).await.unwrap_err();
            assert_ne!(error, crate::llm_gateway::PROVIDER_QUOTA_TERMINATED);
            assert!(state.task_provider_quota_exhausted(&task.task_id).is_none());
            assert_eq!(
                state
                    .task_provider_blocker(&task.task_id)
                    .unwrap()
                    .status_code,
                "provider_retryable_response"
            );
            assert_eq!(quota.calls.load(Ordering::SeqCst), 1);
            assert_eq!(temporary.calls.load(Ordering::SeqCst), 1);
        }
    }
}
