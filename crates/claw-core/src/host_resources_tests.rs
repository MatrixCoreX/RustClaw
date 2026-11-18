use super::*;

struct CgroupFixture(PathBuf);

impl CgroupFixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("host-cgroup-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        Self(root)
    }

    fn write(&self, name: &str, value: &str) {
        fs::write(self.0.join(name), value).unwrap();
    }
}

impl Drop for CgroupFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn cgroup_directory_candidates_keep_membership_before_root_without_duplicates() {
    let fixture = CgroupFixture::new();
    assert_eq!(
        cgroup_directories(&fixture.0, Path::new("/")),
        vec![fixture.0.clone()]
    );
    for membership in ["/user.slice/agent.scope", "user.slice/agent.scope"] {
        assert_eq!(
            cgroup_directories(&fixture.0, Path::new(membership)),
            vec![fixture.0.join("user.slice/agent.scope"), fixture.0.clone()]
        );
    }
}

#[test]
fn cgroup_v2_directory_fixture_preserves_capacity_usage_events_and_pressure() {
    let fixture = CgroupFixture::new();
    fixture.write("memory.max", "536870912\n");
    fixture.write("memory.current", "268435456\n");
    fixture.write("memory.swap.max", "134217728\n");
    fixture.write("memory.swap.current", "0\n");
    fixture.write("memory.events", "high 2\noom 3\noom_kill 1\n");
    fixture.write(
        "memory.pressure",
        "some avg10=1.25 avg60=2.50\nfull avg10=0.50 avg60=0.75\n",
    );
    let value = collect_cgroup_v2_dir(&fixture.0);
    assert_eq!(value.version, Some(2));
    assert_eq!(value.memory_limit_bytes, Some(536_870_912));
    assert_eq!(value.memory_current_bytes, Some(268_435_456));
    assert_eq!(value.swap_limit_bytes, Some(134_217_728));
    assert_eq!(value.swap_used_bytes, Some(0));
    assert_eq!(value.events.high, 2);
    assert_eq!(value.events.oom, 3);
    assert_eq!(value.events.oom_kill, 1);
    let pressure = value.pressure.unwrap();
    assert_eq!(pressure.some_avg10, Some(1.25));
    assert_eq!(pressure.full_avg60, Some(0.75));

    fixture.write("memory.max", "max\n");
    fixture.write("memory.current", "invalid\n");
    fixture.write("memory.swap.current", "-1\n");
    let unavailable = collect_cgroup_v2_dir(&fixture.0);
    assert_eq!(unavailable.memory_limit_bytes, None);
    assert_eq!(unavailable.memory_current_bytes, None);
    assert_eq!(unavailable.swap_used_bytes, None);
}

#[test]
fn cgroup_v1_directory_fixture_subtracts_combined_swap_without_underflow() {
    let fixture = CgroupFixture::new();
    fixture.write("memory.limit_in_bytes", "536870912\n");
    fixture.write("memory.usage_in_bytes", "268435456\n");
    fixture.write("memory.memsw.limit_in_bytes", "805306368\n");
    fixture.write("memory.memsw.usage_in_bytes", "335544320\n");
    let value = collect_cgroup_v1_dir(&fixture.0);
    assert_eq!(value.version, Some(1));
    assert_eq!(value.memory_limit_bytes, Some(536_870_912));
    assert_eq!(value.memory_current_bytes, Some(268_435_456));
    assert_eq!(value.swap_limit_bytes, Some(268_435_456));
    assert_eq!(value.swap_used_bytes, Some(67_108_864));
    assert_eq!(value.events, CgroupMemoryEvents::default());
    assert_eq!(value.pressure, None);
    fixture.write("memory.memsw.limit_in_bytes", "1\n");
    fixture.write("memory.memsw.usage_in_bytes", "1\n");
    let saturated = collect_cgroup_v1_dir(&fixture.0);
    assert_eq!(saturated.swap_limit_bytes, Some(0));
    assert_eq!(saturated.swap_used_bytes, Some(0));
}

#[test]
fn cgroup_directory_fixture_keeps_missing_capacity_unknown_in_both_versions() {
    let fixture = CgroupFixture::new();
    for value in [
        collect_cgroup_v1_dir(&fixture.0),
        collect_cgroup_v2_dir(&fixture.0),
    ] {
        assert_eq!(value.memory_limit_bytes, None);
        assert_eq!(value.memory_current_bytes, None);
        assert_eq!(value.swap_limit_bytes, None);
        assert_eq!(value.swap_used_bytes, None);
        assert_eq!(value.pressure, None);
    }
}

fn snapshot(limit_mib: u64, available_mib: u64) -> HostResourceSnapshot {
    HostResourceSnapshot {
        schema_version: 1,
        sampled_at_epoch_ms: 1,
        physical_memory_bytes: Some(limit_mib * MIB),
        effective_memory_limit_bytes: Some(limit_mib * MIB),
        memory_available_bytes: Some(available_mib * MIB),
        memory_current_bytes: Some((limit_mib - available_mib) * MIB),
        swap_available_bytes: Some(4096 * MIB),
        swap_used_bytes: Some(0),
        cpu_parallelism: 4,
        cgroup_version: Some(2),
        cgroup_events: CgroupMemoryEvents::default(),
        memory_pressure: MemoryPressureSample::default(),
        memory_source: "fixture",
        pressure_source: "fixture",
    }
}

