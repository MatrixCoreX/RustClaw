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

#[test]
fn explicit_small_memory_request_is_only_honored_on_low_memory_hosts() {
    assert_eq!(requested_memory_mb(64, Some(1024)), 64);
    assert_eq!(
        requested_memory_mb(64, Some(4096)),
        DEFAULT_SKILL_MEMORY_MIB
    );
    assert_eq!(requested_memory_mb(64, None), DEFAULT_SKILL_MEMORY_MIB);
    assert_eq!(requested_memory_mb(0, Some(1024)), DEFAULT_SKILL_MEMORY_MIB);
}

#[test]
fn low_memory_host_serializes_runtime_work_and_disables_warm_pool() {
    let plan = runtime_concurrency_plan_for_host(4, 3, 2, true, 4, Some(1024));
    assert_eq!(plan.worker_concurrency, 1);
    assert_eq!(plan.skill_concurrency, 1);
    assert_eq!(plan.memory_background_concurrency, 1);
    assert!(!plan.runner_warm_pool_enabled);
}

#[test]
fn constrained_host_keeps_two_foreground_slots_but_one_background_slot() {
    let plan = runtime_concurrency_plan_for_host(4, 4, 4, true, 4, Some(4096));
    assert_eq!(plan.worker_concurrency, 2);
    assert_eq!(plan.skill_concurrency, 2);
    assert_eq!(plan.memory_background_concurrency, 1);
    assert!(!plan.runner_warm_pool_enabled);
}

#[test]
fn capable_host_preserves_configured_limits() {
    let plan = runtime_concurrency_plan_for_host(3, 2, 2, true, 8, Some(16 * 1024));
    assert_eq!(plan.worker_concurrency, 3);
    assert_eq!(plan.skill_concurrency, 2);
    assert_eq!(plan.memory_background_concurrency, 2);
    assert!(plan.runner_warm_pool_enabled);
}

#[test]
fn cpu_count_remains_a_hard_concurrency_ceiling() {
    let plan = runtime_concurrency_plan_for_host(8, 8, 8, true, 2, None);
    assert_eq!(plan.worker_concurrency, 2);
    assert_eq!(plan.skill_concurrency, 2);
    assert_eq!(plan.memory_background_concurrency, 2);
}
