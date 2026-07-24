const HOST_TEXT_LIMIT_BYTES: u64 = 64 * 1024;
#[cfg(target_os = "macos")]
const HOST_COMMAND_LIMIT_BYTES: usize = 4096;

#[derive(Debug, Clone, Serialize)]
struct HostSystemSummary {
    schema_version: u32,
    collected_at_ts: i64,
    os: HostOperatingSystem,
    architecture: String,
    deployment: Option<String>,
    memory: HostCapacity,
    storage: HostCapacity,
    uptime_seconds: Option<u64>,
    unavailable_fields: Vec<HostUnavailableField>,
}

#[derive(Debug, Clone, Serialize)]
struct HostOperatingSystem {
    family: String,
    name: Option<String>,
    version: Option<String>,
    kernel: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct HostCapacity {
    total_bytes: Option<u64>,
    available_bytes: Option<u64>,
    available_ratio: Option<f64>,
}

impl HostCapacity {
    fn new(total_bytes: Option<u64>, available_bytes: Option<u64>) -> Self {
        let available_ratio = match (total_bytes, available_bytes) {
            (Some(total), Some(available)) if total > 0 => {
                Some((available as f64 / total as f64).clamp(0.0, 1.0))
            }
            _ => None,
        };
        Self {
            total_bytes,
            available_bytes,
            available_ratio,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct HostUnavailableField {
    field: &'static str,
    code: &'static str,
}

#[derive(Debug, Default)]
struct HostPlatformSnapshot {
    os_name: Option<String>,
    os_version: Option<String>,
    kernel: Option<String>,
    deployment: Option<String>,
    memory_total_bytes: Option<u64>,
    memory_available_bytes: Option<u64>,
    uptime_seconds: Option<u64>,
}

async fn host_system_summary(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(response) = require_ui_identity(&state, &headers) {
        return response;
    }

    let workspace_root = state.skill_rt.workspace_root.clone();
    let summary = tokio::task::spawn_blocking(move || collect_host_system_summary(&workspace_root))
        .await
        .unwrap_or_else(|_| HostSystemSummary::collection_failed());
    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(json!(summary)),
            error: None,
        }),
    )
}

impl HostSystemSummary {
    fn collection_failed() -> Self {
        let unavailable_fields = [
            "os.name",
            "os.version",
            "os.kernel",
            "deployment",
            "memory.total_bytes",
            "memory.available_bytes",
            "storage.total_bytes",
            "storage.available_bytes",
            "uptime_seconds",
        ]
        .into_iter()
        .map(|field| HostUnavailableField {
            field,
            code: "collector_failed",
        })
        .collect();
        Self {
            schema_version: 1,
            collected_at_ts: now_unix_seconds(),
            os: HostOperatingSystem {
                family: std::env::consts::OS.to_string(),
                name: None,
                version: None,
                kernel: None,
            },
            architecture: std::env::consts::ARCH.to_string(),
            deployment: None,
            memory: HostCapacity::new(None, None),
            storage: HostCapacity::new(None, None),
            uptime_seconds: None,
            unavailable_fields,
        }
    }
}

fn collect_host_system_summary(workspace_root: &Path) -> HostSystemSummary {
    let platform = collect_host_platform_snapshot();
    let data_volume = nearest_existing_path(&workspace_root.join("data"));
    let (storage_total_bytes, storage_available_bytes) = data_volume
        .as_deref()
        .and_then(storage_capacity_bytes)
        .unwrap_or((None, None));
    let mut unavailable_fields = Vec::new();

    record_unavailable(
        &mut unavailable_fields,
        "os.name",
        &platform.os_name,
        "os_name_unavailable",
    );
    record_unavailable(
        &mut unavailable_fields,
        "os.version",
        &platform.os_version,
        "os_version_unavailable",
    );
    record_unavailable(
        &mut unavailable_fields,
        "os.kernel",
        &platform.kernel,
        "kernel_unavailable",
    );
    record_unavailable(
        &mut unavailable_fields,
        "deployment",
        &platform.deployment,
        "deployment_unavailable",
    );
    record_unavailable(
        &mut unavailable_fields,
        "memory.total_bytes",
        &platform.memory_total_bytes,
        "memory_total_unavailable",
    );
    record_unavailable(
        &mut unavailable_fields,
        "memory.available_bytes",
        &platform.memory_available_bytes,
        "memory_available_unavailable",
    );
    record_unavailable(
        &mut unavailable_fields,
        "storage.total_bytes",
        &storage_total_bytes,
        "storage_total_unavailable",
    );
    record_unavailable(
        &mut unavailable_fields,
        "storage.available_bytes",
        &storage_available_bytes,
        "storage_available_unavailable",
    );
    record_unavailable(
        &mut unavailable_fields,
        "uptime_seconds",
        &platform.uptime_seconds,
        "uptime_unavailable",
    );

    HostSystemSummary {
        schema_version: 1,
        collected_at_ts: now_unix_seconds(),
        os: HostOperatingSystem {
            family: std::env::consts::OS.to_string(),
            name: platform.os_name,
            version: platform.os_version,
            kernel: platform.kernel,
        },
        architecture: std::env::consts::ARCH.to_string(),
        deployment: platform.deployment,
        memory: HostCapacity::new(
            platform.memory_total_bytes,
            platform.memory_available_bytes,
        ),
        storage: HostCapacity::new(storage_total_bytes, storage_available_bytes),
        uptime_seconds: platform.uptime_seconds,
        unavailable_fields,
    }
}

fn record_unavailable<T>(
    fields: &mut Vec<HostUnavailableField>,
    field: &'static str,
    value: &Option<T>,
    code: &'static str,
) {
    if value.is_none() {
        fields.push(HostUnavailableField { field, code });
    }
}

#[cfg(target_os = "linux")]
fn collect_host_platform_snapshot() -> HostPlatformSnapshot {
    let os_release = read_bounded_text(Path::new("/etc/os-release"));
    let (os_name, os_version) = os_release
        .as_deref()
        .map(parse_linux_os_release)
        .unwrap_or_default();
    let meminfo = read_bounded_text(Path::new("/proc/meminfo")).unwrap_or_default();
    let (memory_total_bytes, memory_available_bytes) = parse_linux_meminfo(&meminfo);
    let uptime_seconds = read_bounded_text(Path::new("/proc/uptime"))
        .as_deref()
        .and_then(parse_linux_uptime);
    let kernel = read_bounded_text(Path::new("/proc/sys/kernel/osrelease"))
        .and_then(normalize_host_value);
    let deployment = Some(detect_linux_deployment());

    HostPlatformSnapshot {
        os_name,
        os_version,
        kernel,
        deployment,
        memory_total_bytes,
        memory_available_bytes,
        uptime_seconds,
    }
}

#[cfg(target_os = "macos")]
fn collect_host_platform_snapshot() -> HostPlatformSnapshot {
    let system_version =
        read_bounded_text(Path::new("/System/Library/CoreServices/SystemVersion.plist"));
    let (os_name, os_version) = system_version
        .as_deref()
        .map(parse_macos_system_version)
        .unwrap_or_else(|| (Some("macOS".to_string()), None));
    let memory_total_bytes = bounded_command_output("/usr/sbin/sysctl", &["-n", "hw.memsize"])
        .and_then(|raw| raw.parse::<u64>().ok());
    let page_size = bounded_command_output("/usr/sbin/sysctl", &["-n", "hw.pagesize"])
        .and_then(|raw| raw.parse::<u64>().ok());
    let memory_available_bytes = match (
        bounded_command_output("/usr/bin/vm_stat", &[]),
        page_size,
    ) {
        (Some(vm_stat), Some(page_size)) => parse_macos_available_memory(&vm_stat, page_size),
        _ => None,
    };
    let uptime_seconds =
        bounded_command_output("/usr/sbin/sysctl", &["-n", "kern.boottime"])
            .as_deref()
            .and_then(parse_macos_boot_time)
            .and_then(|boot| u64::try_from(now_unix_seconds()).ok()?.checked_sub(boot));

    HostPlatformSnapshot {
        os_name,
        os_version,
        kernel: bounded_command_output("/usr/bin/uname", &["-r"]),
        deployment: Some("local_host".to_string()),
        memory_total_bytes,
        memory_available_bytes,
        uptime_seconds,
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn collect_host_platform_snapshot() -> HostPlatformSnapshot {
    HostPlatformSnapshot::default()
}

#[cfg(target_os = "linux")]
fn detect_linux_deployment() -> String {
    if Path::new("/.dockerenv").exists() {
        return "container".to_string();
    }
    let cgroup = read_bounded_text(Path::new("/proc/1/cgroup")).unwrap_or_default();
    if ["docker", "containerd", "kubepods", "lxc"]
        .iter()
        .any(|token| cgroup.contains(token))
    {
        "container".to_string()
    } else {
        "host".to_string()
    }
}

fn read_bounded_text(path: &Path) -> Option<String> {
    let file = fs::File::open(path).ok()?;
    if file.metadata().ok()?.len() > HOST_TEXT_LIMIT_BYTES {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(HOST_TEXT_LIMIT_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > HOST_TEXT_LIMIT_BYTES {
        return None;
    }
    String::from_utf8(bytes).ok()
}

#[cfg(target_os = "macos")]
fn bounded_command_output(program: &str, args: &[&str]) -> Option<String> {
    let output = StdCommand::new(program).args(args).output().ok()?;
    if !output.status.success() || output.stdout.len() > HOST_COMMAND_LIMIT_BYTES {
        return None;
    }
    normalize_host_value(String::from_utf8(output.stdout).ok()?)
}

fn normalize_host_value(value: String) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.chars().take(256).collect())
    }
}

fn parse_linux_os_release(text: &str) -> (Option<String>, Option<String>) {
    let mut values = BTreeMap::new();
    for line in text.lines().take(128) {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim().trim_matches('"').trim_matches('\'');
        if matches!(key, "NAME" | "PRETTY_NAME" | "VERSION" | "VERSION_ID") {
            values.insert(key, value.chars().take(256).collect::<String>());
        }
    }
    let name = values
        .get("NAME")
        .or_else(|| values.get("PRETTY_NAME"))
        .cloned()
        .filter(|value| !value.is_empty());
    let version = values
        .get("VERSION")
        .or_else(|| values.get("VERSION_ID"))
        .cloned()
        .filter(|value| !value.is_empty());
    (name, version)
}

fn parse_linux_meminfo(text: &str) -> (Option<u64>, Option<u64>) {
    let mut total = None;
    let mut available = None;
    for line in text.lines().take(256) {
        if let Some(raw) = line.strip_prefix("MemTotal:") {
            total = parse_kib_value(raw);
        } else if let Some(raw) = line.strip_prefix("MemAvailable:") {
            available = parse_kib_value(raw);
        }
    }
    (total, available)
}

fn parse_kib_value(raw: &str) -> Option<u64> {
    raw.split_whitespace()
        .next()?
        .parse::<u64>()
        .ok()
        .map(|value| value.saturating_mul(1024))
}

fn parse_linux_uptime(text: &str) -> Option<u64> {
    text.split_whitespace()
        .next()?
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite() && *value >= 0.0)
        .map(|value| value as u64)
}

#[cfg(any(target_os = "macos", test))]
fn parse_macos_system_version(text: &str) -> (Option<String>, Option<String>) {
    (
        parse_plist_string(text, "ProductName"),
        parse_plist_string(text, "ProductUserVisibleVersion")
            .or_else(|| parse_plist_string(text, "ProductVersion")),
    )
}

#[cfg(any(target_os = "macos", test))]
fn parse_plist_string(text: &str, key: &str) -> Option<String> {
    let marker = format!("<key>{key}</key>");
    let after_key = text.split_once(&marker)?.1;
    let after_open = after_key.split_once("<string>")?.1;
    let value = after_open.split_once("</string>")?.0.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.chars().take(256).collect())
    }
}

