const SYSTEM_DIAGNOSTIC_EXPORT_MAX_BYTES: usize = 256 * 1024;
const SYSTEM_DIAGNOSTIC_DEPENDENCY_LIMIT: usize = 128;

#[derive(Debug, Serialize)]
struct SystemDiagnosticExport {
    schema_version: u32,
    generated_at_ts: i64,
    redacted: bool,
    size_limit_bytes: usize,
    host: HostSystemSummary,
    dependency_summary: HostDependencySummary,
    dependencies: Vec<RedactedDependencyDiagnostic>,
    skills: DiagnosticSkillSummary,
}

#[derive(Debug, Serialize)]
struct RedactedDependencyDiagnostic {
    id: String,
    category: String,
    required: bool,
    installed: bool,
    version: Option<String>,
    installable: bool,
    status_code: String,
    runtime_state: String,
    runtime_reason_code: Option<String>,
}

#[derive(Debug, Serialize)]
struct DiagnosticSkillSummary {
    registry_generation: u64,
    registry_entries: usize,
    enabled: usize,
    disabled: usize,
}

async fn export_system_diagnostics(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(response) = require_ui_identity(&state, &headers) {
        return response;
    }

    let workspace_root = state.skill_rt.workspace_root.clone();
    let resource_status = state
        .skill_rt
        .skill_concurrency_gates
        .resource_broker()
        .status();
    let waiting_summary = host_task_waiting_summary(&state).ok();
    let dependency_context = host_dependency_runtime_context(&state);
    let skill_views = state.get_skill_views_snapshot();
    let registry_entries = skill_views
        .registry
        .as_deref()
        .map(|registry| registry.all_names().len())
        .unwrap_or_default();
    let enabled = skill_views.skills_list.len();
    let skill_summary = DiagnosticSkillSummary {
        registry_generation: skill_views.binding.registry_generation,
        registry_entries,
        enabled,
        disabled: registry_entries.saturating_sub(enabled),
    };

    let export = tokio::task::spawn_blocking(move || {
        let mut host =
            collect_host_system_summary(&workspace_root, resource_status, waiting_summary);
        host.redact_admin_details();
        let dependencies = collect_host_dependencies(&workspace_root, &dependency_context);
        SystemDiagnosticExport {
            schema_version: 1,
            generated_at_ts: now_unix_seconds(),
            redacted: true,
            size_limit_bytes: SYSTEM_DIAGNOSTIC_EXPORT_MAX_BYTES,
            host,
            dependency_summary: dependencies.summary,
            dependencies: dependencies
                .dependencies
                .into_iter()
                .take(SYSTEM_DIAGNOSTIC_DEPENDENCY_LIMIT)
                .map(|dependency| RedactedDependencyDiagnostic {
                    id: dependency.id,
                    category: dependency.category,
                    required: dependency.required,
                    installed: dependency.installed,
                    version: dependency.version,
                    installable: dependency.installable,
                    status_code: dependency.status_code,
                    runtime_state: dependency.runtime_state,
                    runtime_reason_code: dependency.runtime_reason_code,
                })
                .collect(),
            skills: skill_summary,
        }
    })
    .await;

    let Ok(export) = export else {
        return dependency_api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "system_diagnostic_collection_failed",
        );
    };
    let Ok(value) = serde_json::to_value(export) else {
        return dependency_api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "system_diagnostic_encode_failed",
        );
    };
    if serde_json::to_vec(&value)
        .map(|bytes| bytes.len() > SYSTEM_DIAGNOSTIC_EXPORT_MAX_BYTES)
        .unwrap_or(true)
    {
        return dependency_api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "system_diagnostic_size_limit_exceeded",
        );
    }

    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(value),
            error: None,
        }),
    )
}
