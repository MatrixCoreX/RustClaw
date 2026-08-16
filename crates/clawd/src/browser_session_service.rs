use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures_util::StreamExt;
use serde_json::{json, Value};
use sha2::Digest;
use tokio::io::AsyncWriteExt;
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore};
use tokio_util::codec::{FramedRead, LinesCodec};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const PROTOCOL_VERSION: u64 = 1;
const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const DEFAULT_IDLE_LEASE: Duration = Duration::from_secs(300);
const DEFAULT_MAX_LIFETIME: Duration = Duration::from_secs(1800);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BrowserSessionBinding {
    pub(crate) actor_ref: String,
    pub(crate) task_id: String,
    pub(crate) registry_generation: u64,
    pub(crate) registry_digest: String,
    pub(crate) policy_digest: String,
}

#[derive(Debug)]
pub(crate) struct BrowserSessionError {
    pub(crate) code: String,
    pub(crate) message_key: String,
    pub(crate) retryable: bool,
    pub(crate) details: Value,
}

impl BrowserSessionError {
    fn new(code: impl Into<String>, retryable: bool, details: Value) -> Self {
        let code = code.into();
        Self {
            message_key: format!("browser_session.{}", code.to_ascii_lowercase()),
            code,
            retryable,
            details,
        }
    }
}

#[derive(Clone)]
pub(crate) struct BrowserSessionService {
    inner: Arc<ServiceInner>,
}

struct ServiceInner {
    workspace_root: PathBuf,
    bridge_path: PathBuf,
    sessions: Mutex<HashMap<String, Arc<LiveSession>>>,
    session_slots: Arc<Semaphore>,
    idle_lease: Duration,
    max_lifetime: Duration,
}

struct LiveSession {
    id: String,
    binding: BrowserSessionBinding,
    created_at: u64,
    max_expires_at: u64,
    last_used_at: StdMutex<u64>,
    lease_expires_at: StdMutex<u64>,
    last_safe_snapshot: StdMutex<Option<Value>>,
    bridge: Mutex<BridgeProcess>,
    _session_slot: OwnedSemaphorePermit,
}

struct BridgeProcess {
    child: Child,
    stdin: ChildStdin,
    stdout: FramedRead<tokio::process::ChildStdout, LinesCodec>,
}

impl Drop for BridgeProcess {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Some(process_id) = self.child.id() {
            // The bridge is its process-group leader. Killing the group prevents
            // Chromium helpers from surviving a graceful host shutdown where no
            // async close request can still be awaited.
            unsafe {
                libc::kill(-(process_id as i32), libc::SIGKILL);
            }
        }
        let _ = self.child.start_kill();
    }
}

impl BrowserSessionService {
    pub(crate) fn new(workspace_root: &Path) -> Self {
        let workspace_root = workspace_root
            .canonicalize()
            .unwrap_or_else(|_| workspace_root.to_path_buf());
        let inner = Arc::new(ServiceInner {
            bridge_path: workspace_root.join("crates/skills/browser_web/browser_session_bridge.js"),
            workspace_root,
            sessions: Mutex::new(HashMap::new()),
            session_slots: Arc::new(Semaphore::new(detect_session_capacity())),
            idle_lease: DEFAULT_IDLE_LEASE,
            max_lifetime: DEFAULT_MAX_LIFETIME,
        });
        Self::spawn_reaper(&inner);
        Self { inner }
    }

