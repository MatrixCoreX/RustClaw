const AIPP_MEDIA_RECORD_LIMIT: usize = 50;
const AIPP_MEDIA_RECORD_SCAN_LIMIT: usize = 100_000;
const AIPP_MEDIA_RECORD_MAX_BYTES: u64 = 1024 * 1024;
const AIPP_MEDIA_STATE_MAX_BYTES: u64 = 1024 * 1024;
const AIPP_PREVIEW_MAX_BYTES: u64 = 16 * 1024 * 1024;
const AIPP_BUNDLE_ASSET_MAX_BYTES: u64 = 4 * 1024 * 1024;
const AIPP_TASK_ACTIVITY_LIMIT: usize = 50;
const AIPP_TASK_ACTIVITY_INPUT_MAX_CHARS: usize = 20_000;
const AIPP_TASK_ACTIVITY_RESULT_MAX_CHARS: usize = 65_536;
const AIPP_TASK_ACTIVITY_ERROR_MAX_CHARS: usize = 4_096;
const AIPP_TASK_ACTIVITY_ARTIFACT_LIMIT: usize = 64;
const AIPP_TASK_ACTIVITY_URL_LIMIT: usize = 16;
const AIPP_BUNDLE_CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' data: blob: https:; media-src 'self' blob:; font-src 'self'; connect-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'; frame-ancestors 'self'";

#[derive(Debug, Deserialize, Default)]
struct AippMediaQuery {
    limit: Option<usize>,
    before_sequence: Option<u64>,
    cursor_sequence: Option<u64>,
    sort_order: Option<String>,
    kind: Option<String>,
    platform: Option<String>,
    channel: Option<String>,
    status: Option<String>,
    query: Option<String>,
}

#[derive(Debug, Serialize)]
struct AippCatalogItem {
    skill_name: String,
    package_version: String,
    renderer: String,
    data_contract: String,
    icon: String,
    default_locale: String,
    titles: BTreeMap<String, String>,
    descriptions: BTreeMap<String, String>,
    installed: bool,
    entrypoint: Option<String>,
    bridge_capabilities: Vec<String>,
    task_channel_scope: Option<String>,
}

struct ActiveAippPackage {
    manifest: skill_sdk::PackageManifest,
    aipp: skill_sdk::AippSpec,
    package_root: PathBuf,
}

fn active_aipp_package(
    state: &AppState,
    skill_name: &str,
) -> Result<Option<ActiveAippPackage>, String> {
    let registry = state
        .get_skills_registry()
        .ok_or_else(|| "aipp_registry_unavailable".to_string())?;
    if !state.get_skills_list().contains(skill_name) || registry.get(skill_name).is_none() {
        return Ok(None);
    }

    let binding = state
        .get_skill_views_snapshot()
        .binding
        .admission_bindings
        .get(skill_name)
        .cloned();
    let (manifest, package_root) = if let Some(binding) = binding {
        let store = skill_sdk::InstallReceiptStore::new(skill_package_root(state));
        let pointer = store
            .current_pointer(skill_name)
            .map_err(|_| "aipp_install_pointer_invalid".to_string())?;
        if pointer.version != binding.version
            || pointer.receipt_digest != binding.install_receipt_digest
        {
            return Err("aipp_generation_binding_mismatch".to_string());
        }
        let verified = store
            .verified_current_metadata(skill_name)
            .map_err(|_| "aipp_install_receipt_invalid".to_string())?;
        if verified.manifest.aipp.is_none() {
            return Ok(None);
        }
        let manifest_digest = verified
            .manifest
            .digest()
            .map_err(|_| "aipp_manifest_digest_failed".to_string())?;
        let receipt_digest = verified
            .receipt
            .digest()
            .map_err(|_| "aipp_receipt_digest_failed".to_string())?;
        if manifest_digest != binding.manifest_digest
            || receipt_digest != binding.install_receipt_digest
            || verified.manifest.package.version != binding.version
        {
            return Err("aipp_generation_binding_mismatch".to_string());
        }
        let package_root = verified.install_dir;
        (verified.manifest, package_root)
    } else {
        let Some(manifest_path) = registry
            .package_manifest_path(skill_name)
            .map(|relative| state.skill_rt.workspace_root.join(relative))
        else {
            return Ok(None);
        };
        let manifest = skill_sdk::PackageManifest::load(&manifest_path)
            .map_err(|_| "aipp_manifest_invalid".to_string())?;
        let package_root = manifest_path
            .parent()
            .ok_or_else(|| "aipp_install_path_invalid".to_string())?
            .to_path_buf();
        (manifest, package_root)
    };
    let Some(aipp) = manifest.aipp.clone() else {
        return Ok(None);
    };
    Ok(Some(ActiveAippPackage {
        manifest,
        aipp,
        package_root,
    }))
}

fn aipp_admission_service(
    state: &AppState,
) -> Result<crate::skill_admission::SkillAdmissionService, String> {
    let config = claw_core::config::AppConfig::load(&state.reload_ctx.config_path_for_reload)
        .map_err(|_| "aipp_runtime_config_invalid".to_string())?;
    crate::skill_admission::SkillAdmissionService::from_config(
        &state.skill_rt.workspace_root,
        &config,
    )
    .map_err(|_| "aipp_install_state_unavailable".to_string())
}

fn aipp_is_installed(state: &AppState, skill_name: &str) -> bool {
    aipp_admission_service(state)
        .and_then(|service| {
            service
                .aipp_is_installed(skill_name)
                .map_err(|_| "aipp_install_state_unavailable".to_string())
        })
        .unwrap_or(false)
}

