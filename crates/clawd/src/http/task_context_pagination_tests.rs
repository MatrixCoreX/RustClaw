use super::*;

fn history(count: usize) -> TaskDeliveryRecord {
    let mut source = record();
    source.result_json.as_mut().unwrap()["task_journal"]["trace"]["capability_results"] = json!((0
        ..count)
        .map(|index| json!({"capability":"fixture.read", "status":"ok",
            "provenance":{"continuation_args":{"index":index}},
            "data":{"index":index, "password":"hidden"}}))
        .collect::<Vec<_>>());
    source
}

fn index_request() -> RecallRequest {
    RecallRequest {
        field_paths: vec!["/capabilities".into()],
        ..Default::default()
    }
}

#[test]
fn history_pages_cover_boundary_sizes_without_gaps_or_duplicates() {
    for count in [0, 1, 63, 64, 65, 70, 130] {
        let source = history(count);
        let mut request = index_request();
        let mut found = Vec::new();
        loop {
            let value = select_context(&source, &request).unwrap();
            assert_eq!(value["pagination"]["total"], count);
            assert_eq!(value["pagination"]["index_complete"], true);
            let items = value["fields"]["/capabilities"].as_array().unwrap();
            assert!(items.len() <= 64);
            found.extend(
                items
                    .iter()
                    .map(|item| item["index"].as_u64().unwrap() as usize),
            );
            let cursor = value["pagination"]["next_cursor"].as_str();
            assert_eq!(value["pagination"]["has_more"], cursor.is_some());
            match cursor {
                Some(token) => request.cursor = Some(token.into()),
                None => break,
            }
        }
        assert_eq!(found, (0..count).collect::<Vec<_>>());
    }
}

#[test]
fn selected_fields_beyond_first_page_are_redacted_and_snapshot_fenced() {
    let source = history(70);
    let snapshot = select_context(&source, &index_request()).unwrap()["source_digest"]
        .as_str()
        .unwrap()
        .to_string();
    let mut request = RecallRequest {
        field_paths: vec!["/capabilities/69/data".into()],
        source_digest: Some(snapshot),
        ..Default::default()
    };
    let value = select_context(&source, &request).unwrap();
    assert_eq!(
        value["fields"]["/capabilities/69/data"],
        json!({"index":69})
    );
    request.field_paths = vec!["/capabilities/69/data/password".into()];
    assert_eq!(
        select_context(&source, &request),
        Err("context_field_forbidden")
    );
    request.field_paths = vec!["/capabilities/69/data".into()];
    request.source_digest = Some("changed".into());
    assert_eq!(
        select_context(&source, &request),
        Err("context_snapshot_changed")
    );
}

#[test]
fn cursors_bind_snapshot_selection_page_size_actor_and_expiry() {
    let source = history(70);
    let mut request = index_request();
    request.cursor = Some(
        select_context(&source, &request).unwrap()["pagination"]["next_cursor"]
            .as_str()
            .unwrap()
            .into(),
    );
    assert!(select_context(&source, &request).is_ok());
    assert_eq!(
        super::super::select_context(&source, &request, "other-current-task-or-scope", 100),
        Err("context_cursor_invalid")
    );
    assert_eq!(
        super::super::select_context(&source, &request, "fixture-binding", 1000),
        Err("context_cursor_expired")
    );
    let mut changed = history(70);
    changed.task.task_id = source.task.task_id.clone();
    changed.result_json.as_mut().unwrap()["new_field"] = json!(true);
    assert_eq!(
        select_context(&changed, &request),
        Err("context_snapshot_changed")
    );
    let mut other = index_request();
    other.cursor = request.cursor.clone();
    other.page_size = Some(32);
    assert_eq!(
        select_context(&source, &other),
        Err("context_cursor_invalid")
    );
    other.page_size = None;
    other.field_paths.push("/request".into());
    assert_eq!(
        select_context(&source, &other),
        Err("context_cursor_invalid")
    );
    let token = request.cursor.as_mut().unwrap();
    token.replace_range(0..1, if token.starts_with('A') { "B" } else { "A" });
    assert_eq!(
        select_context(&source, &request),
        Err("context_cursor_invalid")
    );
}

