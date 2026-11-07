use super::*;

#[tokio::test]
async fn status_and_policy_reads_do_not_need_the_sqlite_writer_lock() {
    let env = Env::new();
    let id = env.create(b"test").await;
    let path = {
        let db = env.fixture.state().core.db.get().unwrap();
        db.path().unwrap().to_owned()
    };
    let competing = rusqlite::Connection::open(path).unwrap();
    competing.execute_batch("BEGIN IMMEDIATE").unwrap();
    for path in [
        "/memory/import/policy".to_owned(),
        format!("/memory/import/uploads/{id}"),
    ] {
        let (status, reply) = env.call(Method::GET, &path, None).await;
        assert_eq!(status, StatusCode::OK, "{reply}");
    }
    competing.execute_batch("ROLLBACK").unwrap();
}

#[tokio::test]
async fn missing_migration_returns_json_unavailable_without_read_repair() {
    let env = Env::new();
    let id = env.create(b"test").await;
    {
        let db = env.fixture.state().core.db.get().unwrap();
        db.execute("DELETE FROM runtime_schema_migrations WHERE migration_id='032_memory_import_uploads_v1'",[]).unwrap();
    }
    for path in [
        "/memory/import/policy".to_owned(),
        format!("/memory/import/uploads/{id}"),
    ] {
        let (status, reply) = env.call(Method::GET, &path, None).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(reply["error"], "memory_import_upload_schema_unavailable");
    }
    let db = env.fixture.state().core.db.get().unwrap();
    let count:i64=db.query_row("SELECT COUNT(*) FROM runtime_schema_migrations WHERE migration_id='032_memory_import_uploads_v1'",
        [],|r|r.get(0)).unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn preview_cancellation_requires_its_owner_and_removes_pending_plaintext() {
    let env = Env::new();
    let raw = serde_json::to_vec(&export(vec!["private pending content".into()])).unwrap();
    let id = env.ready(&raw).await;
    let (status, preview) = env
        .call(
            Method::POST,
            &format!("/memory/import/uploads/{id}/preview"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    let other = crate::repo::auth::create_auth_key(env.fixture.state(), "user").unwrap();
    let (status, _, _) = env
        .raw(
            Method::DELETE,
            &format!("/memory/import/uploads/{id}"),
            Some(&other),
            Body::empty(),
            HeaderMap::new(),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, reply) = env
        .call(
            Method::DELETE,
            &format!("/memory/import/uploads/{id}"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{reply}");
    assert_eq!(reply["data"]["state"], "cancelled");
    let (status, _) = env
        .call(
            Method::POST,
            "/memory/import/confirm",
            Some(request_confirmation(&preview)),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let db = env.fixture.state().core.db.get().unwrap();
    for table in ["memory_import_upload_items", "memory_import_upload_chunks"] {
        let count: i64 = db
            .query_row(
                &format!("SELECT COUNT(*) FROM {table} WHERE upload_id=?1"),
                [&id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
    }
}

#[tokio::test]
async fn invalid_ingress_configuration_does_not_block_status_or_cancellation_recovery() {
    let env = Env::with_policy(|p| p.json_bytes = 0);
    let actor = crate::resolve_auth_identity_by_key(env.fixture.state(), &env.fixture.key)
        .unwrap()
        .unwrap();
    let id = {
        let db = env.fixture.state().core.db.get().unwrap();
        crate::memory::import_upload::create(
            &db,
            &actor,
            &crate::memory::import_upload::CreateUpload {
                file_digest: digest(b"test"),
                total_bytes: 4,
            },
            crate::memory::import_upload::UploadPolicy {
                chunk_bytes: 64,
                parser_lease_seconds: 10,
            },
            100,
        )
        .unwrap()
        .upload_id
    };
    let (status, reply) = env.call(Method::GET, "/memory/import/policy", None).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(reply["error"], "memory_import_ingress_unavailable");
    assert_eq!(
        env.call(Method::GET, &format!("/memory/import/uploads/{id}"), None)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        env.call(
            Method::DELETE,
            &format!("/memory/import/uploads/{id}"),
            None
        )
        .await
        .0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn body_receipt_deadline_is_structured_and_does_not_append_partial_bytes() {
    let env = Env::with_policy(|p| p.upload_read_seconds = 1);
    let id = env.create(b"test").await;
    let mut headers = HeaderMap::new();
    headers.insert("x-content-sha256", digest(b"test").parse().unwrap());
    let body = Body::from_stream(futures_util::stream::pending::<Result<Bytes, std::io::Error>>());
    let (status, _, raw) = env
        .raw(
            Method::PUT,
            &format!("/memory/import/uploads/{id}/chunks?offset=0"),
            Some(&env.fixture.key),
            body,
            headers,
        )
        .await;
    assert_eq!(status, StatusCode::REQUEST_TIMEOUT);
    assert_eq!(
        serde_json::from_slice::<Value>(&raw).unwrap()["error"],
        "memory_import_body_read_timeout"
    );
    let (_, reply) = env
        .call(Method::GET, &format!("/memory/import/uploads/{id}"), None)
        .await;
    assert_eq!(reply["data"]["received_bytes"], "0");
}

#[tokio::test]
async fn pressure_rejection_exposes_its_typed_reason_and_keeps_recovery_available() {
    let env = Env::new();
    let id = env.create(b"test").await;
    {
        let db = env.fixture.state().core.db.get().unwrap();
        db.execute("UPDATE memory_storage_pressure SET state='explicit_write_blocked',revision=revision+1 WHERE singleton_id=1",[]).unwrap();
    }
    let (status, reply) = env.put(&id, 0, b"test").await;
    assert_eq!(status, StatusCode::INSUFFICIENT_STORAGE, "{reply}");
    assert_eq!(reply["error"], "memory_storage_write_blocked");
    let (_, reply) = env
        .call(Method::GET, &format!("/memory/import/uploads/{id}"), None)
        .await;
    assert_eq!(reply["data"]["received_bytes"], "0");
    assert_eq!(
        env.call(
            Method::DELETE,
            &format!("/memory/import/uploads/{id}"),
            None
        )
        .await
        .0,
        StatusCode::OK
    );
}