async fn get_aipp_catalog(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(response) = require_ui_admin(&state, &headers) {
        return response;
    }
    let mut apps = Vec::new();
    let admission = aipp_admission_service(&state).ok();
    let mut names = state.get_skills_list().iter().cloned().collect::<Vec<_>>();
    names.sort_unstable();
    for name in names {
        match active_aipp_package(&state, &name) {
            Ok(Some(active)) => {
                let installed = admission
                    .as_ref()
                    .and_then(|service| service.aipp_is_installed(&name).ok())
                    .unwrap_or(false);
                let manifest = active.manifest;
                let aipp = active.aipp;
                apps.push(AippCatalogItem {
                    skill_name: name,
                    package_version: manifest.package.version,
                    renderer: aipp.renderer,
                    data_contract: aipp.data_contract,
                    icon: aipp.icon,
                    default_locale: aipp.default_locale,
                    titles: aipp.titles,
                    descriptions: aipp.descriptions,
                    installed,
                    entrypoint: aipp.entrypoint,
                    bridge_capabilities: aipp.bridge_capabilities,
                    task_channel_scope: aipp.task_channel_scope,
                })
            }
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(skill = %name, error = %error, "AiPP catalog entry rejected");
            }
        }
    }
    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(json!({ "schema_version": 1, "apps": apps })),
            error: None,
        }),
    )
}

async fn set_aipp_install_state(
    state: &AppState,
    skill_name: &str,
    installed: bool,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if !matches!(active_aipp_package(state, skill_name), Ok(Some(_))) {
        return aipp_api_error(StatusCode::NOT_FOUND, "aipp_not_available");
    }
    let service = match aipp_admission_service(state) {
        Ok(service) => service,
        Err(error) => return aipp_api_error(StatusCode::INTERNAL_SERVER_ERROR, &error),
    };
    match service.set_aipp_installed(skill_name, installed) {
        Ok(()) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(json!({
                    "schema_version": 1,
                    "skill_name": skill_name,
                    "installed": installed,
                })),
                error: None,
            }),
        ),
        Err(_) => aipp_api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "aipp_install_state_update_failed",
        ),
    }
}

async fn install_aipp(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(skill_name): AxumPath<String>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(response) = require_ui_admin(&state, &headers) {
        return response;
    }
    set_aipp_install_state(&state, &skill_name, true).await
}

async fn remove_aipp(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(skill_name): AxumPath<String>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(response) = require_ui_admin(&state, &headers) {
        return response;
    }
    set_aipp_install_state(&state, &skill_name, false).await
}

fn aipp_media_item(record: &Value) -> Option<Value> {
    let sequence = record.get("global_sequence")?.as_u64()?;
    let kind = record.get("kind")?.as_str()?;
    if !matches!(kind, "video" | "image") {
        return None;
    }
    let https_url = |key: &str| {
        record
            .get(key)
            .and_then(Value::as_str)
            .filter(|value| {
                value.len() <= 4096
                    && value.starts_with("https://")
                    && !value.chars().any(char::is_control)
            })
    };
    let source_url = if kind == "video" {
        https_url("video_page_url")
    } else {
        https_url("source_page_url")
    };
    let image_url = if kind == "image" {
        https_url("image_url")
    } else {
        None
    };
    let preview_available = ["cover_screenshot_path", "image_screenshot_path"]
        .iter()
        .any(|key| {
            record
                .get(key)
                .and_then(Value::as_str)
                .is_some_and(|value| !value.trim().is_empty())
        });
    Some(json!({
        "schema_version": 1,
        "global_sequence": sequence,
        "sequence": record.get("sequence").and_then(Value::as_u64),
        "post_sequence": record.get("post_sequence").and_then(Value::as_u64),
        "image_sequence": record.get("image_sequence").and_then(Value::as_u64),
        "kind": kind,
        "platform": bounded_aipp_text(record.get("platform"), 64),
        "source_mode": bounded_aipp_text(record.get("source_mode"), 64),
        "search_keyword": bounded_aipp_text(record.get("search_keyword"), 512),
        "title": bounded_aipp_text(record.get("title"), 512),
        "platform_text": bounded_aipp_text(record.get("platform_text"), 32_768),
        "source_url": source_url,
        "image_url": image_url,
        "preview_available": preview_available,
        "discovered_at": bounded_aipp_optional_text(record.get("discovered_at"), 64),
        "published_at": bounded_aipp_optional_text(record.get("published_at"), 64),
        "publication_text": bounded_aipp_optional_text(record.get("publication_text"), 128),
        "engagement": aipp_engagement(record),
    }))
}

fn aipp_engagement(record: &Value) -> Option<Value> {
    let engagement = record.get("engagement")?;
    if engagement.get("schema_version").and_then(Value::as_u64) != Some(1)
        || engagement.get("platform").and_then(Value::as_str)
            != record.get("platform").and_then(Value::as_str)
    {
        return None;
    }
    let mut metrics = BTreeMap::new();
    for name in ["views", "likes", "comments", "favorites", "shares"] {
        let Some(metric) = engagement.pointer(&format!("/metrics/{name}")) else {
            continue;
        };
        let Some(display) = metric
            .get("display")
            .and_then(Value::as_str)
            .filter(|value| {
                !value.is_empty()
                    && value.chars().count() <= 32
                    && !value.chars().any(char::is_control)
            })
        else {
            continue;
        };
        metrics.insert(
            name.to_string(),
            json!({
                "display": display,
                "value": metric.get("value").and_then(Value::as_u64),
            }),
        );
    }
    Some(json!({
        "schema_version": 1,
        "platform": engagement.get("platform").and_then(Value::as_str),
        "captured_at": bounded_aipp_optional_text(engagement.get("captured_at"), 64),
        "metrics": metrics,
    }))
}

