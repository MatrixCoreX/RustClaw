use super::*;

#[test]
fn health_check_extra_exposes_runtime_owned_database_probe() {
    let enriched = enrich_health_check_extra(
        Some(json!({
            "clawd_process_count": 1,
            "clawd_health_port_open": true,
            "system_health": {"warnings": []},
        })),
        true,
    );

    assert_eq!(enriched["db_available"], true);
    assert_eq!(enriched["clawd_visible"], true);
    assert_eq!(enriched["overall_status"], "healthy");
    assert_eq!(
        enriched["runtime_probe"]["database"]["source"],
        "runtime_pool_select_1"
    );
}

#[test]
fn health_check_extra_reports_degraded_when_database_probe_fails() {
    let enriched = enrich_health_check_extra(
        Some(json!({
            "clawd_process_count": 1,
            "clawd_health_port_open": true,
        })),
        false,
    );

    assert_eq!(enriched["db_available"], false);
    assert_eq!(enriched["clawd_visible"], true);
    assert_eq!(enriched["overall_status"], "degraded");
}
