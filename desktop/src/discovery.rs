//! Discovery returns untrusted hints only. It never accesses profiles or credentials.
#[cfg(feature = "gui")]
pub mod commands;
mod local;
mod mdns;
mod subnet;
#[cfg(feature = "gui")]
pub use commands::{cancel_discovery, discover_devices, DiscoveryState};

use serde::Serialize;
use std::net::Ipv4Addr;
use tokio_util::sync::CancellationToken;

pub const SERVICE_TYPE: &str = "_agent-runtime._tcp.local.";
const MAX_CANDIDATES: usize = 64;

#[derive(Clone, Debug, Serialize)]
pub struct Candidate {
    pub name: String,
    pub address: String,
    pub kind: &'static str,
    pub source: &'static str,
    pub private_ca: bool,
    pub verified: bool,
    #[serde(skip)]
    pub ips: Vec<Ipv4Addr>,
    #[serde(skip)]
    pub port: u16,
}

#[derive(Default, Serialize)]
pub struct DiscoveryReport {
    pub local: local::LocalReport,
    pub candidates: Vec<Candidate>,
    pub mdns_available: bool,
    pub subnet_available: bool,
    pub scanned_hosts: usize,
    pub limited: bool,
    pub cancelled: bool,
}

pub async fn discover(scan_subnet: bool, cancel: CancellationToken) -> DiscoveryReport {
    let mdns_cancel = cancel.clone();
    let mdns = tokio::task::spawn_blocking(move || mdns::browse(mdns_cancel));
    let subnet = async {
        if scan_subnet {
            subnet::scan(cancel.clone()).await
        } else {
            DiscoveryReport::default()
        }
    };
    let (mdns, mut report, (local, local_candidates)) =
        tokio::join!(mdns, subnet, local::scan(cancel.clone()));
    report.local = local;
    if let Ok(Ok(candidates)) = mdns {
        report.mdns_available = true;
        // Prefer the advertised hostname, allowing certificates bound to a stable name.
        let scanned = std::mem::replace(&mut report.candidates, candidates);
        for candidate in scanned {
            merge(&mut report.candidates, candidate);
        }
    }
    for candidate in local_candidates {
        merge(&mut report.candidates, candidate);
    }
    report.candidates.sort_by(|a, b| a.address.cmp(&b.address));
    report.cancelled = cancel.is_cancelled();
    report
}

fn merge(candidates: &mut Vec<Candidate>, candidate: Candidate) {
    if candidates.len() < MAX_CANDIDATES
        && !candidates.iter().any(|c| {
            c.address == candidate.address
                || (c.kind == candidate.kind
                    && c.port == candidate.port
                    && c.ips.iter().any(|ip| candidate.ips.contains(ip)))
        })
    {
        candidates.push(candidate);
    }
}

#[cfg(test)]
mod tests;