fn bounded_aipp_text(value: Option<&Value>, max_chars: usize) -> String {
    value
        .and_then(Value::as_str)
        .map(|value| value.chars().take(max_chars).collect())
        .unwrap_or_default()
}

fn bounded_aipp_optional_text(value: Option<&Value>, max_chars: usize) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(|value| value.chars().take(max_chars).collect())
}

fn valid_task_activity_filter(value: Option<&str>, allowed: &[&str]) -> Result<(), String> {
    if value.is_none_or(|value| allowed.contains(&value)) {
        Ok(())
    } else {
        Err("aipp_task_activity_filter_invalid".to_string())
    }
}

fn task_activity_urls(input: &str) -> Vec<String> {
    static URL_CANDIDATE: OnceLock<regex::Regex> = OnceLock::new();
    let pattern = URL_CANDIDATE.get_or_init(|| {
        regex::Regex::new(r#"https?://[^\s<>\"']+"#).expect("static URL pattern")
    });
    let mut urls = BTreeSet::new();
    for candidate in pattern.find_iter(input).map(|value| {
        value
            .as_str()
            .trim_end_matches([
                '.', ',', ';', ':', '!', '?', ')', ']', '}', '。', '，', '；', '：', '！',
                '？', '）', '】', '》',
            ])
    }) {
        if candidate.len() > 4_096 || candidate.chars().any(char::is_control) {
            continue;
        }
        let Ok(parsed) = reqwest::Url::parse(candidate) else {
            continue;
        };
        if matches!(parsed.scheme(), "http" | "https")
            && parsed.host_str().is_some()
            && parsed.username().is_empty()
            && parsed.password().is_none()
        {
            urls.insert(candidate.to_string());
        }
        if urls.len() >= AIPP_TASK_ACTIVITY_URL_LIMIT {
            break;
        }
    }
    urls.into_iter().collect()
}

fn task_activity_artifacts(task_id: &str, result: &Value) -> Vec<Value> {
    let route_prefix = format!("/v1/tasks/{task_id}/artifacts/");
    result
        .get("artifacts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(AIPP_TASK_ACTIVITY_ARTIFACT_LIMIT)
        .filter_map(|artifact| {
            let id = bounded_aipp_optional_text(artifact.get("id"), 256)?;
            let filename = bounded_aipp_optional_text(artifact.get("filename"), 512)?;
            let bounded_route = |key: &str| {
                artifact
                    .get(key)
                    .and_then(Value::as_str)
                    .filter(|value| {
                        value.len() <= 4_096
                            && value.starts_with(&route_prefix)
                            && !value.chars().any(char::is_control)
                    })
            };
            let download_url = bounded_route("download_url")?;
            Some(json!({
                "schema_version": 1,
                "id": id,
                "filename": filename,
                "kind": bounded_aipp_text(artifact.get("kind"), 32),
                "mime_type": bounded_aipp_text(artifact.get("mime_type"), 128),
                "size_bytes": artifact.get("size_bytes").and_then(Value::as_u64),
                "download_url": download_url,
                "preview_url": bounded_route("preview_url"),
            }))
        })
        .collect()
}

fn task_activity_item(
    sequence: u64,
    task_id: &str,
    channel: &str,
    status: &str,
    payload_json: &str,
    result_json: Option<&str>,
    error_text: Option<&str>,
    created_at: &str,
    updated_at: &str,
    event_at_ms: u64,
    action_refs: Option<&str>,
) -> Value {
    let payload = serde_json::from_str::<Value>(payload_json).unwrap_or_else(|_| json!({}));
    let result = result_json
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        .unwrap_or_else(|| json!({}));
    let input_text = bounded_aipp_text(
        payload.get("text"),
        AIPP_TASK_ACTIVITY_INPUT_MAX_CHARS,
    );
    let mut actions = action_refs
        .unwrap_or_default()
        .split(',')
        .filter(|value| !value.is_empty() && value.len() <= 256)
        .map(str::to_string)
        .collect::<Vec<_>>();
    actions.sort_unstable();
    actions.dedup();
    json!({
        "schema_version": 1,
        "sequence": sequence,
        "task_id": task_id,
        "channel": channel,
        "status": status,
        "actions": actions,
        "input_text": input_text,
        "result_text": bounded_aipp_text(
            result.get("text"),
            AIPP_TASK_ACTIVITY_RESULT_MAX_CHARS,
        ),
        "error_text": error_text
            .map(|value| value.chars().take(AIPP_TASK_ACTIVITY_ERROR_MAX_CHARS).collect::<String>()),
        "source_urls": task_activity_urls(&input_text),
        "artifacts": task_activity_artifacts(task_id, &result),
        "created_at": created_at,
        "updated_at": updated_at,
        "event_at_ms": event_at_ms,
    })
}

fn read_aipp_task_activity_page(
    db: &rusqlite::Connection,
    skill_name: &str,
    task_channel_scope: &str,
    cleared_through_event_ms: u64,
    query: &AippMediaQuery,
) -> Result<Value, String> {
    let communication_only = match task_channel_scope {
        "all" => false,
        "communication" => true,
        _ => return Err("aipp_task_activity_scope_invalid".to_string()),
    };
    let allowed_channels: &[&str] = if communication_only {
        &["telegram", "whatsapp", "feishu", "lark", "wechat"]
    } else {
        &["telegram", "whatsapp", "ui", "feishu", "lark", "wechat"]
    };
    valid_task_activity_filter(
        query.channel.as_deref(),
        allowed_channels,
    )?;
    valid_task_activity_filter(
        query.status.as_deref(),
        &["queued", "running", "succeeded", "failed", "canceled", "timeout"],
    )?;
    let sort_order = aipp_media_sort_order(query)?;
    let (cursor_operator, order) = if sort_order == "oldest" {
        (">", "ASC")
    } else {
        ("<", "DESC")
    };
    let limit = query.limit.unwrap_or(20).clamp(1, AIPP_TASK_ACTIVITY_LIMIT);
    let needle = query
        .query
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if needle.is_some_and(|value| value.chars().count() > 200) {
        return Err("aipp_task_activity_query_invalid".to_string());
    }
    let sql = format!(
        r#"
        WITH filtered AS MATERIALIZED (
            SELECT tasks.rowid AS activity_sequence,
                   tasks.task_id,
                   tasks.channel,
                   tasks.status,
                   tasks.payload_json,
                   tasks.result_json,
                   tasks.error_text,
                   tasks.created_at,
                   tasks.updated_at
              FROM tasks
             WHERE (:channel IS NULL OR tasks.channel = :channel)
               AND (
                    :communication_only = 0
                    OR tasks.channel IN ('telegram', 'whatsapp', 'feishu', 'lark', 'wechat')
               )
               AND (:status IS NULL OR tasks.status = :status)
               AND (
                    :needle IS NULL
                    OR INSTR(
                        LOWER(CASE WHEN json_valid(tasks.payload_json)
                            THEN COALESCE(json_extract(tasks.payload_json, '$.text'), '')
                            ELSE '' END),
                        LOWER(:needle)
                    ) > 0
                    OR INSTR(
                        LOWER(CASE WHEN json_valid(tasks.result_json)
                            THEN COALESCE(json_extract(tasks.result_json, '$.text'), '')
                            ELSE '' END),
                        LOWER(:needle)
                    ) > 0
               )
               AND (
                    EXISTS (
                        SELECT 1
                          FROM task_event_stream
                         WHERE task_event_stream.task_id = tasks.task_id
                           AND json_extract(task_event_stream.event_json, '$.event_kind') = 'tool_finished'
                           AND json_extract(task_event_stream.event_json, '$.payload.skill') = :skill_name
                           AND task_event_stream.created_at_ms > :cleared_through_event_ms
                    )
                    OR EXISTS (
                        SELECT 1
                          FROM task_event_archive
                         WHERE task_event_archive.task_id = tasks.task_id
                           AND json_extract(task_event_archive.event_json, '$.event_kind') = 'tool_finished'
                           AND json_extract(task_event_archive.event_json, '$.payload.skill') = :skill_name
                           AND task_event_archive.created_at_ms > :cleared_through_event_ms
                    )
               )
               AND (:cursor IS NULL OR tasks.rowid {cursor_operator} :cursor)
             ORDER BY tasks.rowid {order}
             LIMIT :limit
        ), skill_events AS (
            SELECT task_event_stream.task_id,
                   task_event_stream.created_at_ms,
                   json_extract(task_event_stream.event_json, '$.payload.requested_action_ref') AS action_ref
              FROM task_event_stream
              JOIN filtered ON filtered.task_id = task_event_stream.task_id
             WHERE json_extract(task_event_stream.event_json, '$.event_kind') = 'tool_finished'
               AND json_extract(task_event_stream.event_json, '$.payload.skill') = :skill_name
               AND task_event_stream.created_at_ms > :cleared_through_event_ms
            UNION ALL
            SELECT task_event_archive.task_id,
                   task_event_archive.created_at_ms,
                   json_extract(task_event_archive.event_json, '$.payload.requested_action_ref') AS action_ref
              FROM task_event_archive
              JOIN filtered ON filtered.task_id = task_event_archive.task_id
             WHERE json_extract(task_event_archive.event_json, '$.event_kind') = 'tool_finished'
               AND json_extract(task_event_archive.event_json, '$.payload.skill') = :skill_name
               AND task_event_archive.created_at_ms > :cleared_through_event_ms
        ), activity AS (
            SELECT task_id,
                   MAX(created_at_ms) AS event_at_ms,
                   GROUP_CONCAT(DISTINCT action_ref) AS action_refs
              FROM skill_events
             GROUP BY task_id
        )
        SELECT filtered.activity_sequence,
               filtered.task_id,
               filtered.channel,
               filtered.status,
               filtered.payload_json,
               filtered.result_json,
               filtered.error_text,
               filtered.created_at,
               filtered.updated_at,
               activity.event_at_ms,
               activity.action_refs
          FROM filtered
          JOIN activity ON activity.task_id = filtered.task_id
         ORDER BY activity_sequence {order}
        "#
    );
    let mut statement = db
        .prepare(&sql)
        .map_err(|_| "aipp_task_activity_query_failed".to_string())?;
    let requested = (limit + 1) as i64;
    let mut rows = statement
        .query(rusqlite::named_params! {
            ":skill_name": skill_name,
            ":cleared_through_event_ms": cleared_through_event_ms as i64,
            ":communication_only": i64::from(communication_only),
            ":channel": query.channel.as_deref(),
            ":status": query.status.as_deref(),
            ":needle": needle,
            ":cursor": query.cursor_sequence.or(query.before_sequence).map(|value| value as i64),
            ":limit": requested,
        })
        .map_err(|_| "aipp_task_activity_query_failed".to_string())?;
    let mut items = Vec::with_capacity(limit + 1);
    while let Some(row) = rows
        .next()
        .map_err(|_| "aipp_task_activity_query_failed".to_string())?
    {
        items.push(task_activity_item(
            row.get::<_, u64>(0).unwrap_or_default(),
            &row.get::<_, String>(1).unwrap_or_default(),
            &row.get::<_, String>(2).unwrap_or_default(),
            &row.get::<_, String>(3).unwrap_or_default(),
            &row.get::<_, String>(4).unwrap_or_default(),
            row.get::<_, Option<String>>(5).ok().flatten().as_deref(),
            row.get::<_, Option<String>>(6).ok().flatten().as_deref(),
            &row.get::<_, String>(7).unwrap_or_default(),
            &row.get::<_, String>(8).unwrap_or_default(),
            row.get::<_, u64>(9).unwrap_or_default(),
            row.get::<_, Option<String>>(10).ok().flatten().as_deref(),
        ));
    }
    let has_more = items.len() > limit;
    if has_more {
        items.truncate(limit);
    }
    let next_cursor_sequence = has_more
        .then(|| items.last()?.get("sequence")?.as_u64())
        .flatten();
    let updated_at_ms = items
        .iter()
        .filter_map(|item| item.get("event_at_ms").and_then(Value::as_u64))
        .max();
    Ok(json!({
        "schema_version": 1,
        "page_item_count": items.len(),
        "items": items,
        "sort_order": sort_order,
        "next_cursor_sequence": next_cursor_sequence,
        "updated_at_ms": updated_at_ms,
    }))
}

fn record_matches_aipp_query(record: &Value, query: &AippMediaQuery) -> bool {
    if query.kind.as_deref().is_some_and(|kind| {
        !matches!(kind, "video" | "image")
            || record.get("kind").and_then(Value::as_str) != Some(kind)
    }) {
        return false;
    }
    if query.platform.as_deref().is_some_and(|platform| {
        platform.len() > 64
            || record.get("platform").and_then(Value::as_str) != Some(platform)
    }) {
        return false;
    }
    let Some(needle) = query
        .query
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return true;
    };
    if needle.chars().count() > 200 {
        return false;
    }
    let needle = needle.to_lowercase();
    ["title", "platform_text", "search_keyword"]
        .iter()
        .filter_map(|key| record.get(key).and_then(Value::as_str))
        .any(|value| value.to_lowercase().contains(&needle))
}

fn aipp_media_sort_order(query: &AippMediaQuery) -> Result<&'static str, String> {
    match query.sort_order.as_deref().unwrap_or("newest") {
        "newest" => Ok("newest"),
        "oldest" => Ok("oldest"),
        _ => Err("aipp_media_sort_order_invalid".to_string()),
    }
}