#[test]
fn invalid_pages_and_missing_or_compacted_sources_never_look_complete() {
    let mut source = history(70);
    for size in [0, 65, usize::MAX] {
        let mut request = index_request();
        request.page_size = Some(size);
        assert_eq!(
            select_context(&source, &request),
            Err("context_page_invalid")
        );
    }
    source.result_json.as_mut().unwrap()["task_journal"]["trace"]["trace_storage"] =
        json!({"truncated":true});
    assert_eq!(
        select_context(&source, &index_request()),
        Err("context_source_compacted")
    );
    let request = RecallRequest {
        field_paths: vec!["/capabilities/0/data".into()],
        ..Default::default()
    };
    assert_eq!(
        select_context(&source, &request),
        Err("context_source_compacted")
    );
    let request = RecallRequest {
        field_paths: vec!["/request".into()],
        ..Default::default()
    };
    assert!(select_context(&source, &request).is_ok());
    source.result_json = Some(json!({}));
    assert_eq!(
        select_context(&source, &index_request()),
        Err("context_source_unavailable")
    );
    source.result_json = None;
    assert_eq!(
        select_context(&source, &index_request()),
        Err("context_result_unavailable")
    );
}

#[test]
fn paging_uses_verified_capability_stream_when_other_trace_fields_were_compacted() {
    let mut source = history(70);
    let trace = &mut source.result_json.as_mut().unwrap()["task_journal"]["trace"];
    let bytes = serde_json::to_vec(&trace["capability_results"]).unwrap();
    let hash = bytes.iter().fold(0xcbf29ce484222325u64, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x100000001b3)
    });
    trace["trace_storage"] = json!({"truncated":true,"evidence_streams":{"capability_results":format!("fnv64:{hash:016x}")}});
    let mut request = index_request();
    let first = select_context(&source, &request).unwrap();
    request.cursor = Some(first["pagination"]["next_cursor"].as_str().unwrap().into());
    assert_eq!(
        select_context(&source, &request).unwrap()["fields"]["/capabilities"][0]["index"],
        64
    );
    source.result_json.as_mut().unwrap()["task_journal"]["trace"]["capability_results"][0]
        ["status"] = json!("error");
    assert_eq!(
        select_context(&source, &index_request()),
        Err("context_source_compacted")
    );
}

#[test]
fn page_response_keeps_byte_budget_even_for_untrusted_index_fields() {
    let mut source = history(70);
    source.result_json.as_mut().unwrap()["task_journal"]["trace"]["capability_results"][0]
        ["capability"] = json!("x".repeat(25 * 1024));
    assert_eq!(
        select_context(&source, &index_request()),
        Err("context_value_too_large")
    );
    let source = history(70);
    let mut request = index_request();
    request.cursor = Some("x".repeat(1025));
    assert_eq!(
        select_context(&source, &request),
        Err("context_cursor_invalid")
    );
}

#[test]
fn previously_bounded_arguments_are_not_returned_as_complete_fields() {
    let mut source = history(1);
    source.result_json.as_mut().unwrap()["task_journal"]["trace"]["capability_results"][0]
        ["provenance"]["continuation_args_projection"] = json!({"truncated":true});
    let mut request = RecallRequest {
        field_paths: vec!["/capabilities/0/args/index".into()],
        ..Default::default()
    };
    assert_eq!(
        select_context(&source, &request),
        Err("context_field_compacted")
    );
    request.field_paths = vec!["/capabilities/0/data/index".into()];
    assert_eq!(
        select_context(&source, &request).unwrap()["fields"]["/capabilities/0/data/index"],
        0
    );
}

#[test]
fn malformed_scope_payloads_fail_closed() {
    let mut source = record();
    let mut current = record();
    source.task.payload_json = "{".into();
    current.task.payload_json = "{".into();
    assert!(!same_scope(&current, &source));
    source.task.payload_json = json!({"conversation_id":42}).to_string();
    current.task.payload_json = source.task.payload_json.clone();
    assert!(!same_scope(&current, &source));
}