#[test]
fn linux_meminfo_parser_uses_bytes_and_saturates_swap() {
    let value = parse_linux_meminfo(
        "MemTotal: 2048000 kB\nMemAvailable: 512000 kB\nSwapTotal: 100 kB\nSwapFree: 120 kB\n",
    );
    assert_eq!(value.physical_memory_bytes, Some(2_048_000 * 1024));
    assert_eq!(value.memory_available_bytes, Some(512_000 * 1024));
    assert_eq!(value.swap_available_bytes, Some(120 * 1024));
    assert_eq!(value.swap_used_bytes, Some(0));
}

#[test]
fn cgroup_membership_and_unlimited_limits_are_parsed_without_guessing() {
    assert_eq!(
        parse_cgroup_v2_path("0::/user.slice/example.service\n"),
        Some(PathBuf::from("/user.slice/example.service"))
    );
    assert_eq!(
        parse_cgroup_v1_memory_path("7:cpu:/x\n5:memory,blkio:/limited\n"),
        Some(PathBuf::from("/limited"))
    );
    let root = std::env::temp_dir().join(format!("host-resource-limit-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("max"), "max\n").unwrap();
    fs::write(root.join("huge"), format!("{}\n", 1_u64 << 62)).unwrap();
    fs::write(root.join("bounded"), "536870912\n").unwrap();
    assert_eq!(read_limit_file(&root.join("max")), None);
    assert_eq!(read_limit_file(&root.join("huge")), None);
    assert_eq!(read_limit_file(&root.join("bounded")), Some(536_870_912));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn pressure_and_event_parsers_ignore_unknown_fields() {
    let pressure = parse_linux_pressure(
        "some avg10=1.25 avg60=2.50 avg300=3.0 total=9\nfull avg10=0.50 avg60=0.75 avg300=1 total=2\n",
    );
    assert_eq!(pressure.some_avg10, Some(1.25));
    assert_eq!(pressure.full_avg60, Some(0.75));
    let events = parse_cgroup_events("low 1\nhigh 2\nmax 3\noom 4\noom_kill 5\nfuture 99\n");
    assert_eq!(events.high, 2);
    assert_eq!(events.oom_kill, 5);
}

#[test]
fn macos_fixture_counts_reclaimable_pages_and_swap_units() {
    let vm =
        "Pages free: 100.\nPages inactive: 200.\nPages speculative: 50.\nPages purgeable: 25.\n";
    assert_eq!(parse_macos_available_memory(vm, 4096), Some(375 * 4096));
    assert_eq!(
        parse_macos_swap_used("total = 4096.00M  used = 1.50G  free = 2.50G"),
        Some(1_610_612_736)
    );
}

#[test]
fn effective_limit_and_available_helpers_are_consistent() {
    assert_eq!(minimum_known(Some(4), Some(2)), Some(2));
    assert_eq!(minimum_known(Some(4), None), Some(4));
    let value = snapshot(2048, 512);
    assert_eq!(value.effective_memory_mib(), Some(2048));
    assert_eq!(value.available_memory_mib(), Some(512));
    assert_eq!(value.available_swap_mib(), Some(4096));
    assert_eq!(value.available_ratio(), Some(0.25));
}

#[test]
fn pressure_classification_uses_capacity_available_memory_and_psi() {
    let policy = ResourcePressurePolicy::default();
    assert_eq!(
        classify_pressure(&snapshot(1024, 700), policy),
        ResourcePressureState::Constrained
    );
    assert_eq!(
        classify_pressure(&snapshot(8192, 5000), policy),
        ResourcePressureState::Normal
    );
    assert_eq!(
        classify_pressure(&snapshot(8192, 500), policy),
        ResourcePressureState::Critical
    );
    let mut pressured = snapshot(8192, 5000);
    pressured.memory_pressure.full_avg10 = Some(12.0);
    assert_eq!(
        classify_pressure(&pressured, policy),
        ResourcePressureState::Constrained
    );
}

#[test]
fn tracker_requires_hysteresis_but_escalates_new_oom_immediately() {
    let policy = ResourcePressurePolicy {
        escalation_samples: 2,
        recovery_samples: 3,
        ..ResourcePressurePolicy::default()
    };
    let mut tracker = ResourcePressureTracker::new(policy);
    let constrained = snapshot(1024, 700);
    assert_eq!(tracker.observe(&constrained), ResourcePressureState::Normal);
    assert_eq!(
        tracker.observe(&constrained),
        ResourcePressureState::Constrained
    );
    let normal = snapshot(8192, 5000);
    assert_eq!(tracker.observe(&normal), ResourcePressureState::Constrained);
    assert_eq!(tracker.observe(&normal), ResourcePressureState::Constrained);
    assert_eq!(tracker.observe(&normal), ResourcePressureState::Normal);
    let mut oom = normal;
    oom.cgroup_events.oom_kill = 1;
    assert_eq!(tracker.observe(&oom), ResourcePressureState::Critical);
}

#[test]
fn tracker_does_not_treat_preexisting_event_counters_as_new_pressure() {
    let policy = ResourcePressurePolicy {
        escalation_samples: 1,
        recovery_samples: 1,
        ..ResourcePressurePolicy::default()
    };
    let mut tracker = ResourcePressureTracker::new(policy);
    let mut normal = snapshot(8192, 5000);
    normal.cgroup_events.oom_kill = 7;
    assert_eq!(tracker.observe(&normal), ResourcePressureState::Normal);
    normal.cgroup_events.high = 1;
    assert_eq!(tracker.observe(&normal), ResourcePressureState::Critical);
}
