use super::{merge, Candidate, DiscoveryReport};
use futures_util::{stream, StreamExt};
use if_addrs::IfAddr;
use reqwest::{redirect::Policy, Client};
use serde_json::Value;
use std::{
    collections::BTreeSet,
    net::{Ipv4Addr, SocketAddr},
    time::Duration,
};
use tokio_util::sync::CancellationToken;

const MAX_HOSTS: usize = 508;
const CONCURRENCY: usize = 24;
const RESPONSE_LIMIT: usize = 4096;

#[path = "interfaces.rs"]
mod interfaces;

pub(super) fn targets(networks: &[(Ipv4Addr, Ipv4Addr)]) -> (Vec<Ipv4Addr>, bool) {
    let own: BTreeSet<_> = networks.iter().map(|(ip, _)| *ip).collect();
    let mut targets = BTreeSet::new();
    let mut limited = false;
    for &(ip, mask) in networks {
        if !ip.is_private() {
            continue;
        }
        let mask = u32::from(mask);
        let prefix = mask.leading_ones();
        if mask != u32::MAX.checked_shl(32 - prefix).unwrap_or(0) || prefix > 30 {
            continue;
        }
        limited |= prefix < 24;
        let bounded_mask = mask | 0xffffff00;
        let first = (u32::from(ip) & bounded_mask) + 1;
        let last = u32::from(ip) | !bounded_mask;
        for value in first..last {
            let target = Ipv4Addr::from(value);
            if own.contains(&target) || !target.is_private() {
                continue;
            }
            if targets.len() == MAX_HOSTS && !targets.contains(&target) {
                limited = true;
                break;
            }
            targets.insert(target);
        }
    }
    (targets.into_iter().collect(), limited)
}

pub(super) fn discovery_client() -> Result<Client, reqwest::Error> {
    // Separate client: no session cookies, proxy, authorization or TLS bypass.
    Client::builder()
        .no_proxy()
        .redirect(Policy::none())
        .connect_timeout(Duration::from_millis(600))
        .timeout(Duration::from_millis(1200))
        .pool_max_idle_per_host(0)
        .build()
}

pub(super) fn session_contract(body: &[u8]) -> bool {
    let Ok(value) = serde_json::from_slice::<Value>(body) else {
        return false;
    };
    value.get("ok") == Some(&Value::Bool(true))
        && value.pointer("/data/logged_in") == Some(&Value::Bool(false))
        && ["csrf_token", "username", "role"]
            .iter()
            .all(|field| value.get("data").and_then(|d| d.get(field)) == Some(&Value::Null))
}

pub(super) async fn probe_session(client: &Client, endpoint: SocketAddr) -> bool {
    let Ok(mut response) = client
        .get(format!("http://{endpoint}/webd/session"))
        .send()
        .await
    else {
        return false;
    };
    if response.status() != 200
        || !response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.split(';').next().unwrap_or("").trim() == "application/json")
        || response
            .content_length()
            .is_some_and(|n| n > RESPONSE_LIMIT as u64)
    {
        return false;
    }
    let mut body = Vec::new();
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) if body.len() + chunk.len() <= RESPONSE_LIMIT => {
                body.extend_from_slice(&chunk)
            }
            Ok(None) => return session_contract(&body),
            _ => return false,
        }
    }
}

async fn port_open(ip: Ipv4Addr, port: u16) -> bool {
    matches!(
        tokio::time::timeout(
            Duration::from_millis(500),
            tokio::net::TcpStream::connect((ip, port))
        )
        .await,
        Ok(Ok(_))
    )
}

async fn probe(client: &Client, ip: Ipv4Addr) -> Option<Candidate> {
    if !probe_session(client, (ip, 80).into()).await {
        return None;
    }
    // An open port is just a hint; real TLS / SSH verification still happens on connect.
    let (kind, port, address) = if port_open(ip, 443).await {
        ("https", 443, format!("https://{ip}"))
    } else if port_open(ip, 22).await {
        ("ssh", 22, ip.to_string())
    } else {
        return None;
    };
    Some(Candidate {
        name: ip.to_string(),
        address,
        kind,
        port,
        source: "subnet",
        private_ca: false,
        verified: false,
        ips: vec![ip],
    })
}

pub(super) async fn scan(cancel: CancellationToken) -> DiscoveryReport {
    let mut report = DiscoveryReport::default();
    if cancel.is_cancelled() {
        return report;
    }
    let Ok(interfaces) = if_addrs::get_if_addrs() else {
        return report;
    };
    let networks: Vec<_> = interfaces
        .into_iter()
        .filter(|i| i.is_oper_up() && !i.is_p2p() && interfaces::physical_interface(i))
        .filter_map(|i| match i.addr {
            IfAddr::V4(v) => Some((v.ip, v.netmask)),
            _ => None,
        })
        .collect();
    let (hosts, limited) = targets(&networks);
    report.limited = limited;
    report.subnet_available = !hosts.is_empty();
    let Ok(client) = discovery_client() else {
        report.subnet_available = false;
        return report;
    };
    let results = stream::iter(hosts.into_iter().map(|ip| {
        let client = client.clone();
        async move { probe(&client, ip).await }
    }))
    .buffer_unordered(CONCURRENCY);
    tokio::pin!(results);
    let deadline = tokio::time::sleep(Duration::from_secs(20));
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => break,
            _ = &mut deadline => { report.limited = true; break; },
            result = results.next() => match result {
                Some(candidate) => {
                    report.scanned_hosts += 1;
                    if let Some(candidate) = candidate { merge(&mut report.candidates, candidate); }
                },
                None => break,
            }
        }
    }
    report
}