#[tokio::test]
async fn every_page_rechecks_agent_owner_recipient_and_key_authorization() {
    let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    state.seed_test_auth_identity("page-owner", "admin");
    state.seed_test_auth_identity("foreign-owner", "user");
    let owner = crate::resolve_auth_identity_by_key(&state, "page-owner")
        .unwrap()
        .unwrap();
    let source = history(70);
    let current_id = Uuid::new_v4();
    {
        let db = state.core.db.get().unwrap();
        for (id, status, result) in [
            (
                source.task.task_id.clone(),
                "succeeded",
                source.result_json.clone(),
            ),
            (current_id.to_string(), "running", None),
        ] {
            db.execute("INSERT INTO tasks (task_id,user_id,chat_id,user_key,principal_id,channel,kind,payload_json,status,result_json,created_at,updated_at)
                VALUES (?1,1,2,'page-owner',?2,'ui','ask',?3,?4,?5,'1','1')",
                rusqlite::params![id,owner.principal_id,source.task.payload_json,status,result.map(|v|v.to_string())]).unwrap();
        }
    }
    let mut headers = HeaderMap::new();
    headers.insert(
        claw_core::product_identity::AUTH_KEY_HEADER,
        "page-owner".parse().unwrap(),
    );
    let first = || RecallRequest {
        context_ref: context_ref(&source.task.task_id),
        ..index_request()
    };
    let (status, Json(reply)) = recall(
        State(state.clone()),
        headers.clone(),
        Path(current_id),
        Json(first()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let token = reply.data.unwrap()["pagination"]["next_cursor"]
        .as_str()
        .unwrap()
        .to_string();
    let next = || RecallRequest {
        cursor: Some(token.clone()),
        ..first()
    };
    let (status, Json(reply)) = recall(
        State(state.clone()),
        headers.clone(),
        Path(current_id),
        Json(next()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        reply.data.unwrap()["fields"]["/capabilities"][0]["index"],
        64
    );
    for (column, value) in [
        (
            "payload_json",
            json!({"conversation_id":"one","agent_runtime_snapshot":{"agent_id":"different"}})
                .to_string(),
        ),
        ("channel", "wechat".into()),
        ("external_chat_id", "other-recipient".into()),
        ("principal_id", "other-owner".into()),
    ] {
        let original: Option<String> = state
            .core
            .db
            .get()
            .unwrap()
            .query_row(
                &format!("SELECT {column} FROM tasks WHERE task_id=?1"),
                [&source.task.task_id],
                |row| row.get(0),
            )
            .unwrap();
        state
            .core
            .db
            .get()
            .unwrap()
            .execute(
                &format!("UPDATE tasks SET {column}=?1 WHERE task_id=?2"),
                rusqlite::params![value, source.task.task_id],
            )
            .unwrap();
        let (status, _) = recall(
            State(state.clone()),
            headers.clone(),
            Path(current_id),
            Json(next()),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{column}");
        state
            .core
            .db
            .get()
            .unwrap()
            .execute(
                &format!("UPDATE tasks SET {column}=?1 WHERE task_id=?2"),
                rusqlite::params![original, source.task.task_id],
            )
            .unwrap();
    }
    let mut foreign = headers.clone();
    foreign.insert(
        claw_core::product_identity::AUTH_KEY_HEADER,
        "foreign-owner".parse().unwrap(),
    );
    assert_eq!(
        recall(
            State(state.clone()),
            foreign,
            Path(current_id),
            Json(next())
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        recall(
            State(state.clone()),
            HeaderMap::new(),
            Path(current_id),
            Json(next())
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    state
        .core
        .db
        .get()
        .unwrap()
        .execute(
            "UPDATE auth_keys SET enabled=0 WHERE user_key='page-owner'",
            [],
        )
        .unwrap();
    assert_eq!(
        recall(
            State(state.clone()),
            headers.clone(),
            Path(current_id),
            Json(next())
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    state
        .core
        .db
        .get()
        .unwrap()
        .execute(
            "UPDATE auth_keys SET enabled=1 WHERE user_key='page-owner'",
            [],
        )
        .unwrap();
    state
        .core
        .db
        .get()
        .unwrap()
        .execute("DELETE FROM tasks WHERE task_id=?1", [&source.task.task_id])
        .unwrap();
    assert_eq!(
        recall(State(state), headers, Path(current_id), Json(next()))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
}
