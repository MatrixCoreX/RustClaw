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

use claw_core::host_resources::{HostResourceSnapshot, ResourcePressureState};

const PROTOCOL_VERSION: u64 = 1;
const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const DEFAULT_IDLE_LEASE: Duration = Duration::from_secs(300);
const DEFAULT_MAX_LIFETIME: Duration = Duration::from_secs(1800);
const LOW_MEMORY_SCREENSHOT_HOST_MAX_MIB: u64 = 4 * 1024;
const LOW_MEMORY_SCREENSHOT_MAX_TILE_PIXELS: u64 = 2 * 1024 * 1024;
const LOW_MEMORY_SCREENSHOT_MAX_TILES: u64 = 64;

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
    _lifetime: Arc<ServiceLifetime>,
    inner: Arc<ServiceInner>,
}

struct ServiceLifetime {
    inner: std::sync::Weak<ServiceInner>,
    cancelled: CancellationToken,
}

impl Drop for ServiceLifetime {
    fn drop(&mut self) {
        self.cancelled.cancel();
        if let (Some(inner), Ok(runtime)) =
            (self.inner.upgrade(), tokio::runtime::Handle::try_current())
        {
            runtime.spawn(async move {
                let sessions = std::mem::take(&mut *inner.sessions.lock().await);
                for (_, session) in sessions {
                    gracefully_stop_bridge(&session).await;
                }
            });
        }
        // Without an executor, BridgeProcess::drop still kills the process group.
    }
}

struct ServiceInner {
    workspace_root: PathBuf,
    bridge_path: PathBuf,
    sessions: Mutex<HashMap<String, Arc<LiveSession>>>,
    session_slots: Arc<Semaphore>,
    idle_lease: Duration,
    max_lifetime: Duration,
    resource_broker: crate::resource_scheduler::ResourceBroker,
}

struct LiveSession {
    id: String,
    binding: BrowserSessionBinding,
    created_at: u64,
    max_expires_at: u64,
    last_used_at: StdMutex<u64>,
    heartbeat_at: StdMutex<u64>,
    lease_expires_at: StdMutex<u64>,
    idle_lease_seconds: u64,
    last_safe_snapshot: StdMutex<Option<Value>>,
    bridge: Mutex<BridgeProcess>,
    _session_slot: OwnedSemaphorePermit,
    resource_lease: crate::resource_scheduler::ResourceLease,
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
    #[cfg(test)]
    pub(crate) fn new(workspace_root: &Path) -> Self {
        Self::new_with_resource_broker(
            workspace_root,
            crate::resource_scheduler::ResourceBroker::default(),
        )
    }