fn read_aipp_media_page(root: &Path, query: &AippMediaQuery) -> Result<Value, String> {
    let records_root = root.join("records");
    let mut names = match fs::read_dir(&records_root) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .filter_map(|entry| entry.file_name().to_str().map(str::to_string))
            .filter(|name| name.len() == 17 && name.ends_with(".json"))
            .collect::<Vec<_>>(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(_) => return Err("aipp_media_storage_read_failed".to_string()),
    };
    if names.len() > AIPP_MEDIA_RECORD_SCAN_LIMIT {
        return Err("aipp_media_record_limit_exceeded".to_string());
    }
    let sort_order = aipp_media_sort_order(query)?;
    if sort_order == "oldest" {
        names.sort_unstable();
    } else {
        names.sort_unstable_by(|left, right| right.cmp(left));
    }
    let limit = query.limit.unwrap_or(24).clamp(1, AIPP_MEDIA_RECORD_LIMIT);
    let has_filter = query.kind.is_some()
        || query.platform.is_some()
        || query
            .query
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty());
    let mut items = Vec::with_capacity(limit + 1);
    let mut matching_total = if has_filter { 0 } else { names.len() };
    let cursor_sequence = query.cursor_sequence.or_else(|| {
        (sort_order == "newest")
            .then_some(query.before_sequence)
            .flatten()
    });
    for name in names {
        let file_sequence = name
            .strip_suffix(".json")
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or_default();
        if !has_filter && cursor_sequence.is_some_and(|cursor| {
            if sort_order == "oldest" {
                file_sequence <= cursor
            } else {
                file_sequence >= cursor
            }
        }) {
            continue;
        }
        let record_path = records_root.join(&name);
        if fs::metadata(&record_path)
            .map(|metadata| metadata.len() > AIPP_MEDIA_RECORD_MAX_BYTES)
            .unwrap_or(true)
        {
            continue;
        }
        let raw = match fs::read(record_path) {
            Ok(raw) => raw,
            Err(_) => continue,
        };
        if raw.len() as u64 > AIPP_MEDIA_RECORD_MAX_BYTES {
            continue;
        }
        let record: Value = match serde_json::from_slice(&raw) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let sequence = record
            .get("global_sequence")
            .and_then(Value::as_u64)
            .unwrap_or_default();
        if sequence == 0 || (has_filter && !record_matches_aipp_query(&record, query)) {
            continue;
        }
        if has_filter {
            matching_total = matching_total.saturating_add(1);
        }
        if cursor_sequence.is_some_and(|cursor| {
            if sort_order == "oldest" {
                sequence <= cursor
            } else {
                sequence >= cursor
            }
        }) {
            continue;
        }
        if items.len() <= limit {
            if let Some(item) = aipp_media_item(&record) {
                items.push(item);
            }
        }
        if !has_filter && items.len() > limit {
            break;
        }
    }
    let has_more = items.len() > limit;
    if has_more {
        items.truncate(limit);
    }
    let next_cursor_sequence = has_more
        .then(|| items.last()?.get("global_sequence")?.as_u64())
        .flatten();
    let next_before_sequence =
        (sort_order == "newest").then_some(next_cursor_sequence).flatten();
    let state_path = root.join("state.json");
    let state = fs::metadata(&state_path)
        .ok()
        .filter(|metadata| metadata.len() <= AIPP_MEDIA_STATE_MAX_BYTES)
        .and_then(|_| fs::read(state_path).ok())
        .filter(|raw| raw.len() as u64 <= AIPP_MEDIA_STATE_MAX_BYTES)
        .and_then(|raw| serde_json::from_slice::<Value>(&raw).ok())
        .unwrap_or_else(|| json!({}));
    let platform_states = state
        .get("platforms")
        .and_then(Value::as_object)
        .map(|platforms| {
            platforms
                .iter()
                .filter(|(name, _)| name.len() <= 64)
                .take(32)
                .map(|(name, value)| {
                    let state = bounded_aipp_text(value.get("state"), 64);
                    (
                        name.clone(),
                        json!({
                            "state": if state.is_empty() { "unknown" } else { &state },
                            "enabled": value.get("enabled").and_then(Value::as_bool).unwrap_or(false),
                            "paused": value.get("paused").and_then(Value::as_bool).unwrap_or(false),
                        }),
                    )
                })
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    let active_run = state
        .get("active_run")
        .filter(|value| !value.is_null())
        .map(|run| {
            let platforms = run
                .get("platforms")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .filter(|value| value.len() <= 64)
                .take(16)
                .collect::<Vec<_>>();
            let count = |key: &str| {
                run.pointer(&format!("/counts/{key}"))
                    .and_then(Value::as_u64)
                    .unwrap_or_default()
            };
            json!({
                "run_id": bounded_aipp_optional_text(run.get("run_id"), 128),
                "platforms": platforms,
                "lifecycle_state": bounded_aipp_optional_text(run.get("lifecycle_state"), 64),
                "started_at": bounded_aipp_optional_text(run.get("started_at"), 64),
                "heartbeat_at": bounded_aipp_optional_text(run.get("heartbeat_at"), 64),
                "counts": {
                    "items": count("items"),
                    "videos": count("videos"),
                    "images": count("images"),
                    "duplicates": count("duplicates"),
                    "failures": count("failures"),
                },
            })
        });
    Ok(json!({
        "schema_version": 1,
        "items": items,
        "matching_total": matching_total,
        "sort_order": sort_order,
        "next_cursor_sequence": next_cursor_sequence,
        "next_before_sequence": next_before_sequence,
        "platform_states": platform_states,
        "active_run": active_run,
        "updated_at": bounded_aipp_optional_text(state.get("updated_at"), 64),
    }))
}

async fn get_aipp_items(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(skill_name): AxumPath<String>,
    Query(query): Query<AippMediaQuery>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(response) = require_ui_admin(&state, &headers) {
        return response;
    }
    let active = match active_aipp_package(&state, &skill_name) {
        Ok(Some(active)) if aipp_is_installed(&state, &skill_name) => active,
        _ => return aipp_api_error(StatusCode::NOT_FOUND, "aipp_not_available"),
    };
    if active.aipp.data_contract == "skill_task_activity_v1" {
        let task_channel_scope = active
            .aipp
            .task_channel_scope
            .clone()
            .unwrap_or_else(|| "all".to_string());
        let cleared_through_event_ms = match aipp_admission_service(&state)
            .and_then(|service| {
                service
                    .aipp_cleared_through_event_ms(&skill_name)
                    .map_err(|_| "aipp_view_state_unavailable".to_string())
            })
        {
            Ok(sequence) => sequence,
            Err(_) => {
                return aipp_api_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "aipp_view_state_unavailable",
                )
            }
        };
        let query_state = state.clone();
        let query_skill = skill_name.clone();
        let page = tokio::task::spawn_blocking(move || {
            let db = query_state
                .core
                .db
                .get()
                .map_err(|_| "aipp_task_activity_database_unavailable".to_string())?;
            read_aipp_task_activity_page(
                &db,
                &query_skill,
                &task_channel_scope,
                cleared_through_event_ms,
                &query,
            )
        })
        .await;
        return match page {
            Ok(Ok(data)) => (
                StatusCode::OK,
                Json(ApiResponse {
                    ok: true,
                    data: Some(data),
                    error: None,
                }),
            ),
            Ok(Err(error)) => {
                let status = match error.as_str() {
                    "aipp_task_activity_filter_invalid"
                    | "aipp_task_activity_query_invalid"
                    | "aipp_media_sort_order_invalid" => StatusCode::BAD_REQUEST,
                    _ => StatusCode::INTERNAL_SERVER_ERROR,
                };
                aipp_api_error(status, &error)
            }
            Err(_) => {
                aipp_api_error(StatusCode::INTERNAL_SERVER_ERROR, "aipp_read_task_failed")
            }
        };
    }
    if active.aipp.data_contract != "media_collection_v1" {
        return aipp_api_error(StatusCode::NOT_FOUND, "aipp_not_available");
    }
    let root = match state.core.skill_storage.resolved_directory_path(&skill_name) {
        Ok(path) => path,
        Err(_) => return aipp_api_error(StatusCode::BAD_REQUEST, "aipp_storage_invalid"),
    };
    let page = tokio::task::spawn_blocking(move || read_aipp_media_page(&root, &query)).await;
    match page {
        Ok(Ok(data)) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(data),
                error: None,
            }),
        ),
        Ok(Err(error)) => aipp_api_error(StatusCode::INTERNAL_SERVER_ERROR, &error),
        Err(_) => aipp_api_error(StatusCode::INTERNAL_SERVER_ERROR, "aipp_read_task_failed"),
    }
}

