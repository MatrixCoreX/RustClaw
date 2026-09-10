use super::{subnet, Candidate};
use futures_util::{stream, StreamExt};
use serde::Serialize;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use tokio_util::sync::CancellationToken;

#[path = "local_installation.rs"]
mod installation;

#[derive(Default, Serialize)]
pub struct LocalReport {
    pub checked: bool,
    pub installation_hint: bool,
    pub running: bool,
}

pub(super) async fn probe(endpoints: Vec<SocketAddr>, cancel: CancellationToken) -> Vec<Candidate> {
    let Ok(client) = subnet::discovery_client() else {
        return Vec::new();
    };
    let requests = stream::iter(
        endpoints
            .into_iter()
            .filter(|endpoint| {
                matches!(endpoint.ip(), IpAddr::V4(ip) if ip == Ipv4Addr::LOCALHOST)
                    || matches!(endpoint.ip(), IpAddr::V6(ip) if ip == Ipv6Addr::LOCALHOST)
            })
            .map(|endpoint| {
                let client = client.clone();
                async move {
                    if !subnet::probe_session(&client, endpoint).await {
                        return None;
                    }
                    Some(Candidate {
                        name: "127.0.0.1".into(),
                        address: format!("http://{endpoint}"),
                        kind: "local",
                        source: "local",
                        private_ca: false,
                        verified: false,
                        ips: Vec::new(),
                        port: endpoint.port(),
                    })
                }
            }),
    )
    .buffer_unordered(6);
    tokio::pin!(requests);
    let mut candidates = Vec::new();
    loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => break,
            result = requests.next() => match result {
                Some(Some(candidate)) => candidates.push(candidate),
                Some(None) => {},
                None => break,
            },
        }
    }
    candidates.sort_by(|a, b| a.address.cmp(&b.address));
    candidates
}

pub(super) async fn scan(cancel: CancellationToken) -> (LocalReport, Vec<Candidate>) {
    if cancel.is_cancelled() {
        return (LocalReport::default(), Vec::new());
    }
    // Only documented local web-console defaults, never a sweep over local ports.
    let endpoints = [8788, 80]
        .into_iter()
        .flat_map(|port| {
            [
                SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
                SocketAddr::from((Ipv6Addr::LOCALHOST, port)),
            ]
        })
        .collect();
    let installation = tokio::task::spawn_blocking(installation::installed_hint);
    let candidates = probe(endpoints, cancel.clone()).await;
    let installation_hint = tokio::select! {
        _ = cancel.cancelled() => false,
        result = tokio::time::timeout(std::time::Duration::from_secs(2), installation) => result.ok().and_then(Result::ok).unwrap_or(false),
    };
    (
        LocalReport {
            checked: !cancel.is_cancelled(),
            installation_hint,
            running: !candidates.is_empty(),
        },
        candidates,
    )
}

#[cfg(test)]
#[path = "local_tests.rs"]
mod tests;
