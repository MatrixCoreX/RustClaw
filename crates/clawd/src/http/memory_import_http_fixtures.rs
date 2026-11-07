use super::*;

pub(super) struct Env {
    pub(super) app: Router,
    pub(super) fixture: Fixture,
}
impl Env {
    pub(super) fn new() -> Self {
        Self::with_policy(|_| {})
    }
    pub(super) fn with_policy(change: impl FnOnce(&mut IngressPolicy)) -> Self {
        let fixture = Fixture::file();
        let mut state = fixture.state().clone();
        change(&mut state.policy.memory_import_ingress);
        let app = super::super::memory_routes::router().with_state(state);
        Self { fixture, app }
    }
    pub(super) async fn raw(
        &self,
        method: Method,
        path: &str,
        key: Option<&str>,
        body: Body,
        headers: HeaderMap,
    ) -> (StatusCode, HeaderMap, Vec<u8>) {
        let mut request = Request::builder().method(method).uri(path);
        if let Some(key) = key {
            request = request.header(claw_core::product_identity::AUTH_KEY_HEADER, key);
        }
        for (name, value) in &headers {
            request = request.header(name, value);
        }
        let response = self
            .app
            .clone()
            .oneshot(request.body(body).unwrap())
            .await
            .unwrap();
        let (parts, body) = response.into_parts();
        (
            parts.status,
            parts.headers,
            to_bytes(body, 4 * 1024 * 1024).await.unwrap().to_vec(),
        )
    }
    pub(super) async fn call(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let bytes = body
            .map(|v| serde_json::to_vec(&v).unwrap())
            .unwrap_or_default();
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::CONTENT_TYPE,
            "application/json".parse().unwrap(),
        );
        let (status, _, raw) = self
            .raw(
                method,
                path,
                Some(&self.fixture.key),
                Body::from(bytes),
                headers,
            )
            .await;
        (status, serde_json::from_slice(&raw).unwrap())
    }
    pub(super) async fn create(&self, raw: &[u8]) -> String {
        let (status, reply) = self
            .call(
                Method::POST,
                "/memory/import/uploads",
                Some(json!({
                    "file_digest":digest(raw),"total_bytes":raw.len()
                })),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{reply}");
        reply["data"]["upload_id"].as_str().unwrap().to_owned()
    }
    pub(super) async fn put(&self, id: &str, offset: usize, bytes: &[u8]) -> (StatusCode, Value) {
        let mut headers = HeaderMap::new();
        headers.insert("x-content-sha256", digest(bytes).parse().unwrap());
        let (status, _, raw) = self
            .raw(
                Method::PUT,
                &format!("/memory/import/uploads/{id}/chunks?offset={offset}"),
                Some(&self.fixture.key),
                Body::from(bytes.to_vec()),
                headers,
            )
            .await;
        (status, serde_json::from_slice(&raw).unwrap())
    }
    pub(super) async fn ready(&self, raw: &[u8]) -> String {
        let id = self.create(raw).await;
        let (status, policy) = self.call(Method::GET, "/memory/import/policy", None).await;
        assert_eq!(status, StatusCode::OK);
        let chunk = policy["data"]["max_chunk_bytes"]
            .as_str()
            .unwrap()
            .parse::<usize>()
            .unwrap()
            .min(64 * 1024);
        for (index, bytes) in raw.chunks(chunk).enumerate() {
            let (status, reply) = self.put(&id, index * chunk, bytes).await;
            assert_eq!(status, StatusCode::OK, "{reply}");
        }
        id
    }
}
pub(super) fn digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}
pub(super) fn export(contents: Vec<String>) -> crate::memory::ux::MemoryExport {
    let items=contents.into_iter().map(|content|serde_json::from_value(json!({
        "id":"source_record","revision":1,"kind":"fact","scope_kind":"principal","origin":"explicit",
        "status":"active","content":content,"source":"user","evidence_available":false,"trust_tier":"user_asserted",
        "updated_at_ts":1,"expires_at_ts":null,"supersedes_memory_id":null,"last_recalled_at_ts":null,"freshness":"fresh"
    })).unwrap()).collect::<Vec<crate::memory::ux::MemoryListItem>>();
    crate::memory::ux::MemoryExport {
        schema_version: 1,
        exported_at_ts: 100,
        scope_kind: "principal".into(),
        checksum: digest(&serde_json::to_vec(&items).unwrap()),
        items,
    }
}
pub(super) fn request_confirmation(preview: &Value) -> Value {
    json!({"import_id":preview["data"]["import_id"],
        "expected_payload_digest":preview["data"]["payload_digest"]})
}
