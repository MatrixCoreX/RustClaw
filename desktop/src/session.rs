use crate::{
    credentials::{self, LoginSecret},
    profile::Profile,
    transport::{bytes_body, small_json, ByteStream, Transport, WireResponse},
    Result,
};
use http::{HeaderMap, Method};
use serde::Serialize;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use zeroize::Zeroizing;

pub struct Session {
    pub id: Uuid,
    pub profile: Profile,
    pub transport: Transport,
    pub cancelled: CancellationToken,
    auth: Mutex<Auth>,
}
#[derive(Default)]
struct Auth {
    key: Zeroizing<String>,
    csrf: Zeroizing<String>,
    identity: Option<Value>,
}
#[derive(Serialize)]
pub struct SessionInfo {
    pub id: Uuid,
    pub profile: Profile,
    pub origin: String,
    pub identity: Option<Value>,
}
#[derive(Serialize)]
pub struct LoginResult {
    pub session: SessionInfo,
    pub remembered: bool,
    pub warning: Option<String>,
}

impl Session {
    pub async fn connect(profile: Profile, password: Zeroizing<String>) -> Result<Arc<Self>> {
        Self::connect_with_key(profile, password, None, None).await
    }
    pub async fn connect_with_key(
        profile: Profile,
        password: Zeroizing<String>,
        key: Option<Zeroizing<String>>,
        passphrase: Option<Zeroizing<String>>,
    ) -> Result<Arc<Self>> {
        let transport = Transport::connect_with_key(
            &profile.connection,
            &password,
            key.as_ref().map(|s| s.as_str()),
            passphrase.as_ref().map(|s| s.as_str()),
        )
        .await?;
        Ok(Arc::new(Self {
            id: Uuid::new_v4(),
            profile,
            transport,
            cancelled: CancellationToken::new(),
            auth: Mutex::new(Auth::default()),
        }))
    }
    pub async fn info(&self) -> SessionInfo {
        SessionInfo {
            id: self.id,
            profile: self.profile.clone(),
            origin: self.transport.origin.origin().ascii_serialization(),
            identity: self.auth.lock().await.identity.clone(),
        }
    }
    pub async fn login(&self, input: LoginSecret, remember: bool) -> Result<LoginResult> {
        let mut auth = self.auth.lock().await;
        *auth = Auth::default();
        if self.cancelled.is_cancelled() {
            return Err("connection_closed".into());
        }
        let headers = HeaderMap::from_iter([(
            http::header::CONTENT_TYPE,
            http::HeaderValue::from_static("application/json"),
        )]);
        match input.mode.as_str() {
            "key" => {
                let body = json!({"user_key": input.secret}).to_string();
                let (status, value) = small_json(
                    self.transport
                        .send(
                            Method::POST,
                            "/v1/auth/ui-key/verify",
                            headers,
                            Some(bytes_body(body)),
                        )
                        .await?,
                )
                .await?;
                validate_login(status, &value)?;
                auth.key = Zeroizing::new(input.secret.clone());
            }
            "password" => {
                let body =
                    json!({"username": input.username, "password": input.secret}).to_string();
                let (status, value) = small_json(
                    self.transport
                        .send(Method::POST, "/webd/login", headers, Some(bytes_body(body)))
                        .await?,
                )
                .await?;
                validate_login(status, &value)?;
                let csrf = login_csrf_token(&value)?;
                auth.csrf = Zeroizing::new(csrf.into());
            }
            _ => return Err("login_mode_invalid".into()),
        }
        let mut headers = HeaderMap::new();
        if !auth.key.is_empty() {
            headers.insert(
                "x-agent-key",
                auth.key.parse().map_err(|_| "credential_invalid")?,
            );
        }
        let (status, value) = small_json(
            self.transport
                .send(Method::GET, "/v1/auth/me", headers, None)
                .await?,
        )
        .await?;
        validate_login(status, &value)?;
        if self.cancelled.is_cancelled() {
            return Err("connection_closed".into());
        }
        let mut identity = value["data"].clone();
        if let Some(object) = identity.as_object_mut() {
            object.remove("user_key");
        }
        auth.identity = Some(identity);
        drop(auth);
        let warning = if remember {
            let id = self.profile.id;
            tokio::task::spawn_blocking(move || credentials::save(id, &input))
                .await
                .map_err(|_| "credential_store_unavailable")?
                .err()
        } else {
            None
        };
        Ok(LoginResult {
            session: self.info().await,
            remembered: remember && warning.is_none(),
            warning,
        })
    }
    pub async fn request(
        &self,
        method: Method,
        path: &str,
        mut headers: HeaderMap,
        body: Option<ByteStream>,
    ) -> Result<WireResponse> {
        if self.cancelled.is_cancelled() {
            return Err("connection_closed".into());
        }
        let auth = self.auth.lock().await;
        if auth.identity.is_none() {
            return Err("login_required".into());
        }
        if !auth.key.is_empty() {
            headers.insert(
                "x-agent-key",
                auth.key.parse().map_err(|_| "credential_invalid")?,
            );
        }
        if !matches!(method, Method::GET | Method::HEAD | Method::OPTIONS) && !auth.csrf.is_empty()
        {
            headers.insert(
                "x-agent-csrf-token",
                auth.csrf.parse().map_err(|_| "csrf_invalid")?,
            );
        }
        drop(auth);
        let response = tokio::select! {
            _ = self.cancelled.cancelled() => Err("connection_closed".into()),
            result = self.transport.send(method, path, headers, body) => result,
        }?;
        if path == "/v1/auth/me" {
            let (status, mut value) = small_json(response).await?;
            if let Some(data) = value.get_mut("data").and_then(Value::as_object_mut) {
                data.remove("user_key");
            }
            return Ok(WireResponse {
                status,
                headers: HeaderMap::from_iter([(
                    http::header::CONTENT_TYPE,
                    http::HeaderValue::from_static("application/json"),
                )]),
                body: bytes_body(value.to_string()),
            });
        }
        Ok(response)
    }
    pub async fn close(&self) {
        self.cancelled.cancel();
        *self.auth.lock().await = Auth::default();
        self.transport.close().await;
    }
}
// WEBD emits UUID.simple(): 32 lowercase hexadecimal characters, as the web UI expects.
fn login_csrf_token(value: &Value) -> Result<&str> {
    let token = value["data"]["csrf_token"].as_str().ok_or("csrf_missing")?;
    if token.len() != 32
        || !token
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Err("csrf_invalid".into());
    }
    Ok(token)
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;

fn validate_login(status: u16, value: &Value) -> Result<()> {
    if status == 429 {
        return Err("login_temporarily_locked".into());
    }
    if !(200..300).contains(&status) || value["ok"] != true {
        return Err("login_rejected".into());
    }
    Ok(())
}
