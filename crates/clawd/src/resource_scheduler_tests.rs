use super::*;

#[test]
fn host_grant_never_exceeds_worker_safety_ceiling() {
    let request = SkillResourceRequest {
        class: SkillResourceClass::Cpu,
        cpu_cores: 2,
        memory_mb: 512,
        ..SkillResourceRequest::default()
    };
    let grant = host_grant(Some(&request), 3);
    assert!(grant.admitted);
    assert!((1..=3).contains(&grant.max_concurrency));
}

#[test]
fn missing_gpu_uses_declared_fallback_or_waits() {
    if gpu_device_available() {
        return;
    }
    let mut request = SkillResourceRequest {
        class: SkillResourceClass::Gpu,
        gpu_slots: 1,
        ..SkillResourceRequest::default()
    };
    assert!(!host_grant(Some(&request), 4).admitted);
    request.allow_cpu_fallback = true;
    assert!(host_grant(Some(&request), 4).admitted);
}

#[test]
fn impossible_memory_request_is_not_started() {
    let request = SkillResourceRequest {
        class: SkillResourceClass::Memory,
        memory_mb: u64::MAX,
        ..SkillResourceRequest::default()
    };
    let grant = host_grant(Some(&request), 4);
    assert!(!grant.admitted);
    assert_eq!(grant.wait_reason, Some("memory_unavailable"));
}