#[cfg(any(target_os = "macos", test))]
fn parse_macos_available_memory(text: &str, page_size: u64) -> Option<u64> {
    let free = parse_macos_vm_pages(text, "Pages free")?;
    let inactive = parse_macos_vm_pages(text, "Pages inactive").unwrap_or(0);
    let speculative = parse_macos_vm_pages(text, "Pages speculative").unwrap_or(0);
    Some(
        free.saturating_add(inactive)
            .saturating_add(speculative)
            .saturating_mul(page_size),
    )
}

#[cfg(any(target_os = "macos", test))]
fn parse_macos_vm_pages(text: &str, key: &str) -> Option<u64> {
    text.lines().take(256).find_map(|line| {
        let line = line.trim();
        let raw = line.strip_prefix(key)?.strip_prefix(':')?.trim();
        raw.trim_end_matches('.').parse::<u64>().ok()
    })
}

#[cfg(any(target_os = "macos", test))]
fn parse_macos_boot_time(text: &str) -> Option<u64> {
    let (_, raw) = text.split_once("sec =")?;
    raw.split([',', '}']).next()?.trim().parse::<u64>().ok()
}

fn nearest_existing_path(path: &Path) -> Option<PathBuf> {
    let mut current = path;
    loop {
        if current.exists() {
            return Some(current.to_path_buf());
        }
        current = current.parent()?;
    }
}

#[cfg(unix)]
fn storage_capacity_bytes(path: &Path) -> Option<(Option<u64>, Option<u64>)> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let path = CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut stats = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: `path` is a valid, NUL-terminated path and `stats` points to
    // writable memory for the duration of the libc call.
    if unsafe { libc::statvfs(path.as_ptr(), stats.as_mut_ptr()) } != 0 {
        return None;
    }
    // SAFETY: statvfs returned success and initialized the output structure.
    let stats = unsafe { stats.assume_init() };
    let block_size = if stats.f_frsize > 0 {
        stats.f_frsize
    } else {
        stats.f_bsize
    };
    let total = stats.f_blocks.saturating_mul(block_size);
    let available = stats.f_bavail.saturating_mul(block_size);
    Some((Some(total), Some(available)))
}

#[cfg(not(unix))]
fn storage_capacity_bytes(_path: &Path) -> Option<(Option<u64>, Option<u64>)> {
    None
}

fn now_unix_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_secs()).ok())
        .unwrap_or(0)
}
