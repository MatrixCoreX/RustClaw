use super::{
    direct::DirectSession,
    node_selection::{self, CheckedNode},
    nodes::{Node, Nodes},
    owner_transport::OwnerTransport,
    protocol::Capabilities,
};
use crate::{
    commands::{main_only, DesktopState},
    session::Session,
    transport::WireResponse,
    wallet::commands::WalletState,
    Result,
};
use http::Method;
use serde::Serialize;
use serde_json::Value;
use std::{path::PathBuf, sync::Arc};
use tauri::{State, WebviewWindow};
use tokio::sync::Mutex;
use uuid::Uuid;

pub struct StandaloneState {
    pub(crate) nodes: Mutex<Nodes>,
    transition: Mutex<()>,
}
impl StandaloneState {
    pub fn new(path: PathBuf) -> Result<Self> {
        Ok(Self {
            nodes: Mutex::new(Nodes::new(path)?),
            transition: Mutex::new(()),
        })
    }
}
#[derive(Serialize)]
pub struct ConnectionInfo {
    id: Uuid,
    node: Node,
    assets: Capabilities,
    bancor: Capabilities,
    response_ms: u64,
    automatic: bool,
}

pub enum Connection {
    Device(Arc<Session>),
    Direct(Arc<DirectSession>),
}
impl Connection {
    pub fn id(&self) -> Uuid {
        match self {
            Self::Device(s) => s.id,
            Self::Direct(s) => s.id,
        }
    }
    pub fn profile_id(&self) -> Uuid {
        match self {
            Self::Device(s) => s.profile.id,
            Self::Direct(s) => s.node.id,
        }
    }
    pub async fn label(&self) -> (String, String) {
        match self {
            Self::Device(s) => {
                let info = s.info().await;
                (info.profile.alias, info.origin)
            }
            Self::Direct(s) => (s.node.origin.clone(), s.node.origin.clone()),
        }
    }
}
impl OwnerTransport for Connection {
    async fn owner_request(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<WireResponse> {
        match self {
            Self::Device(s) => s.owner_request(method, path, body).await,
            Self::Direct(s) => s.owner_request(method, path, body).await,
        }
    }
    fn cancelled(&self) -> bool {
        match self {
            Self::Device(s) => s.cancelled.is_cancelled(),
            Self::Direct(s) => s.cancelled.is_cancelled(),
        }
    }
    fn expected_node(&self) -> Option<(&str, Option<&str>)> {
        match self {
            Self::Device(_) => None,
            Self::Direct(s) => s.expected_node(),
        }
    }
}
pub async fn resolve(
    desktop: &DesktopState,
    standalone: &StandaloneState,
    id: Uuid,
) -> Result<Connection> {
    if let Some(session) = standalone
        .nodes
        .lock()
        .await
        .current
        .as_ref()
        .filter(|s| s.id == id && !s.cancelled.is_cancelled())
        .cloned()
    {
        return Ok(Connection::Direct(session));
    }
    Ok(Connection::Device(desktop.session(id).await?))
}

#[tauri::command]
pub async fn wallet_nodes(
    window: WebviewWindow,
    state: State<'_, StandaloneState>,
) -> Result<Value> {
    main_only(&window)?;
    serde_json::to_value(&state.nodes.lock().await.document)
        .map_err(|_| "wallet_node_config_invalid".into())
}
#[tauri::command]
pub async fn wallet_add_node(
    window: WebviewWindow,
    state: State<'_, StandaloneState>,
    origin: String,
) -> Result<Node> {
    main_only(&window)?;
    let _gate = state.transition.lock().await;
    state.nodes.lock().await.add(&origin)
}
#[tauri::command]
pub async fn wallet_connect_node(
    window: WebviewWindow,
    state: State<'_, StandaloneState>,
    wallet: State<'_, WalletState>,
    node_id: Uuid,
) -> Result<ConnectionInfo> {
    main_only(&window)?;
    let _transition = state.transition.lock().await;
    let _gate = wallet.operation_gate.lock().await;
    let node = state
        .nodes
        .lock()
        .await
        .document
        .nodes
        .iter()
        .find(|n| n.id == node_id)
        .cloned()
        .ok_or("wallet_node_missing")?;
    let checked = node_selection::check(node).await?;
    activate(&state, &wallet, checked, false).await
}

#[tauri::command]
pub async fn wallet_prefer_node(
    window: WebviewWindow,
    state: State<'_, StandaloneState>,
    wallet: State<'_, WalletState>,
) -> Result<ConnectionInfo> {
    main_only(&window)?;
    let _transition = state.transition.lock().await;
    // Serialize selection with prepare/confirm. A pending or unresolved write must
    // remain bound to its original node until the user resolves it.
    let _gate = wallet.operation_gate.lock().await;
    {
        let operations = wallet.operations.lock().await;
        if operations.pending.is_some() {
            return Err("wallet_confirmation_pending".into());
        }
        if operations.records.iter().any(|r| r.status == "pending") {
            return Err("wallet_unresolved_operation".into());
        }
    }
    let (nodes, selected, ledger) = {
        let nodes = state.nodes.lock().await;
        (
            nodes.document.nodes.clone(),
            nodes.document.selected,
            nodes.document.ledger_id.clone(),
        )
    };
    let checked = node_selection::prefer(nodes, selected, ledger.as_deref()).await?;
    activate(&state, &wallet, checked, true).await
}

async fn activate(
    state: &StandaloneState,
    wallet: &WalletState,
    checked: CheckedNode,
    automatic: bool,
) -> Result<ConnectionInfo> {
    let CheckedNode {
        session,
        assets,
        bancor,
        response_ms,
    } = checked;
    let node = session.node.clone();
    let session = Arc::new(session);
    let mut nodes = state.nodes.lock().await;
    let previous = nodes.document.selected;
    let previous_ledger = nodes.document.ledger_id.clone();
    nodes.document.selected = node.id;
    nodes.document.ledger_id = Some(assets.ledger_id.clone());
    if let Err(e) = nodes.persist() {
        nodes.document.selected = previous;
        nodes.document.ledger_id = previous_ledger;
        return Err(e);
    }
    if automatic {
        if let Some(current) = &nodes.current {
            if current.node.id == node.id
                && current.ledger == session.ledger
                && !current.cancelled.is_cancelled()
            {
                return Ok(ConnectionInfo {
                    id: current.id,
                    node,
                    assets,
                    bancor,
                    response_ms,
                    automatic,
                });
            }
        }
    }
    wallet.lock().await;
    nodes.disconnect();
    nodes.current = Some(session.clone());
    Ok(ConnectionInfo {
        id: session.id,
        node,
        assets,
        bancor,
        response_ms,
        automatic,
    })
}
#[tauri::command]
pub async fn wallet_disconnect_node(
    window: WebviewWindow,
    state: State<'_, StandaloneState>,
    wallet: State<'_, WalletState>,
) -> Result<()> {
    main_only(&window)?;
    let _transition = state.transition.lock().await;
    let _gate = wallet.operation_gate.lock().await;
    state.nodes.lock().await.disconnect();
    wallet.lock().await;
    Ok(())
}
#[tauri::command]
pub async fn wallet_market_read(
    window: WebviewWindow,
    state: State<'_, StandaloneState>,
    session_id: Uuid,
    path: String,
) -> Result<Value> {
    main_only(&window)?;
    let session = state
        .nodes
        .lock()
        .await
        .current
        .as_ref()
        .filter(|s| s.id == session_id)
        .cloned()
        .ok_or("stale_connection")?;
    session.market(&path).await
}