    pub(crate) fn new_with_resource_broker(
        workspace_root: &Path,
        resource_broker: crate::resource_scheduler::ResourceBroker,
    ) -> Self {
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
            resource_broker,
        });
        let lifetime = Arc::new(ServiceLifetime {
            inner: Arc::downgrade(&inner),
            cancelled: CancellationToken::new(),
        });
        Self::spawn_reaper(&inner, lifetime.cancelled.clone());
        Self {
            inner,
            _lifetime: lifetime,
        }
    }

    fn spawn_reaper(inner: &Arc<ServiceInner>, cancelled: CancellationToken) {
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let weak = Arc::downgrade(inner);
        runtime.spawn(async move {
            tokio::select! {
              _ = cancelled.cancelled() => {},
              _ = async move {
                let mut interval = tokio::time::interval(Duration::from_secs(30));
                interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                loop {
                    interval.tick().await;
                    let Some(inner) = weak.upgrade() else { break };
                    let now = unix_now();
                    let (expired, active) = {
                        let sessions = inner.sessions.lock().await;
                        let mut expired = Vec::new();
                        let mut active = Vec::new();
                        for (id, session) in sessions.iter() {
                            let lease = session.lease_expires_at.lock().map(|v| *v).unwrap_or(0);
                            if now >= session.max_expires_at {
                                expired.push((id.clone(), "max_lifetime"));
                            } else if now >= lease {
                                expired.push((id.clone(), "idle_deadline"));
                            } else {
                                active.push(session.clone());
                            }
                        }
                        (expired, active)
                    };
                    for session in active {
                        refresh_session_resource_reservation(&session).await;
                    }
                    for (session_id, reason) in expired {
                        shutdown_session(&inner, &session_id, reason).await;
                    }
                }
              } => {},
            }
        });
    }

    #[cfg(test)]
    pub(crate) async fn open(
        &self,
        binding: BrowserSessionBinding,
        input: Value,
        cancellation: Option<CancellationToken>,
    ) -> Result<Value, BrowserSessionError> {
        self.open_with_resource_lease(binding, input, cancellation, None, None)
            .await
    }

    pub(crate) async fn open_with_resource_lease(
        &self,
        binding: BrowserSessionBinding,
        mut input: Value,
        cancellation: Option<CancellationToken>,
        resource_lease: Option<crate::resource_scheduler::ResourceLease>,
        resource_request: Option<&claw_core::skill_registry::SkillResourceRequest>,
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
        let fallback_request = claw_core::skill_registry::SkillResourceRequest {
            class: claw_core::skill_registry::SkillResourceClass::Memory,
            cpu_cores: 1,
            memory_mb: 512,
            network_slots: 1,
            browser_slots: 1,
            ..claw_core::skill_registry::SkillResourceRequest::default()
        };
        let resource_lease = match resource_lease {
            Some(resource_lease) => resource_lease,
            None => self
                .inner
                .resource_broker
                .try_acquire(resource_request.or(Some(&fallback_request)), 1)
                .map_err(|grant| {
                    BrowserSessionError::new(
                        "BROWSER_RESOURCE_LIMIT",
                        true,
                        json!({
                            "reason": grant.wait_reason,
                            "resource_grant": grant.projection,
                        }),
                    )
                })?,
        };
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
            heartbeat_at: StdMutex::new(now),
            lease_expires_at: StdMutex::new(now.saturating_add(self.inner.idle_lease.as_secs())),
            idle_lease_seconds: self.inner.idle_lease.as_secs(),
            last_safe_snapshot: StdMutex::new(None),
            bridge: Mutex::new(bridge),
            _session_slot: session_slot,
            resource_lease,
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
        if object
            .get("include_screenshot")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            object.insert(
                "screenshot_contract".to_string(),
                screenshot_contract(&self.inner.resource_broker),
            );
        }
        match call_bridge(&session, input, cancellation).await {
            Ok(result) => {
                refresh_session_resource_reservation(&session).await;
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
                terminate_live_session(&session, "open_failed").await;
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
        if command == "screenshot"
            || input
                .get("include_screenshot")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        {
            input.as_object_mut().expect("object normalized").insert(
                "screenshot_contract".to_string(),
                screenshot_contract(&self.inner.resource_broker),
            );
        }
        match call_bridge(&session, input, cancellation).await {
            Ok(result) => {
                refresh_session_resource_reservation(&session).await;
                Ok(project_result(&session, result))
            }
            Err(error) => {
                let error = with_last_safe_snapshot(&session, error);
                if bridge_error_is_fatal(&error.code) {
                    shutdown_session(&self.inner, session_id, "fatal_bridge_error").await;
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
        terminate_live_session(&session, "user_close").await;
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
            terminate_live_session(&session, "task_terminal").await;
        }
        count
    }

    pub(crate) async fn reclaim_idle(&self, minimum_idle: Duration) -> usize {
        let now = unix_now();
        let minimum_idle_seconds = minimum_idle.as_secs();
        let session_ids = {
            let sessions = self.inner.sessions.lock().await;
            sessions
                .iter()
                .filter_map(|(session_id, session)| {
                    let last_used = session.last_used_at.lock().map(|value| *value).ok()?;
                    (now.saturating_sub(last_used) >= minimum_idle_seconds)
                        .then(|| session_id.clone())
                })
                .collect::<Vec<_>>()
        };
        let count = session_ids.len();
        for session_id in session_ids {
            shutdown_session(&self.inner, &session_id, "resource_pressure_idle_reclaim").await;
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
            let reason = if now >= session.max_expires_at {
                "max_lifetime"
            } else {
                "idle_deadline"
            };
            shutdown_session(&self.inner, session_id, reason).await;
            return Err(BrowserSessionError::new(
                "BROWSER_SESSION_EXPIRED",
                true,
                json!({"session_id": session_id}),
            ));
        }
        touch_session(&session, now);
        Ok(session)
    }
}

fn screenshot_contract(broker: &crate::resource_scheduler::ResourceBroker) -> Value {
    let status = broker.status();
    screenshot_contract_for_resources(
        status.snapshot.effective_memory_mib(),
        status.pressure_state,
    )
}

fn screenshot_contract_for_resources(
    effective_memory_mib: Option<u64>,
    pressure_state: ResourcePressureState,
) -> Value {
    let segmented = effective_memory_mib
        .is_some_and(|memory_mib| memory_mib <= LOW_MEMORY_SCREENSHOT_HOST_MAX_MIB)
        || matches!(
            pressure_state,
            ResourcePressureState::Constrained | ResourcePressureState::Critical
        );
    if segmented {
        json!({
            "schema_version": 1,
            "mode": "segmented_full_page",
            "max_tile_pixels": LOW_MEMORY_SCREENSHOT_MAX_TILE_PIXELS,
            "max_tiles": LOW_MEMORY_SCREENSHOT_MAX_TILES,
            "preserve_full_page": true,
        })
    } else {
        json!({
            "schema_version": 1,
            "mode": "single_full_page",
            "preserve_full_page": true,
        })
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
    touch_session(session, now);
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
    let heartbeat_at = session.heartbeat_at.lock().map(|v| *v).unwrap_or(0);
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
        "heartbeat_at": heartbeat_at,
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

fn touch_session(session: &LiveSession, now: u64) {
    if let Ok(mut value) = session.last_used_at.lock() {
        *value = now;
    }
    if let Ok(mut value) = session.heartbeat_at.lock() {
        *value = now;
    }
    if let Ok(mut value) = session.lease_expires_at.lock() {
        *value = now.saturating_add(session.idle_lease_seconds);
    }
}

async fn shutdown_session(inner: &Arc<ServiceInner>, session_id: &str, exit_reason: &'static str) {
    if let Some(session) = inner.sessions.lock().await.remove(session_id) {
        terminate_live_session(&session, exit_reason).await;
    }
}

async fn terminate_live_session(session: &LiveSession, exit_reason: &'static str) {
    let process_id = session.bridge.lock().await.child.id();
    tracing::info!(
        session_id = session.id,
        task_id = session.binding.task_id,
        actor_ref = session.binding.actor_ref,
        process_id,
        exit_reason,
        "browser_session_terminated"
    );
    kill_bridge(session).await;
}

async fn refresh_session_resource_reservation(session: &LiveSession) {
    let process_id = session.bridge.lock().await.child.id();
    if let Some(memory_mib) =
        process_id.and_then(crate::skills::runner_pool::process_tree_resident_memory_mib)
    {
        session
            .resource_lease
            .refresh_materialized_memory(memory_mib);
    }
}

async fn kill_bridge(session: &LiveSession) {
    let mut bridge = session.bridge.lock().await;
    let _ = crate::skills::terminate_subprocess_group(bridge.child.id()).await;
    let _ = bridge.child.kill().await;
    let _ = bridge.child.wait().await;
}

async fn gracefully_stop_bridge(session: &LiveSession) {
    let mut bridge = session.bridge.lock().await;
    #[cfg(unix)]
    if let Some(pid) = bridge.child.id() {
        // Let the bridge close Chromium and reap its children before exiting.
        // The group-wide hard kill remains the bounded fallback.
        unsafe {
            libc::kill(pid as i32, libc::SIGTERM);
        }
        if tokio::time::timeout(Duration::from_secs(5), bridge.child.wait())
            .await
            .is_ok_and(|result| result.is_ok())
        {
            return;
        }
    }
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
    let snapshot = HostResourceSnapshot::collect();
    let cpu_capacity = snapshot.cpu_parallelism.div_ceil(2).clamp(1, 4);
    if snapshot
        .available_memory_mib()
        .is_some_and(|available| available < 1536)
    {
        return 1;
    }
    cpu_capacity
}

#[cfg(test)]
#[path = "browser_session_service_tests.rs"]
mod tests;
