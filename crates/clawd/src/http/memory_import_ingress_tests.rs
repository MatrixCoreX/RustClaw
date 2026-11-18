use super::*;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

#[tokio::test]
async fn policy_projects_config_and_requires_authentication() {
    let env = Env::with_policy(|p| {
        p.json_bytes = 1024;
        p.upload_bytes = 256;
    });
    let (status, _, raw) = env
        .raw(
            Method::GET,
            "/memory/import/policy",
            None,
            Body::empty(),
            HeaderMap::new(),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(!serde_json::from_slice::<Value>(&raw).unwrap()["ok"]
        .as_bool()
        .unwrap());
    let (status, policy) = env.call(Method::GET, "/memory/import/policy", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(policy["data"]["max_chunk_bytes"], "256");
    assert_eq!(policy["data"]["max_value_bytes"], "1024");
    assert_eq!(policy["data"]["file_limit_scope"], "memory_domain_budget");
    assert!(policy["data"]["max_items"].is_null());
    assert!(policy["data"]["max_content_characters"].is_null());
}

#[tokio::test]
async fn create_rejects_payload_supplied_ownership_and_does_not_write_a_session() {
    let env = Env::new();
    let (status, reply) = env
        .call(
            Method::POST,
            "/memory/import/uploads",
            Some(json!({
                "file_digest":digest(b"test"),"total_bytes":4,"principal_id":"forged-owner"
            })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(reply["error"], "memory_import_upload_metadata_invalid");
    let db = env.fixture.state().core.db.get().unwrap();
    let count: i64 = db
        .query_row("SELECT COUNT(*) FROM memory_import_uploads", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn identical_chunk_retry_is_idempotent_and_out_of_order_bytes_do_not_advance() {
    let env = Env::new();
    let id = env.create(b"abcdefgh").await;
    let (status, first) = env.put(&id, 0, b"abcd").await;
    assert_eq!(status, StatusCode::OK);
    let (status, again) = env.put(&id, 0, b"abcd").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(again["data"]["revision"], first["data"]["revision"]);
    let (status, reply) = env.put(&id, 5, b"efg").await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(reply["error"], "memory_import_upload_offset_conflict");
    let (_, current) = env
        .call(Method::GET, &format!("/memory/import/uploads/{id}"), None)
        .await;
    assert_eq!(current["data"]["received_bytes"], "4");
}

#[tokio::test]
async fn declared_and_streamed_oversize_chunks_return_json_413_without_partial_append() {
    let env = Env::with_policy(|p| {
        p.json_bytes = 1024;
        p.upload_bytes = 16;
    });
    let bytes = vec![b'a'; 20];
    let id = env.create(&bytes).await;
    for known_length in [true, false] {
        let mut headers = HeaderMap::new();
        headers.insert("x-content-sha256", digest(&bytes).parse().unwrap());
        if known_length {
            headers.insert("content-length", "20".parse().unwrap());
        }
        let body = if known_length {
            Body::from(bytes.clone())
        } else {
            Body::from_stream(futures_util::stream::iter([
                Ok::<_, std::io::Error>(Bytes::from(vec![b'a'; 10])),
                Ok::<_, std::io::Error>(Bytes::from(vec![b'a'; 10])),
            ]))
        };
        let (status, _, raw) = env
            .raw(
                Method::PUT,
                &format!("/memory/import/uploads/{id}/chunks?offset=0"),
                Some(&env.fixture.key),
                body,
                headers,
            )
            .await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(
            serde_json::from_slice::<Value>(&raw).unwrap()["error"],
            "memory_import_upload_chunk_too_large"
        );
    }
    let (_, current) = env
        .call(Method::GET, &format!("/memory/import/uploads/{id}"), None)
        .await;
    assert_eq!(current["data"]["received_bytes"], "0");
}

#[tokio::test]
async fn another_owner_is_rejected_before_any_chunk_body_is_consumed() {
    let env = Env::new();
    let id = env.create(b"test").await;
    let key = crate::repo::auth::create_auth_key(env.fixture.state(), "user").unwrap();
    let consumed = Arc::new(AtomicBool::new(false));
    let observed = consumed.clone();
    let body = Body::from_stream(futures_util::stream::once(async move {
        observed.store(true, Ordering::SeqCst);
        Ok::<_, std::io::Error>(Bytes::from_static(b"test"))
    }));
    let mut headers = HeaderMap::new();
    headers.insert("x-content-sha256", digest(b"test").parse().unwrap());
    let (status, _, raw) = env
        .raw(
            Method::PUT,
            &format!("/memory/import/uploads/{id}/chunks?offset=0"),
            Some(&key),
            body,
            headers,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        serde_json::from_slice::<Value>(&raw).unwrap()["error"],
        "memory_import_upload_not_found"
    );
    assert!(!consumed.load(Ordering::SeqCst));
}

#[tokio::test]
async fn original_key_revocation_during_body_receipt_rejects_persistence() {
    let env = Env::new();
    let id = env.create(b"test").await;
    let actor = crate::resolve_auth_identity_by_key(env.fixture.state(), &env.fixture.key)
        .unwrap()
        .unwrap();
    {
        let db = env.fixture.state().core.db.get().unwrap();
        let sibling = format!("http-import-sibling-{}", uuid::Uuid::new_v4().simple());
        db.execute(
            "INSERT INTO auth_keys(user_key,role,enabled,created_at,principal_id)
            VALUES(?1,'user',1,'1',?2)",
            rusqlite::params![sibling, actor.principal_id],
        )
        .unwrap();
        db.execute("INSERT INTO credential_bindings(binding_id,credential_digest,principal_id,status,created_at)
            VALUES(?1,?2,?3,'active','1')",rusqlite::params![uuid::Uuid::new_v4().simple().to_string(),
                crate::repo::auth::credential_digest(&sibling),actor.principal_id]).unwrap();
    }
    let state = env.fixture.state().clone();
    let key = env.fixture.key.clone();
    let body = Body::from_stream(futures_util::stream::once(async move {
        let db = state.core.db.get().unwrap();
        db.execute("UPDATE auth_keys SET enabled=0 WHERE user_key=?1", [key])
            .unwrap();
        Ok::<_, std::io::Error>(Bytes::from_static(b"test"))
    }));
    let mut headers = HeaderMap::new();
    headers.insert("x-content-sha256", digest(b"test").parse().unwrap());
    let (status, _, raw) = env
        .raw(
            Method::PUT,
            &format!("/memory/import/uploads/{id}/chunks?offset=0"),
            Some(&env.fixture.key),
            body,
            headers,
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(!serde_json::from_slice::<Value>(&raw).unwrap()["ok"]
        .as_bool()
        .unwrap());
    let db = env.fixture.state().core.db.get().unwrap();
    assert!(
        crate::repo::auth::require_active_principal_credential(&db, &actor.principal_id).is_ok()
    );
    let received: i64 = db
        .query_row(
            "SELECT received_bytes FROM memory_import_uploads WHERE upload_id=?1",
            [&id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(received, 0);
}

#[tokio::test]
async fn malformed_metadata_query_and_digest_have_structured_errors() {
    let env = Env::new();
    let id = env.create(b"test").await;
    for (offset, digest_value) in [("-1", digest(b"test")), ("0", "malformed".to_owned())] {
        let mut headers = HeaderMap::new();
        headers.insert("x-content-sha256", digest_value.parse().unwrap());
        let (status, _, raw) = env
            .raw(
                Method::PUT,
                &format!("/memory/import/uploads/{id}/chunks?offset={offset}"),
                Some(&env.fixture.key),
                Body::from("test"),
                headers,
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(serde_json::from_slice::<Value>(&raw).unwrap()["ok"], false);
    }
    let (status, _, raw) = env
        .raw(
            Method::POST,
            "/memory/import/uploads",
            Some(&env.fixture.key),
            Body::from("{"),
            HeaderMap::new(),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        serde_json::from_slice::<Value>(&raw).unwrap()["error"],
        "memory_import_upload_metadata_invalid"
    );
}
