use super::*;

pub(super) async fn run_blocking<T: Send + 'static>(
    stage: &'static str,
    work: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
) -> anyhow::Result<T> {
    // Await each operation; an already-started SQL call may finish after its
    // waiter is dropped. These closures never perform network sends or retries.
    let queued_at = std::time::Instant::now();
    let (result, queue_ms, execution_ms) = tokio::task::spawn_blocking(move || {
        let started_at = std::time::Instant::now();
        let queue_ms = queued_at.elapsed().as_millis();
        let result = work();
        (result, queue_ms, started_at.elapsed().as_millis())
    })
    .await
    .map_err(|error| {
        anyhow!("channel_delivery_persistence_join_failed stage={stage} error={error}")
    })?;
    tracing::debug!(
        stage,
        queue_ms,
        execution_ms,
        success = result.is_ok(),
        "channel_delivery_persistence_completed"
    );
    result
}

pub(super) async fn claim(
    state: &AppState,
    envelope: &ChannelDeliveryEnvelope,
) -> anyhow::Result<ClaimChannelDeliveryDispatchOutcome> {
    let pool = state.core.db.clone();
    let envelope = envelope.clone();
    run_blocking("dispatch_claim", move || {
        crate::repo::claim_channel_delivery_dispatch(
            &pool,
            &envelope,
            DELIVERY_DISPATCH_LEASE_SECONDS,
        )
    })
    .await
}

pub(super) async fn load_receipt(
    state: &AppState,
    idempotency_key: &str,
) -> anyhow::Result<Option<ChannelDeliveryReceipt>> {
    let pool = state.core.db.clone();
    let key = idempotency_key.to_string();
    run_blocking("receipt_read", move || {
        let db = pool
            .get()
            .map_err(|_| anyhow!("channel_delivery_receipt_db_failed"))?;
        crate::repo::channel_delivery_receipt::load_channel_delivery_receipt_from_db(&db, &key)
            .map_err(|_| anyhow!("channel_delivery_receipt_load_failed"))
    })
    .await
}

pub(super) async fn record_receipt(
    state: &AppState,
    receipt: &ChannelDeliveryReceipt,
) -> anyhow::Result<crate::repo::channel_delivery_receipt::RecordChannelDeliveryReceiptOutcome> {
    let pool = state.core.db.clone();
    let receipt = receipt.clone();
    run_blocking("receipt_record", move || {
        crate::repo::record_channel_delivery_receipt(&pool, &receipt)
    })
    .await
}

pub(super) async fn complete(
    state: &AppState,
    idempotency_key: &str,
    lease_token: &str,
) -> anyhow::Result<()> {
    let pool = state.core.db.clone();
    let key = idempotency_key.to_string();
    let token = lease_token.to_string();
    run_blocking("dispatch_complete", move || {
        crate::repo::complete_channel_delivery_dispatch(&pool, &key, &token)
    })
    .await
}

pub(super) async fn renew(
    state: &AppState,
    idempotency_key: &str,
    lease_token: &str,
) -> anyhow::Result<bool> {
    let pool = state.core.db.clone();
    let key = idempotency_key.to_string();
    let token = lease_token.to_string();
    run_blocking("dispatch_renew", move || {
        crate::repo::renew_channel_delivery_dispatch(
            &pool,
            &key,
            &token,
            DELIVERY_DISPATCH_LEASE_SECONDS,
        )
    })
    .await
}
