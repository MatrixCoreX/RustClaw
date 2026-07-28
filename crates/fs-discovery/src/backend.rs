use std::time::Instant;

use crate::{
    ripgrep, walker, BackendPreference, DiscoveryError, DiscoveryReport, DiscoveryRequest,
    RipgrepStatus, TargetKind,
};

pub fn discover(request: &DiscoveryRequest) -> Result<DiscoveryReport, DiscoveryError> {
    match request.backend {
        BackendPreference::Rust => walker::discover_rust(request, None),
        BackendPreference::Ripgrep => ripgrep::discover(request).map_err(|failure| {
            if failure.unavailable {
                DiscoveryError::BackendUnavailable(failure.reason_code)
            } else {
                DiscoveryError::BackendFailed(failure.reason_code)
            }
        }),
        BackendPreference::Auto => discover_auto(request),
    }
}

fn discover_auto(request: &DiscoveryRequest) -> Result<DiscoveryReport, DiscoveryError> {
    if request.selector.target_kind != TargetKind::File {
        return walker::discover_rust(
            request,
            Some("directories_require_rust_backend".to_string()),
        );
    }
    let started = Instant::now();
    match ripgrep::discover(request) {
        Ok(report) => Ok(report),
        Err(failure) => walker::discover_rust(
            request,
            Some(format!(
                "{};ripgrep_attempt_ms={}",
                failure.reason_code,
                started.elapsed().as_millis()
            )),
        ),
    }
}

pub fn ripgrep_status() -> RipgrepStatus {
    ripgrep::status()
}
