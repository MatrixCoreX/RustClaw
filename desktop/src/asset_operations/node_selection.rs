//! Read-only, bounded node checks. No account, balance request or signature is sent.
use super::{
    client,
    direct::DirectSession,
    nodes::Node,
    protocol::{Capabilities, Service},
};
use crate::Result;
use futures_util::{stream, StreamExt};
use std::time::{Duration, Instant};
use uuid::Uuid;

pub struct CheckedNode {
    pub session: DirectSession,
    pub assets: Capabilities,
    pub bancor: Capabilities,
    pub response_ms: u64,
}

pub async fn check(node: Node) -> Result<CheckedNode> {
    let started = Instant::now();
    let mut session = DirectSession::new(node)?;
    let (assets, bancor) = tokio::try_join!(
        client::capabilities(&session, Service::Assets, "balances"),
        client::capabilities(&session, Service::Bancor, "balances")
    )?;
    if assets.ledger_id != bancor.ledger_id {
        return Err("wallet_node_changed".into());
    }
    session.ledger = Some(assets.ledger_id.clone());
    Ok(CheckedNode {
        session,
        assets,
        bancor,
        response_ms: started.elapsed().as_millis().max(1) as u64,
    })
}

/// Measure full capability responses over the verified transport, not ping or TCP alone.
/// At most eight nodes run together, with a six-second budget for the whole selection.
pub async fn prefer(nodes: Vec<Node>, selected: Uuid, ledger: Option<&str>) -> Result<CheckedNode> {
    let checks = stream::iter(nodes.into_iter().map(|node| async move {
        tokio::time::timeout(Duration::from_secs(4), check(node))
            .await
            .ok()
            .and_then(Result::ok)
    }))
    .buffer_unordered(8);
    tokio::pin!(checks);
    let deadline = tokio::time::sleep(Duration::from_secs(6));
    tokio::pin!(deadline);
    let mut available = Vec::new();
    loop {
        tokio::select! {
            biased;
            _ = &mut deadline => break,
            next = checks.next() => match next {
                Some(Some(node)) => available.push(node),
                Some(None) => {},
                None => break,
            }
        }
    }
    choose(available, selected, ledger)
}

fn choose(
    mut available: Vec<CheckedNode>,
    selected: Uuid,
    ledger: Option<&str>,
) -> Result<CheckedNode> {
    // Older installations have no ledger pin yet. Anchor to their selected node;
    // if it is unavailable, do not guess between different responding ledgers.
    let anchor = ledger.map(str::to_owned).or_else(|| {
        available
            .iter()
            .find(|n| n.session.node.id == selected)
            .map(|n| n.assets.ledger_id.clone())
    });
    let anchor = match anchor {
        Some(ledger) => ledger,
        None => {
            let first = available
                .first()
                .ok_or("wallet_node_no_healthy")?
                .assets
                .ledger_id
                .clone();
            if available.iter().any(|n| n.assets.ledger_id != first) {
                return Err("wallet_node_ledger_ambiguous".into());
            }
            first
        }
    };
    available.retain(|n| n.assets.ledger_id == anchor);
    available.sort_by_key(|n| (n.response_ms, n.session.node.id != selected));
    available
        .into_iter()
        .next()
        .ok_or_else(|| "wallet_node_no_healthy".into())
}

#[cfg(test)]
#[path = "node_selection_tests.rs"]
mod tests;
