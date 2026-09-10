use crate::{
    commands::{main_only, DesktopState},
    session::Session,
    transport::{bytes_body, small_json},
    Result,
};
use http::{HeaderMap, Method};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{Manager, State, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use uuid::Uuid;

#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub struct AippScope {
    pub session_id: Uuid,
    pub skill_name: String,
    pub package_version: String,
    pub entrypoint: String,
    pub bridge_capabilities: Vec<String>,
    pub locale: String,
}

async fn catalog_entry(session: &Session, skill: &str) -> Result<Value> {
    let (status, value) = small_json(
        session
            .request(Method::GET, "/v1/aipps", HeaderMap::new(), None)
            .await?,
    )
    .await?;
    if status != 200 {
        return Err("aipp_unavailable".into());
    }
    value["data"]["apps"]
        .as_array()
        .and_then(|apps| {
            apps.iter().find(|app| {
                app["skill_name"] == skill
                    && app["installed"] == true
                    && app["renderer"] == "sandbox_bundle_v1"
            })
        })
        .cloned()
        .ok_or("aipp_unavailable".into())
}
pub async fn validate_scope(session: &Session, scope: &AippScope) -> Result<()> {
    let entry = catalog_entry(session, &scope.skill_name).await?;
    if entry["package_version"] != scope.package_version
        || entry["entrypoint"] != scope.entrypoint
        || entry["bridge_capabilities"] != json!(scope.bridge_capabilities)
    {
        return Err("aipp_installation_changed".into());
    }
    Ok(())
}

#[tauri::command]
pub async fn aipp_open(
    window: WebviewWindow,
    app: tauri::AppHandle,
    state: State<'_, DesktopState>,
    session_id: Uuid,
    skill_name: String,
    locale: String,
) -> Result<()> {
    main_only(&window)?;
    #[cfg(debug_assertions)]
    eprintln!("aipp_open_start");
    if skill_name.is_empty()
        || skill_name.len() > 128
        || !skill_name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        return Err("aipp_skill_invalid".into());
    }
    let session = state.session(session_id).await?;
    let entry = catalog_entry(&session, &skill_name).await?;
    #[cfg(debug_assertions)]
    eprintln!("aipp_open_catalog_verified");
    let scope = AippScope {
        session_id,
        skill_name,
        package_version: entry["package_version"]
            .as_str()
            .ok_or("aipp_contract_invalid")?
            .into(),
        entrypoint: entry["entrypoint"]
            .as_str()
            .ok_or("aipp_contract_invalid")?
            .into(),
        bridge_capabilities: serde_json::from_value(entry["bridge_capabilities"].clone())
            .map_err(|_| "aipp_contract_invalid")?,
        locale: if locale == "en" { "en" } else { "zh" }.into(),
    };
    let label = format!("aipp-{}", Uuid::new_v4());
    {
        let mut scopes = state.aipps.lock().await;
        if scopes.len() >= 8 {
            return Err("aipp_window_limit".into());
        }
        scopes.insert(label.clone(), scope);
    }
    state.aipp_limits.lock().await.insert(
        label.clone(),
        std::sync::Arc::new(tokio::sync::Semaphore::new(4)),
    );
    let asset_prefix = format!(
        "/{session_id}/v1/aipps/{}/assets/",
        entry["skill_name"]
            .as_str()
            .ok_or("aipp_contract_invalid")?
    );
    let result = WebviewWindowBuilder::new(&app, &label, WebviewUrl::App("aipp.html".into()))
        .title(entry["titles"][&locale].as_str().unwrap_or("AiAPP"))
        .inner_size(1080.0, 760.0)
        .on_navigation(move |url| {
            crate::media::local_page(url, "/aipp.html")
                || crate::webview_origin::device_asset(url, &asset_prefix)
        })
        .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
        .build();
    #[cfg(debug_assertions)]
    eprintln!("aipp_open_window_created");
    match result {
        Ok(window) => {
            window.on_window_event(move |event| {
                if matches!(event, tauri::WindowEvent::Destroyed) {
                    let app = app.clone();
                    let label = label.clone();
                    tauri::async_runtime::spawn(async move {
                        let state = app.state::<DesktopState>();
                        state.aipps.lock().await.remove(&label);
                        if let Some(limit) = state.aipp_limits.lock().await.remove(&label) {
                            limit.close();
                        };
                    });
                }
            });
        }
        Err(_) => {
            state.aipps.lock().await.remove(&label);
            state.aipp_limits.lock().await.remove(&label);
            return Err("aipp_window_failed".into());
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn aipp_context(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<AippScope> {
    state
        .aipps
        .lock()
        .await
        .get(window.label())
        .cloned()
        .ok_or("aipp_scope_denied".into())
}

#[tauri::command]
pub async fn aipp_bridge(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    capability: String,
    args: Value,
) -> Result<Value> {
    let limit = state
        .aipp_limits
        .lock()
        .await
        .get(window.label())
        .cloned()
        .ok_or("aipp_scope_denied")?;
    let _permit = limit.try_acquire().map_err(|_| "aipp_bridge_busy")?;
    let scope = state
        .aipps
        .lock()
        .await
        .get(window.label())
        .cloned()
        .ok_or("aipp_scope_denied")?;
    if !scope.bridge_capabilities.contains(&capability)
        || !args.is_object()
        || args.to_string().len() > 64 * 1024
    {
        return Err("aipp_capability_denied".into());
    }
    let session = state.session(scope.session_id).await?;
    validate_scope(&session, &scope).await?;
    // The server still owns resolver, verifier, authorization and skill installation checks.
    let body = json!({"channel":"ui", "kind":"ask", "idempotency_key": format!("aipp-{}", Uuid::new_v4()),
        "payload":{"entrypoint":"run_capability", "capability":capability, "args":args}});
    let headers = HeaderMap::from_iter([(
        http::header::CONTENT_TYPE,
        http::HeaderValue::from_static("application/json"),
    )]);
    let (status, value) = small_json(
        session
            .request(
                Method::POST,
                "/v1/tasks",
                headers,
                Some(bytes_body(body.to_string())),
            )
            .await?,
    )
    .await?;
    if !(200..300).contains(&status) || value["ok"] != true {
        return Err("aipp_submission_failed".into());
    }
    let task = value["data"]["task_id"]
        .as_str()
        .ok_or("aipp_task_invalid")?;
    if !task.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
        return Err("aipp_task_invalid".into());
    }
    loop {
        if limit.is_closed() {
            return Err("aipp_window_closed".into());
        }
        tokio::select! {
            _ = session.cancelled.cancelled() => return Err("connection_closed".into()),
            _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => {},
        }
        validate_scope(&session, &scope).await?;
        let (_, value) = small_json(
            session
                .request(
                    Method::GET,
                    &format!("/v1/tasks/{task}"),
                    HeaderMap::new(),
                    None,
                )
                .await?,
        )
        .await?;
        let data = &value["data"];
        match data["status"].as_str() {
            Some("succeeded" | "failed" | "canceled" | "timeout") => {
                return Ok(json!({
                    "task_id": data["task_id"], "status": data["status"], "result_json": data["result_json"], "error_text": data["error_text"],
                }))
            }
            _ => {}
        }
    }
}