async fn clear_aipp_items(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(skill_name): AxumPath<String>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(response) = require_ui_admin(&state, &headers) {
        return response;
    }
    let active = match active_aipp_package(&state, &skill_name) {
        Ok(Some(active)) if aipp_is_installed(&state, &skill_name) => active,
        _ => return aipp_api_error(StatusCode::NOT_FOUND, "aipp_not_available"),
    };
    if active.aipp.data_contract != "skill_task_activity_v1" {
        return aipp_api_error(StatusCode::BAD_REQUEST, "aipp_items_clear_unsupported");
    }
    let cleared_through_event_ms = {
        let db = match state.core.db.get() {
            Ok(db) => db,
            Err(_) => {
                return aipp_api_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "aipp_items_clear_database_unavailable",
                )
            }
        };
        match db.query_row(
            "SELECT COALESCE(MAX(created_at_ms), 0)
               FROM (
                    SELECT created_at_ms
                      FROM task_event_stream
                     WHERE json_extract(event_json, '$.event_kind') = 'tool_finished'
                       AND json_extract(event_json, '$.payload.skill') = ?1
                    UNION ALL
                    SELECT created_at_ms
                      FROM task_event_archive
                     WHERE json_extract(event_json, '$.event_kind') = 'tool_finished'
                       AND json_extract(event_json, '$.payload.skill') = ?1
               )",
            rusqlite::params![skill_name],
            |row| row.get::<_, u64>(0),
        ) {
            Ok(sequence) => sequence,
            Err(_) => {
                return aipp_api_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "aipp_items_clear_database_unavailable",
                )
            }
        }
    };
    let service = match aipp_admission_service(&state) {
        Ok(service) => service,
        Err(_) => {
            return aipp_api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "aipp_view_state_unavailable",
            )
        }
    };
    if service
        .set_aipp_cleared_through_event_ms(&skill_name, cleared_through_event_ms)
        .is_err()
    {
        return aipp_api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "aipp_items_clear_state_update_failed",
        );
    }
    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(json!({
                "schema_version": 1,
                "skill_name": skill_name,
                "cleared_through_event_ms": cleared_through_event_ms,
                "source_records_preserved": true,
            })),
            error: None,
        }),
    )
}

