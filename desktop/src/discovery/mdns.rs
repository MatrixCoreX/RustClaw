use super::{merge, Candidate, SERVICE_TYPE};
use mdns_sd::{ResolvedService, ServiceDaemon, ServiceEvent};
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

pub(super) fn candidate(info: &ResolvedService) -> Option<Candidate> {
    let host = info.get_hostname().trim_end_matches('.');
    if host.len() > 253
        || !host.ends_with(".local")
        || host.split('.').any(|label| {
            label.is_empty()
                || label.len() > 63
                || !label
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-')
                || label.starts_with('-')
                || label.ends_with('-')
        })
        || info.get_port() == 0
        || info
            .get_property_val_str("scheme")
            .is_some_and(|v| v != "https")
        || info
            .get_property_val_str("api")
            .is_some_and(|v| v != "webd-v1")
    {
        return None;
    }
    let mut ips: Vec<_> = info
        .get_addresses_v4()
        .into_iter()
        .filter(|ip| ip.is_private())
        .collect();
    ips.sort();
    if ips.is_empty() {
        return None;
    }
    Some(Candidate {
        name: host.to_owned(),
        address: format!("https://{host}:{}", info.get_port()),
        kind: "https",
        source: "mdns",
        private_ca: info.get_property_val_str("tls") == Some("private_ca"),
        verified: false,
        ips,
        port: info.get_port(),
    })
}

#[cfg(not(target_os="android"))]
pub(super) fn browse(cancel: CancellationToken) -> crate::Result<Vec<Candidate>> {
    let daemon = ServiceDaemon::new().map_err(|_| "discovery_unavailable")?;
    let outcome = (|| {
        let receiver = daemon
            .browse(SERVICE_TYPE)
            .map_err(|_| "discovery_unavailable")?;
        let end = Instant::now() + Duration::from_secs(4);
        let mut candidates = Vec::new();
        while Instant::now() < end && !cancel.is_cancelled() {
            if let Ok(ServiceEvent::ServiceResolved(info)) =
                receiver.recv_timeout(Duration::from_millis(100))
            {
                if let Some(candidate) = candidate(&info) {
                    merge(&mut candidates, candidate);
                }
            }
        }
        Ok(candidates)
    })();
    let _ = daemon.stop_browse(SERVICE_TYPE);
    let _ = daemon.shutdown();
    outcome
}

#[cfg(target_os="android")]
pub(super) fn browse(cancel: CancellationToken) -> crate::Result<Vec<Candidate>> {
    if cancel.is_cancelled() { return Ok(vec![]); }
    let records = crate::android::bridge::string("discoverMdns", &[])?.ok_or("discovery_unavailable")?;
    if cancel.is_cancelled() { return Ok(vec![]); }
    #[derive(serde::Deserialize)]
    struct Found { name: String, ips: Vec<std::net::Ipv4Addr>, port: u16, private_ca: bool }
    let found: Vec<Found> = serde_json::from_str(&records).map_err(|_| "discovery_unavailable")?;
    Ok(found.into_iter().take(64).filter_map(|item| {
        let ip = item.ips.into_iter().find(|ip| ip.is_private())?;
        if item.port == 0 || item.name.len() > 253 { return None; }
        Some(Candidate { name: item.name, address: format!("https://{ip}:{}",item.port),
            kind: "https", source: "mdns", private_ca: item.private_ca,
            verified: false, ips: vec![ip], port: item.port })
    }).collect())
}