    fn spawn_reaper(inner: &Arc<ServiceInner>) {
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let weak = Arc::downgrade(inner);
        runtime.spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                let Some(inner) = weak.upgrade() else { break };
                let now = unix_now();
                let expired = {
                    let sessions = inner.sessions.lock().await;
                    sessions
                        .iter()
                        .filter_map(|(id, session)| {
                            let lease = session.lease_expires_at.lock().map(|v| *v).unwrap_or(0);
                            (now >= lease || now >= session.max_expires_at).then(|| id.clone())
                        })
                        .collect::<Vec<_>>()
                };
                for session_id in expired {
                    shutdown_session(&inner, &session_id).await;
                }
            }
        });
    }

    pub(crate) async fn open(
        &self,
        binding: BrowserSessionBinding,
        mut input: Value,
        cancellation: Option<CancellationToken>,
    ) -> Result<Value, BrowserSessionError> {
        if !self.inner.bridge_path.is_file() {
            return Err(BrowserSessionError::new(
                "BROWSER_RUNTIME_UNAVAILABLE",
                false,
                json!({"reason": "bridge_missing"}),
            ));
        }
        let session_slot = self
            .inner
            .session_slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| {
                BrowserSessionError::new(
                    "BROWSER_RESOURCE_LIMIT",
                    true,
                    json!({"reason": "session_capacity"}),
                )
            })?;
        let session_id = Uuid::new_v4().simple().to_string();
        let now = unix_now();
        let task_ref = hex::encode(sha2::Sha256::digest(binding.task_id.as_bytes()));
        let artifact_root = self
            .inner
            .workspace_root
            .join(claw_core::workspace_state::WORKSPACE_STATE_DIR_NAME)
            .join("artifacts/browser-sessions")
            .join(task_ref)
            .join(&session_id);
        let bridge = spawn_bridge(&self.inner.workspace_root, &self.inner.bridge_path).await?;
        let session = Arc::new(LiveSession {
            id: session_id.clone(),
            binding,
            created_at: now,
            max_expires_at: now.saturating_add(self.inner.max_lifetime.as_secs()),
            last_used_at: StdMutex::new(now),
            lease_expires_at: StdMutex::new(now.saturating_add(self.inner.idle_lease.as_secs())),
            last_safe_snapshot: StdMutex::new(None),
            bridge: Mutex::new(bridge),
            _session_slot: session_slot,
        });
        if !input.is_object() {
            input = json!({});
        }
        let object = input.as_object_mut().expect("object normalized");
        object.insert(
            "command".to_string(),
            Value::String("session_open".to_string()),
        );
        object.insert(
            "workspace_root".to_string(),
            Value::String(self.inner.workspace_root.display().to_string()),
        );
        object.insert(
            "artifact_root".to_string(),
            Value::String(artifact_root.display().to_string()),
        );
        match call_bridge(&session, input, cancellation).await {
            Ok(result) => {
                self.inner
                    .sessions
                    .lock()
                    .await
                    .insert(session_id, session.clone());
                Ok(project_result(
                    &session,
                    compact_session_open_result(result),
                ))
            }
            Err(error) => {
                kill_bridge(&session).await;
                Err(error)
            }
        }
    }

    pub(crate) async fn request(
        &self,
        session_id: &str,
        binding: &BrowserSessionBinding,
        command: &str,
        mut input: Value,
        cancellation: Option<CancellationToken>,
    ) -> Result<Value, BrowserSessionError> {
        let session = self.lookup_bound(session_id, binding).await?;
        if !input.is_object() {
            input = json!({});
        }
        input
            .as_object_mut()
            .expect("object normalized")
            .insert("command".to_string(), Value::String(command.to_string()));
        match call_bridge(&session, input, cancellation).await {
            Ok(result) => Ok(project_result(&session, result)),
            Err(error) => {
                let error = with_last_safe_snapshot(&session, error);
                if bridge_error_is_fatal(&error.code) {
                    shutdown_session(&self.inner, session_id).await;
                }
                Err(error)
            }
        }
    }

    pub(crate) async fn close(
        &self,
        session_id: &str,
        binding: &BrowserSessionBinding,
        cancellation: Option<CancellationToken>,
    ) -> Result<Value, BrowserSessionError> {
        let session = self.lookup_bound(session_id, binding).await?;
        let response = call_bridge(&session, json!({"command": "session_close"}), cancellation)
            .await
            .map(|result| project_result(&session, result));
        self.inner.sessions.lock().await.remove(session_id);
        kill_bridge(&session).await;
        response
    }

    pub(crate) async fn close_task_sessions(&self, task_id: &str) -> usize {
        let sessions = {
            let mut live = self.inner.sessions.lock().await;
            let session_ids = live
                .iter()
                .filter_map(|(session_id, session)| {
                    (session.binding.task_id == task_id).then(|| session_id.clone())
                })
                .collect::<Vec<_>>();
            session_ids
                .into_iter()
                .filter_map(|session_id| live.remove(&session_id))
                .collect::<Vec<_>>()
        };
        let count = sessions.len();
        for session in sessions {
            kill_bridge(&session).await;
        }
        count
    }

    async fn lookup_bound(
        &self,
        session_id: &str,
        binding: &BrowserSessionBinding,
    ) -> Result<Arc<LiveSession>, BrowserSessionError> {
        let session = self.inner.sessions.lock().await.get(session_id).cloned();
        let Some(session) = session else {
            return Err(BrowserSessionError::new(
                "BROWSER_SESSION_LOST",
                true,
                json!({"session_id": session_id}),
            ));
        };
        if &session.binding != binding {
            return Err(BrowserSessionError::new(
                "BROWSER_SESSION_BINDING_MISMATCH",
                false,
                json!({"session_id": session_id}),
            ));
        }
        let now = unix_now();
        let lease = session.lease_expires_at.lock().map(|v| *v).unwrap_or(0);
        if now >= lease || now >= session.max_expires_at {
            shutdown_session(&self.inner, session_id).await;
            return Err(BrowserSessionError::new(
                "BROWSER_SESSION_EXPIRED",
                true,
                json!({"session_id": session_id}),
            ));
        }
        Ok(session)
    }
}

