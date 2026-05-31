use super::{
    build_system_warnings, load_is_high, parse_df_root_kilobytes, parse_linux_meminfo,
    parse_linux_uptime, parse_macos_available_memory_bytes, parse_macos_boot_time_seconds,
    parse_macos_load_avg, resource_is_low, SystemHealthSnapshot,
};

fn snapshot() -> SystemHealthSnapshot {
    SystemHealthSnapshot {
        os_family: "linux".to_string(),
        arch: "x86_64".to_string(),
        kernel_release: Some("6.8.0".to_string()),
        hostname: Some("demo".to_string()),
        service_manager: "systemd".to_string(),
        cpu_count: Some(4),
        uptime_seconds: Some(10),
        load_avg_1m: Some(0.5),
        load_avg_5m: Some(0.4),
        load_avg_15m: Some(0.3),
        memory_total_bytes: Some(8 * 1024 * 1024 * 1024),
        memory_available_bytes: Some(4 * 1024 * 1024 * 1024),
        disk_root_total_bytes: Some(100 * 1024 * 1024 * 1024),
        disk_root_available_bytes: Some(40 * 1024 * 1024 * 1024),
        warnings: Vec::new(),
    }
}

#[test]
fn linux_meminfo_parser_reads_total_and_available() {
    let text =
        "MemTotal:       16384256 kB\nMemFree:         2048000 kB\nMemAvailable:    8192000 kB\n";
    let (total, available) = parse_linux_meminfo(text);
    assert_eq!(total, Some(16_384_256 * 1024));
    assert_eq!(available, Some(8_192_000 * 1024));
}

#[test]
fn macos_vm_stat_parser_estimates_available_bytes() {
    let vm_stat = "\
Mach Virtual Memory Statistics: (page size of 16384 bytes)\n\
Pages free:                               100.\n\
Pages active:                            5000.\n\
Pages inactive:                           250.\n\
Pages speculative:                         50.\n";
    assert_eq!(
        parse_macos_available_memory_bytes(vm_stat, 16_384),
        Some((100 + 250 + 50) * 16_384)
    );
}

#[test]
fn df_parser_reads_root_capacity() {
    let text = "\
Filesystem 1024-blocks      Used Available Capacity Mounted on\n\
/dev/disk3s1   976490576 12345678 98765432    12% /\n";
    assert_eq!(
        parse_df_root_kilobytes(text),
        Some((976_490_576, 98_765_432))
    );
}

#[test]
fn system_warnings_include_low_disk_memory_and_high_load() {
    let mut data = snapshot();
    data.memory_available_bytes = Some(128 * 1024 * 1024);
    data.disk_root_available_bytes = Some(2 * 1024 * 1024 * 1024);
    data.load_avg_1m = Some(12.0);
    let warnings = build_system_warnings(&data);
    assert!(warnings.contains(&"memory_available_low".to_string()));
    assert!(warnings.contains(&"disk_root_low".to_string()));
    assert!(warnings.contains(&"load_high".to_string()));
}

#[test]
fn resource_thresholds_use_absolute_or_percent_floor() {
    assert!(resource_is_low(
        Some(10 * 1024 * 1024 * 1024),
        Some(400 * 1024 * 1024),
        512 * 1024 * 1024,
        0.10,
    ));
    assert!(resource_is_low(
        Some(100 * 1024 * 1024 * 1024),
        Some(8 * 1024 * 1024 * 1024),
        5 * 1024 * 1024 * 1024,
        0.10,
    ));
}

#[test]
fn load_warning_threshold_scales_with_cpu_count() {
    assert!(load_is_high(Some(8.5), Some(4)));
    assert!(!load_is_high(Some(3.5), Some(4)));
}

#[test]
fn parse_os_specific_runtime_values() {
    assert_eq!(parse_linux_uptime("12345.67 67890.12"), Some(12_345));
    assert_eq!(
        parse_macos_boot_time_seconds("{ sec = 1718649985, usec = 0 }"),
        Some(1_718_649_985)
    );
    assert_eq!(
        parse_macos_load_avg("{ 2.31 1.82 1.40 }"),
        (Some(2.31), Some(1.82), Some(1.40))
    );
}