fn resolve_aipp_preview(root: &Path, sequence: u64) -> Result<(PathBuf, &'static str), String> {
    if sequence == 0 {
        return Err("aipp_preview_not_found".to_string());
    }
    let record_path = root
        .join("records")
        .join(format!("{sequence:012}.json"));
    let record: Value = serde_json::from_slice(
        &fs::read(record_path).map_err(|_| "aipp_preview_not_found".to_string())?,
    )
    .map_err(|_| "aipp_preview_record_invalid".to_string())?;
    let relative = ["cover_screenshot_path", "image_screenshot_path"]
        .iter()
        .find_map(|key| record.get(key).and_then(Value::as_str))
        .ok_or_else(|| "aipp_preview_not_found".to_string())?;
    let relative_path = Path::new(relative);
    if relative_path.is_absolute()
        || relative_path.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
    {
        return Err("aipp_preview_path_invalid".to_string());
    }
    let exports_root = fs::canonicalize(root.join("exports"))
        .map_err(|_| "aipp_preview_not_found".to_string())?;
    let preview = fs::canonicalize(exports_root.join(relative_path))
        .map_err(|_| "aipp_preview_not_found".to_string())?;
    let metadata = fs::metadata(&preview).map_err(|_| "aipp_preview_not_found".to_string())?;
    if !preview.starts_with(&exports_root)
        || !metadata.is_file()
        || metadata.len() > AIPP_PREVIEW_MAX_BYTES
    {
        return Err("aipp_preview_path_invalid".to_string());
    }
    let content_type = match preview
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        _ => return Err("aipp_preview_type_unsupported".to_string()),
    };
    Ok((preview, content_type))
}