async fn spawn_bridge(
    workspace_root: &Path,
    bridge_path: &Path,
) -> Result<BridgeProcess, BrowserSessionError> {
    let mut command = Command::new("node");
    command
        .arg(bridge_path)
        .current_dir(workspace_root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .env_clear();
    for key in [
        "PATH",
        "HOME",
        "LANG",
        "LC_ALL",
        "TMPDIR",
        "SSL_CERT_FILE",
        "SSL_CERT_DIR",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "NO_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
        "no_proxy",
    ] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    crate::skills::place_subprocess_in_own_process_group(&mut command);
    let mut child = command.spawn().map_err(|error| {
        BrowserSessionError::new(
            "BROWSER_RUNTIME_UNAVAILABLE",
            false,
            json!({"provider_error_kind": error.kind().to_string()}),
        )
    })?;
    let stdin = child.stdin.take().ok_or_else(|| {
        BrowserSessionError::new("BROWSER_BRIDGE_FAILED", true, json!({"stream": "stdin"}))
    })?;
    let stdout = child.stdout.take().ok_or_else(|| {
        BrowserSessionError::new("BROWSER_BRIDGE_FAILED", true, json!({"stream": "stdout"}))
    })?;
    Ok(BridgeProcess {
        child,
        stdin,
        stdout: FramedRead::new(stdout, LinesCodec::new_with_max_length(MAX_RESPONSE_BYTES)),
    })
}

async fn call_bridge(
    session: &Arc<LiveSession>,
    mut request: Value,
    cancellation: Option<CancellationToken>,
) -> Result<Value, BrowserSessionError> {
    let request_id = Uuid::new_v4().simple().to_string();
    let object = request.as_object_mut().ok_or_else(|| {
        BrowserSessionError::new("INVALID_ARGUMENT", false, json!({"field": "request"}))
    })?;
    object.insert("schema_version".to_string(), Value::from(PROTOCOL_VERSION));
    object.insert("request_id".to_string(), Value::String(request_id.clone()));
    let mut payload = serde_json::to_vec(&request).map_err(|_| {
        BrowserSessionError::new("INVALID_ARGUMENT", false, json!({"field": "request"}))
    })?;
    payload.push(b'\n');

    let operation = async {
        let mut bridge = session.bridge.lock().await;
        bridge
            .stdin
            .write_all(&payload)
            .await
            .map_err(bridge_io_error)?;
        bridge.stdin.flush().await.map_err(bridge_io_error)?;
        let line = bridge
            .stdout
            .next()
            .await
            .ok_or_else(|| {
                BrowserSessionError::new("BROWSER_SESSION_LOST", true, json!({"stream": "stdout"}))
            })?
            .map_err(|error| {
                BrowserSessionError::new(
                    "BROWSER_BRIDGE_FAILED",
                    true,
                    json!({"provider_error_kind": error.to_string()}),
                )
            })?;
        let response: Value = serde_json::from_str(&line).map_err(|_| {
            BrowserSessionError::new("BROWSER_BRIDGE_PROTOCOL_ERROR", true, json!({}))
        })?;
        if response.get("request_id").and_then(Value::as_str) != Some(request_id.as_str()) {
            return Err(BrowserSessionError::new(
                "BROWSER_BRIDGE_PROTOCOL_ERROR",
                true,
                json!({"reason": "request_id_mismatch"}),
            ));
        }
        if response.get("status").and_then(Value::as_str) == Some("error") {
            return Err(BrowserSessionError {
                code: response
                    .get("error_code")
                    .and_then(Value::as_str)
                    .unwrap_or("BROWSER_BRIDGE_FAILED")
                    .to_string(),
                message_key: response
                    .get("message_key")
                    .and_then(Value::as_str)
                    .unwrap_or("browser_session.browser_bridge_failed")
                    .to_string(),
                retryable: response
                    .get("retryable")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                details: response
                    .get("details")
                    .cloned()
                    .unwrap_or_else(|| json!({})),
            });
        }
        response.get("result").cloned().ok_or_else(|| {
            BrowserSessionError::new("BROWSER_BRIDGE_PROTOCOL_ERROR", true, json!({}))
        })
    };

    let result = if let Some(cancellation) = cancellation {
        tokio::select! {
            _ = cancellation.cancelled() => Err(BrowserSessionError::new(
                "BROWSER_SESSION_CANCELLED", false, json!({"task_id": session.binding.task_id})
            )),
            result = tokio::time::timeout(REQUEST_TIMEOUT, operation) => match result {
                Ok(result) => result,
                Err(_) => Err(BrowserSessionError::new("BROWSER_ACTION_TIMEOUT", true, json!({}))),
            },
        }
    } else {
        match tokio::time::timeout(REQUEST_TIMEOUT, operation).await {
            Ok(result) => result,
            Err(_) => Err(BrowserSessionError::new(
                "BROWSER_ACTION_TIMEOUT",
                true,
                json!({}),
            )),
        }
    }?;
    let now = unix_now();
    if let Ok(mut value) = session.last_used_at.lock() {
        *value = now;
    }
    if let Ok(mut value) = session.lease_expires_at.lock() {
        *value = now.saturating_add(DEFAULT_IDLE_LEASE.as_secs());
    }
    if let Some(snapshot) = result
        .get("snapshot")
        .cloned()
        .or_else(|| result.get("snapshot_id").is_some().then(|| result.clone()))
    {
        if let Ok(mut last) = session.last_safe_snapshot.lock() {
            *last = Some(snapshot);
        }
    }
    Ok(result)
}

fn compact_session_open_result(mut result: Value) -> Value {
    if let Some(object) = result.as_object_mut() {
        object.remove("snapshot");
    }
    result
}

fn project_result(session: &LiveSession, result: Value) -> Value {
    let lease_expires_at = session.lease_expires_at.lock().map(|v| *v).unwrap_or(0);
    json!({
        "schema_version": 1,
        "session_id": session.id,
        "session_generation": 1,
        "actor_ref": session.binding.actor_ref,
        "task_id": session.binding.task_id,
        "registry_generation": session.binding.registry_generation,
        "registry_generation_digest": session.binding.registry_digest,
        "policy_digest": session.binding.policy_digest,
        "created_at": session.created_at,
        "lease_expires_at": lease_expires_at,
        "max_expires_at": session.max_expires_at,
        "result": result,
    })
}

fn with_last_safe_snapshot(
    session: &LiveSession,
    mut error: BrowserSessionError,
) -> BrowserSessionError {
    if let Some(object) = error.details.as_object_mut() {
        if let Ok(snapshot) = session.last_safe_snapshot.lock() {
            object.insert(
                "last_safe_snapshot".to_string(),
                snapshot.clone().unwrap_or(Value::Null),
            );
        }
    }
    error
}

fn bridge_io_error(error: std::io::Error) -> BrowserSessionError {
    BrowserSessionError::new(
        "BROWSER_SESSION_LOST",
        true,
        json!({"provider_error_kind": error.kind().to_string()}),
    )
}

fn bridge_error_is_fatal(code: &str) -> bool {
    matches!(
        code,
        "BROWSER_SESSION_LOST"
            | "BROWSER_SESSION_CANCELLED"
            | "BROWSER_ACTION_TIMEOUT"
            | "BROWSER_BRIDGE_FAILED"
            | "BROWSER_BRIDGE_PROTOCOL_ERROR"
    )
}

async fn shutdown_session(inner: &Arc<ServiceInner>, session_id: &str) {
    if let Some(session) = inner.sessions.lock().await.remove(session_id) {
        kill_bridge(&session).await;
    }
}

async fn kill_bridge(session: &LiveSession) {
    let mut bridge = session.bridge.lock().await;
    let _ = crate::skills::terminate_subprocess_group(bridge.child.id()).await;
    let _ = bridge.child.kill().await;
    let _ = bridge.child.wait().await;
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn detect_session_capacity() -> usize {
    let cpu_capacity = std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1)
        .div_ceil(2)
        .clamp(1, 4);
    #[cfg(target_os = "linux")]
    {
        let available_kib = std::fs::read_to_string("/proc/meminfo")
            .ok()
            .and_then(|content| {
                content.lines().find_map(|line| {
                    line.strip_prefix("MemAvailable:")?
                        .split_whitespace()
                        .next()?
                        .parse::<u64>()
                        .ok()
                })
            })
            .unwrap_or(u64::MAX);
        if available_kib < 1_572_864 {
            return 1;
        }
    }
    cpu_capacity
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_binding(actor: &str, task: &str) -> BrowserSessionBinding {
        BrowserSessionBinding {
            actor_ref: actor.to_string(),
            task_id: task.to_string(),
            registry_generation: 7,
            registry_digest: "registry-a".to_string(),
            policy_digest: "policy-a".to_string(),
        }
    }

    #[test]
    fn binding_separates_actor_task_registry_and_policy() {
        let base = fixture_binding("actor-a", "task-a");
        let mut changed = base.clone();
        changed.actor_ref = "actor-b".to_string();
        assert_ne!(base, changed);
        changed = base.clone();
        changed.task_id = "task-b".to_string();
        assert_ne!(base, changed);
        changed = base.clone();
        changed.registry_generation = 8;
        assert_ne!(base, changed);
        changed = base.clone();
        changed.policy_digest = "policy-b".to_string();
        assert_ne!(base, changed);
    }

    #[test]
    fn error_contract_uses_stable_machine_fields() {
        let error = BrowserSessionError::new("BROWSER_SESSION_LOST", true, json!({}));
        assert_eq!(error.message_key, "browser_session.browser_session_lost");
        assert!(error.retryable);
    }

    #[test]
    fn detected_session_capacity_is_bounded() {
        assert!((1..=4).contains(&detect_session_capacity()));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn live_service_enforces_binding_and_recovers_from_bridge_loss() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .expect("workspace");
        let service = BrowserSessionService::new(&workspace);
        let binding = fixture_binding("actor-a", "task-live-browser-session");
        let opened = service
            .open(binding.clone(), json!({"locale": "en-US"}), None)
            .await
            .expect("browser_fixture_open");
        let session_id = opened["session_id"].as_str().expect("session id");
        let page_id = opened["result"]["page_id"].as_str().expect("page id");
        let page_generation = opened["result"]["page_generation"]
            .as_u64()
            .expect("page generation");
        assert!(opened["result"].get("snapshot").is_none());
        let session = service
            .inner
            .sessions
            .lock()
            .await
            .get(session_id)
            .cloned()
            .expect("live session");
        assert!(session
            .last_safe_snapshot
            .lock()
            .expect("snapshot lock")
            .is_some());
        drop(session);

        let mut other_actor = binding.clone();
        other_actor.actor_ref = "actor-b".to_string();
        let mismatch = service
            .request(
                session_id,
                &other_actor,
                "snapshot",
                json!({"page_id": page_id, "expected_page_generation": page_generation}),
                None,
            )
            .await
            .expect_err("browser_cross_actor");
        assert_eq!(mismatch.code, "BROWSER_SESSION_BINDING_MISMATCH");

        let snapshot = service
            .request(
                session_id,
                &binding,
                "snapshot",
                json!({"page_id": page_id, "expected_page_generation": page_generation}),
                None,
            )
            .await
            .expect("bound actor snapshot");
        assert_eq!(snapshot["session_id"], session_id);
        assert_eq!(snapshot["result"]["page_id"], page_id);

        let session = service
            .inner
            .sessions
            .lock()
            .await
            .get(session_id)
            .cloned()
            .expect("live session");
        kill_bridge(&session).await;
        drop(session);
        let lost = service
            .request(
                session_id,
                &binding,
                "snapshot",
                json!({"page_id": page_id, "expected_page_generation": page_generation}),
                None,
            )
            .await
            .expect_err("browser_bridge_dead");
        assert_eq!(lost.code, "BROWSER_SESSION_LOST");
        assert!(lost.details.get("last_safe_snapshot").is_some());
        assert!(!service.inner.sessions.lock().await.contains_key(session_id));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn expired_and_restarted_sessions_fail_closed() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .expect("workspace");
        let service = BrowserSessionService::new(&workspace);
        let binding = fixture_binding("actor-expired", "task-expired-browser-session");
        let opened = service
            .open(binding.clone(), json!({}), None)
            .await
            .expect("browser_fixture_open");
        let session_id = opened["session_id"].as_str().expect("session id");
        let page_id = opened["result"]["page_id"].as_str().expect("page id");
        let page_generation = opened["result"]["page_generation"]
            .as_u64()
            .expect("page generation");
        let session = service
            .inner
            .sessions
            .lock()
            .await
            .get(session_id)
            .cloned()
            .expect("live session");
        *session.lease_expires_at.lock().expect("lease lock") = unix_now();
        drop(session);
        let expired = service
            .request(
                session_id,
                &binding,
                "snapshot",
                json!({"page_id": page_id, "expected_page_generation": page_generation}),
                None,
            )
            .await
            .expect_err("browser_expired");
        assert_eq!(expired.code, "BROWSER_SESSION_EXPIRED");
        assert!(!service.inner.sessions.lock().await.contains_key(session_id));

        let restarted = BrowserSessionService::new(&workspace);
        let lost = restarted
            .request(
                session_id,
                &binding,
                "snapshot",
                json!({"page_id": page_id, "expected_page_generation": page_generation}),
                None,
            )
            .await
            .expect_err("browser_restart_lost");
        assert_eq!(lost.code, "BROWSER_SESSION_LOST");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn task_cleanup_releases_only_the_matching_session_lease() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .expect("workspace");
        let service = BrowserSessionService::new(&workspace);
        let first_binding = fixture_binding("actor-cleanup", "task-cleanup-first");
        let second_binding = fixture_binding("actor-cleanup", "task-cleanup-second");
        let first = service
            .open(first_binding.clone(), json!({}), None)
            .await
            .expect("first browser session");
        let second = service
            .open(second_binding.clone(), json!({}), None)
            .await
            .expect("second browser session");
        let first_id = first["session_id"].as_str().expect("first id");
        let second_id = second["session_id"].as_str().expect("second id");

        assert_eq!(service.close_task_sessions("task-cleanup-first").await, 1);
        let first_lost = service
            .request(first_id, &first_binding, "observe_debug", json!({}), None)
            .await
            .expect_err("cleaned session must be gone");
        assert_eq!(first_lost.code, "BROWSER_SESSION_LOST");
        service
            .request(second_id, &second_binding, "observe_debug", json!({}), None)
            .await
            .expect("unrelated session remains available");
        assert_eq!(service.close_task_sessions("task-cleanup-second").await, 1);
    }

    #[cfg(target_os = "linux")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn dropping_host_service_reaps_live_bridge_process() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .expect("workspace");
        let service = BrowserSessionService::new(&workspace);
        let binding = fixture_binding("actor-drop", "task-drop-browser-session");
        let opened = service
            .open(binding, json!({}), None)
            .await
            .expect("browser_fixture_open");
        let session_id = opened["session_id"].as_str().expect("session id");
        let process_id = {
            let session = service
                .inner
                .sessions
                .lock()
                .await
                .get(session_id)
                .cloned()
                .expect("live session");
            let process_id = session.bridge.lock().await.child.id().expect("child pid");
            process_id
        };
        assert!(Path::new(&format!("/proc/{process_id}")).exists());
        assert!(!linux_process_group_members(process_id).is_empty());
        let descendants = linux_process_descendants(process_id);
        assert!(!descendants.is_empty(), "browser_child_missing");
        drop(service);
        for _ in 0..20 {
            if linux_process_group_members(process_id).is_empty()
                && descendants
                    .iter()
                    .all(|pid| !Path::new(&format!("/proc/{pid}")).exists())
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("browser_bridge_survived:{process_id}");
    }

    #[cfg(target_os = "linux")]
    fn linux_process_group_members(group_id: u32) -> Vec<u32> {
        linux_process_table()
            .into_iter()
            .filter_map(|(process_id, _, process_group)| {
                (process_group == group_id).then_some(process_id)
            })
            .collect()
    }

    #[cfg(target_os = "linux")]
    fn linux_process_descendants(parent_id: u32) -> Vec<u32> {
        let table = linux_process_table();
        let mut parents = vec![parent_id];
        let mut descendants = Vec::new();
        while let Some(parent) = parents.pop() {
            for (process_id, candidate_parent, _) in &table {
                if *candidate_parent == parent && !descendants.contains(process_id) {
                    descendants.push(*process_id);
                    parents.push(*process_id);
                }
            }
        }
        descendants
    }

    #[cfg(target_os = "linux")]
    fn linux_process_table() -> Vec<(u32, u32, u32)> {
        let Ok(entries) = std::fs::read_dir("/proc") else {
            return Vec::new();
        };
        entries
            .flatten()
            .filter_map(|entry| entry.file_name().to_string_lossy().parse::<u32>().ok())
            .filter_map(|process_id| {
                let Ok(stat) = std::fs::read_to_string(format!("/proc/{process_id}/stat")) else {
                    return None;
                };
                let Some(after_name) = stat.rsplit_once(')').map(|(_, suffix)| suffix.trim())
                else {
                    return None;
                };
                let fields = after_name.split_whitespace().collect::<Vec<_>>();
                Some((
                    process_id,
                    fields.get(1)?.parse::<u32>().ok()?,
                    fields.get(2)?.parse::<u32>().ok()?,
                ))
            })
            .collect()
    }
}
