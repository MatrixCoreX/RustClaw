use super::{
    client,
    protocol::*,
    standalone::{self, StandaloneState},
    Pending, Record,
};
use crate::{
    commands::{main_only, DesktopState},
    wallet::commands::{open_window, wallet_only, WalletState},
    Result,
};
use http::Method;
use serde::Serialize;
use tauri::{State, WebviewWindow};
use uuid::Uuid;
use zeroize::Zeroizing;

#[derive(Serialize)]
pub struct Confirmation {
    pub account_name: String,
    pub public_key: String,
    pub device_label: String,
    pub origin: String,
    pub payload: Payload,
}
#[tauri::command]
pub async fn wallet_pending(
    window: WebviewWindow,
    state: State<'_, WalletState>,
) -> Result<Option<Confirmation>> {
    wallet_only(&window)?;
    let mut ops = state.operations.lock().await;
    if ops
        .pending
        .as_ref()
        .is_some_and(|p| p.payload.expires_at_unix <= client::now())
    {
        ops.pending = None;
    }
    Ok(ops.pending.as_ref().map(|p| Confirmation {
        account_name: p.account.name.clone(),
        public_key: p.account.public_key.clone(),
        device_label: p.device_label.clone(),
        origin: p.origin.clone(),
        payload: p.payload.clone(),
    }))
}
#[tauri::command]
pub async fn wallet_cancel_operation(
    window: WebviewWindow,
    state: State<'_, WalletState>,
) -> Result<()> {
    wallet_only(&window)?;
    state.operations.lock().await.pending = None;
    Ok(())
}
#[tauri::command]
pub async fn wallet_capabilities(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    standalone: State<'_, StandaloneState>,
    session_id: Uuid,
    service: Service,
) -> Result<Capabilities> {
    main_only(&window)?;
    let session = standalone::resolve(&state, &standalone, session_id).await?;
    client::capabilities(&session, service, "balances").await
}
#[tauri::command]
pub async fn wallet_read(
    window: WebviewWindow,
    desktop: State<'_, DesktopState>,
    standalone: State<'_, StandaloneState>,
    state: State<'_, WalletState>,
    session_id: Uuid,
    account_id: Uuid,
    service: Service,
    page: Option<u32>,
) -> Result<ReadResult> {
    main_only(&window)?;
    let _gate = state.operation_gate.lock().await;
    let generation = state.selection.lock().unwrap().generation;
    state.check(account_id, generation)?;
    let account = state.vault.lock().unwrap().account(account_id)?;
    let intent = page
        .map(|page| Intent::History { page })
        .unwrap_or(Intent::Balances);
    let session = standalone::resolve(&desktop, &standalone, session_id).await?;
    let cap = client::capabilities(&session, service, intent.action()).await?;
    let result: ReadResult =
        client::public_read(&session, &cap, &account.public_key, &intent).await?;
    state.check(account_id, generation)?;
    result.validate_public(&cap, &account.public_key, page.unwrap_or(1))?;
    Ok(result)
}
#[tauri::command]
pub async fn wallet_prepare(
    window: WebviewWindow,
    app: tauri::AppHandle,
    desktop: State<'_, DesktopState>,
    standalone: State<'_, StandaloneState>,
    state: State<'_, WalletState>,
    session_id: Uuid,
    account_id: Uuid,
    service: Service,
    intent: Intent,
) -> Result<Uuid> {
    main_only(&window)?;
    if !intent.write() {
        return Err("wallet_intent_invalid".into());
    }
    let _gate = state.operation_gate.lock().await;
    let generation = state.selection.lock().unwrap().generation;
    state.check(account_id, generation)?;
    if state.operations.lock().await.pending.is_some() {
        return Err("wallet_confirmation_pending".into());
    }
    let account = state.vault.lock().unwrap().account(account_id)?;
    if !account.backed_up {
        return Err("wallet_backup_required".into());
    }
    intent.validate(service, &account.public_key)?;
    let session = standalone::resolve(&desktop, &standalone, session_id).await?;
    let cap = client::capabilities(&session, service, intent.action()).await?;
    let id = Uuid::new_v4();
    if state.operations.lock().await.records.iter().any(|r| {
        r.profile_id == session.profile_id()
            && r.account_id == account_id
            && r.ledger_id == cap.ledger_id
            && r.status == "pending"
    }) {
        return Err("wallet_unresolved_operation".into());
    }
    let (payload, bytes) =
        client::challenge(&session, &cap, &account.public_key, id, &intent).await?;
    state.check(account_id, generation)?;
    let (device_label, origin) = session.label().await;
    state.operations.lock().await.pending = Some(Pending {
        generation,
        session_id,
        profile_id: session.profile_id(),
        account,
        cap,
        intent,
        payload,
        bytes,
        device_label,
        origin,
    });
    if let Err(e) = open_window(&app) {
        state.operations.lock().await.pending = None;
        return Err(e);
    }
    Ok(id)
}
#[tauri::command]
pub async fn wallet_confirm(
    window: WebviewWindow,
    desktop: State<'_, DesktopState>,
    standalone: State<'_, StandaloneState>,
    state: State<'_, WalletState>,
    operation_id: Uuid,
    password: String,
) -> Result<Outcome> {
    let password = Zeroizing::new(password);
    wallet_only(&window)?;
    let _gate = state.operation_gate.lock().await;
    let p = state
        .operations
        .lock()
        .await
        .pending
        .as_ref()
        .cloned()
        .ok_or("wallet_confirmation_missing")?;
    if p.payload.operation_id != operation_id {
        return Err("wallet_confirmation_missing".into());
    }
    let generation = p.generation;
    state.check(p.account.id, generation)?;
    let session = standalone::resolve(&desktop, &standalone, p.session_id).await?;
    let cap = client::capabilities(&session, p.cap.service, p.intent.action()).await?;
    validate_challenge(
        &p.bytes,
        &cap,
        &p.account.public_key,
        operation_id,
        &p.intent,
        client::now(),
    )?;
    state.check(p.account.id, generation)?;
    let vault = state.vault.clone();
    let id = p.account.id;
    let bytes = p.bytes.clone();
    let worker_cap = cap.clone();
    let worker_intent = p.intent.clone();
    let signature = Zeroizing::new(
        tokio::task::spawn_blocking(move || {
            vault.lock().unwrap().sign_with_password(
                id,
                bytes.as_bytes(),
                &password,
                worker_cap,
                worker_intent,
            )
        })
        .await
        .map_err(|_| "wallet_storage_unavailable")??,
    );
    state.check(p.account.id, generation)?;
    let mut ops = state.operations.lock().await;
    if ops
        .pending
        .as_ref()
        .is_none_or(|pending| pending.payload.operation_id != operation_id)
    {
        return Err("wallet_confirmation_missing".into());
    }
    ops.pending = None;
    drop(ops);
    state.operations.lock().await.mark_submitted(&p)?;
    state.check(p.account.id, generation)?;
    let result = client::request::<Outcome>(
        &session,
        Method::POST,
        &format!("{API}/operations/verify"),
        Some(client::verify_body(&p.payload, &signature)),
    )
    .await;
    let result = result.map_err(|error| client::submission_error(&error))?;
    result
        .validate(operation_id, &p.account.public_key, &p.cap.ledger_id)
        .map_err(|_| "wallet_outcome_unknown")?;
    let mut ops = state.operations.lock().await;
    if let Some(record) = ops
        .records
        .iter_mut()
        .find(|r| r.operation_id == operation_id)
    {
        record.status = result.status.clone();
        record.receipt_id = result.receipt_id.clone();
    }
    ops.persist().map_err(|_| "wallet_outcome_unknown")?;
    Ok(result)
}
#[tauri::command]
pub async fn wallet_operations(
    window: WebviewWindow,
    desktop: State<'_, DesktopState>,
    standalone: State<'_, StandaloneState>,
    state: State<'_, WalletState>,
    session_id: Uuid,
    account_id: Uuid,
) -> Result<Vec<Record>> {
    main_only(&window)?;
    let session = standalone::resolve(&desktop, &standalone, session_id).await?;
    let selected = state.selection.lock().unwrap().account_id;
    if selected != Some(account_id) {
        return Err("wallet_selection_changed".into());
    }
    Ok(state
        .operations
        .lock()
        .await
        .records
        .iter()
        .filter(|r| r.profile_id == session.profile_id() && r.account_id == account_id)
        .cloned()
        .collect())
}
#[tauri::command]
pub async fn wallet_check_operation(
    window: WebviewWindow,
    desktop: State<'_, DesktopState>,
    standalone: State<'_, StandaloneState>,
    state: State<'_, WalletState>,
    session_id: Uuid,
    account_id: Uuid,
    operation_id: Uuid,
) -> Result<Outcome> {
    main_only(&window)?;
    let _gate = state.operation_gate.lock().await;
    let generation = state.selection.lock().unwrap().generation;
    state.check(account_id, generation)?;
    let session = standalone::resolve(&desktop, &standalone, session_id).await?;
    let record = state
        .operations
        .lock()
        .await
        .records
        .iter()
        .find(|r| {
            r.profile_id == session.profile_id()
                && r.account_id == account_id
                && r.operation_id == operation_id
        })
        .cloned()
        .ok_or("wallet_operation_missing")?;
    let cap = client::capabilities(&session, record.service, "operation_status").await?;
    if cap.ledger_id != record.ledger_id || cap.node_url != record.node_url {
        return Err("wallet_node_changed".into());
    }
    let intent = Intent::OperationStatus { operation_id };
    let result: Outcome = client::public_read(&session, &cap, &record.public_key, &intent).await?;
    state.check(account_id, generation)?;
    result.validate(operation_id, &record.public_key, &record.ledger_id)?;
    let mut ops = state.operations.lock().await;
    if let Some(record) = ops
        .records
        .iter_mut()
        .find(|r| r.operation_id == operation_id)
    {
        record.status = result.status.clone();
        record.receipt_id = result.receipt_id.clone();
    }
    ops.persist()?;
    Ok(result)
}