async fn get_aipp_media_preview(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath((skill_name, sequence)): AxumPath<(String, u64)>,
) -> axum::response::Response {
    if let Err(response) = require_ui_admin(&state, &headers) {
        return response.into_response();
    }
    let active = active_aipp_package(&state, &skill_name);
    if !aipp_is_installed(&state, &skill_name)
        || !matches!(active, Ok(Some(ref active)) if active.aipp.data_contract == "media_collection_v1")
    {
        return aipp_api_error(StatusCode::NOT_FOUND, "aipp_not_available").into_response();
    }
    let root = match state.core.skill_storage.resolved_directory_path(&skill_name) {
        Ok(path) => path,
        Err(_) => {
            return aipp_api_error(StatusCode::BAD_REQUEST, "aipp_storage_invalid")
                .into_response()
        }
    };
    let resolved = tokio::task::spawn_blocking(move || resolve_aipp_preview(&root, sequence)).await;
    let (path, content_type) = match resolved {
        Ok(Ok(value)) => value,
        Ok(Err(error)) => {
            return aipp_api_error(StatusCode::NOT_FOUND, &error).into_response()
        }
        Err(_) => {
            return aipp_api_error(StatusCode::INTERNAL_SERVER_ERROR, "aipp_read_task_failed")
                .into_response()
        }
    };
    let bytes = match tokio::fs::read(path).await {
        Ok(value) => value,
        Err(_) => {
            return aipp_api_error(StatusCode::NOT_FOUND, "aipp_preview_not_found")
                .into_response()
        }
    };
    let mut response = axum::response::Response::new(axum::body::Body::from(bytes));
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static(content_type),
    );
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("private, max-age=300"),
    );
    response.headers_mut().insert(
        axum::http::header::X_CONTENT_TYPE_OPTIONS,
        axum::http::HeaderValue::from_static("nosniff"),
    );
    response
}

fn aipp_bundle_content_type(path: &Path) -> Option<&'static str> {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("html") => Some("text/html; charset=utf-8"),
        Some("css") => Some("text/css; charset=utf-8"),
        Some("js" | "mjs") => Some("text/javascript; charset=utf-8"),
        Some("json") => Some("application/json; charset=utf-8"),
        Some("png") => Some("image/png"),
        Some("jpg" | "jpeg") => Some("image/jpeg"),
        Some("webp") => Some("image/webp"),
        Some("ico") => Some("image/x-icon"),
        Some("woff") => Some("font/woff"),
        Some("woff2") => Some("font/woff2"),
        _ => None,
    }
}

fn resolve_aipp_bundle_asset(
    active: &ActiveAippPackage,
    requested: &str,
) -> Result<(PathBuf, &'static str), String> {
    if active.aipp.renderer != "sandbox_bundle_v1"
        || active.aipp.data_contract != "capability_bridge_v1"
    {
        return Err("aipp_bundle_not_available".to_string());
    }
    let asset_root = active
        .aipp
        .asset_root
        .as_deref()
        .ok_or_else(|| "aipp_bundle_root_invalid".to_string())?;
    let requested = requested.trim_start_matches('/');
    let requested = if requested.is_empty() {
        active.aipp.entrypoint.as_deref().unwrap_or_default()
    } else {
        requested
    };
    let requested_path = Path::new(requested);
    if requested_path.is_absolute()
        || requested_path.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
        || !requested_path.starts_with(asset_root)
    {
        return Err("aipp_bundle_path_invalid".to_string());
    }
    let canonical_root = fs::canonicalize(active.package_root.join(asset_root))
        .map_err(|_| "aipp_bundle_not_available".to_string())?;
    let asset = fs::canonicalize(active.package_root.join(requested_path))
        .map_err(|_| "aipp_bundle_asset_not_found".to_string())?;
    let metadata = fs::metadata(&asset).map_err(|_| "aipp_bundle_asset_not_found".to_string())?;
    if !asset.starts_with(&canonical_root)
        || !metadata.is_file()
        || metadata.len() > AIPP_BUNDLE_ASSET_MAX_BYTES
    {
        return Err("aipp_bundle_path_invalid".to_string());
    }
    let content_type = aipp_bundle_content_type(&asset)
        .ok_or_else(|| "aipp_bundle_type_unsupported".to_string())?;
    Ok((asset, content_type))
}

fn verify_aipp_bundle_artifacts(
    state: &AppState,
    skill_name: &str,
    active: &ActiveAippPackage,
) -> Result<(), String> {
    let Some(binding) = state
        .get_skill_views_snapshot()
        .binding
        .admission_bindings
        .get(skill_name)
        .cloned()
    else {
        // Repository-maintained base packages are trusted source artifacts and have no
        // admission receipt. Path confinement is still enforced by resolve_aipp_bundle_asset.
        return Ok(());
    };
    let verified = skill_sdk::InstallReceiptStore::new(skill_package_root(state))
        .verified_current_install(skill_name)
        .map_err(|_| "aipp_install_receipt_invalid".to_string())?;
    let manifest_digest = verified
        .manifest
        .digest()
        .map_err(|_| "aipp_manifest_digest_failed".to_string())?;
    let receipt_digest = verified
        .receipt
        .digest()
        .map_err(|_| "aipp_receipt_digest_failed".to_string())?;
    if verified.install_dir != active.package_root
        || verified.manifest.package.version != binding.version
        || manifest_digest != binding.manifest_digest
        || receipt_digest != binding.install_receipt_digest
    {
        return Err("aipp_generation_binding_mismatch".to_string());
    }
    Ok(())
}

fn apply_aipp_bundle_headers(response: &mut axum::response::Response, content_type: &'static str) {
    response.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static(content_type),
    );
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("private, max-age=300"),
    );
    response.headers_mut().insert(
        axum::http::header::X_CONTENT_TYPE_OPTIONS,
        axum::http::HeaderValue::from_static("nosniff"),
    );
    response.headers_mut().insert(
        axum::http::header::REFERRER_POLICY,
        axum::http::HeaderValue::from_static("no-referrer"),
    );
    response.headers_mut().insert(
        axum::http::HeaderName::from_static("permissions-policy"),
        axum::http::HeaderValue::from_static(
            "camera=(), microphone=(), geolocation=(), payment=(), usb=(), serial=()",
        ),
    );
    if content_type.starts_with("text/html") {
        response.headers_mut().insert(
            axum::http::header::CONTENT_SECURITY_POLICY,
            axum::http::HeaderValue::from_static(AIPP_BUNDLE_CSP),
        );
    }
}

async fn get_aipp_bundle_asset(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath((skill_name, asset_path)): AxumPath<(String, String)>,
) -> axum::response::Response {
    if let Err(response) = require_ui_admin(&state, &headers) {
        return response.into_response();
    }
    let active = match active_aipp_package(&state, &skill_name) {
        Ok(Some(active)) => active,
        _ => return aipp_api_error(StatusCode::NOT_FOUND, "aipp_not_available").into_response(),
    };
    if !aipp_is_installed(&state, &skill_name) {
        return aipp_api_error(StatusCode::NOT_FOUND, "aipp_not_installed").into_response();
    }
    if let Err(error) = verify_aipp_bundle_artifacts(&state, &skill_name, &active) {
        return aipp_api_error(StatusCode::NOT_FOUND, &error).into_response();
    }
    let (path, content_type) = match resolve_aipp_bundle_asset(&active, &asset_path) {
        Ok(value) => value,
        Err(error) => return aipp_api_error(StatusCode::NOT_FOUND, &error).into_response(),
    };
    let bytes = match tokio::fs::read(path).await {
        Ok(bytes) => bytes,
        Err(_) => return aipp_api_error(StatusCode::NOT_FOUND, "aipp_bundle_asset_not_found").into_response(),
    };
    let mut response = axum::response::Response::new(axum::body::Body::from(bytes));
    apply_aipp_bundle_headers(&mut response, content_type);
    response
}

fn aipp_api_error(
    status: StatusCode,
    error: &str,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    (
        status,
        Json(ApiResponse {
            ok: false,
            data: Some(json!({
                "error_code": error,
                "message_key": format!("clawd.ui.aipp.{error}"),
            })),
            error: Some(error.to_string()),
        }),
    )
}
