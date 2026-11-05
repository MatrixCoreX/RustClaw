const AIPP_PREVIEW_MAX_BYTES: u64 = 16 * 1024 * 1024;
const AIPP_BUNDLE_ASSET_MAX_BYTES: u64 = 4 * 1024 * 1024;
const AIPP_BUNDLE_CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' data: blob: https:; media-src 'self' blob:; font-src 'self'; connect-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'; frame-ancestors 'self'";

fn resolve_aipp_preview(root: &Path, sequence: u64) -> Result<(PathBuf, &'static str), String> {
    if sequence == 0 {
        return Err("aipp_preview_not_found".to_string());
    }
    let record_path = root.join("records").join(format!("{sequence:012}.json"));
    if record_path
        .metadata()
        .map(|metadata| metadata.len() > AIPP_MEDIA_RECORD_MAX_BYTES)
        .unwrap_or(true)
    {
        return Err("aipp_preview_record_invalid".to_string());
    }
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
    let exports_root =
        fs::canonicalize(root.join("exports")).map_err(|_| "aipp_preview_not_found".to_string())?;
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

type AippAssetResolution = Result<(PathBuf, &'static str), (StatusCode, Json<ApiResponse<Value>>)>;

fn prepare_aipp_media_preview(
    state: &AppState,
    headers: &HeaderMap,
    skill_name: &str,
    sequence: u64,
) -> AippAssetResolution {
    require_ui_admin(state, headers)?;
    let active = active_aipp_package(state, skill_name);
    if !aipp_is_installed(state, skill_name)
        || !matches!(active, Ok(Some(ref active)) if active.aipp.data_contract == "media_collection_v1")
    {
        return Err(aipp_api_error(StatusCode::NOT_FOUND, "aipp_not_available"));
    }
    let root = state
        .core
        .skill_storage
        .resolved_directory_path(skill_name)
        .map_err(|_| aipp_api_error(StatusCode::BAD_REQUEST, "aipp_storage_invalid"))?;
    resolve_aipp_preview(&root, sequence)
        .map_err(|error| aipp_api_error(StatusCode::NOT_FOUND, &error))
}

async fn get_aipp_media_preview(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath((skill_name, sequence)): AxumPath<(String, u64)>,
) -> axum::response::Response {
    let resolved = tokio::task::spawn_blocking(move || {
        prepare_aipp_media_preview(&state, &headers, &skill_name, sequence)
    })
    .await;
    let (path, content_type) = match resolved {
        Ok(Ok(value)) => value,
        Ok(Err(response)) => return response.into_response(),
        Err(error) => {
            tracing::warn!(%error, "aipp_preview_read_join_failed");
            return aipp_api_error(StatusCode::INTERNAL_SERVER_ERROR, "aipp_read_task_failed")
                .into_response();
        }
    };
    let file = match tokio::fs::File::open(path).await {
        Ok(file) => file,
        Err(_) => {
            return aipp_api_error(StatusCode::NOT_FOUND, "aipp_preview_not_found").into_response();
        }
    };
    let content_length = match file.metadata().await {
        Ok(metadata) if metadata.is_file() && metadata.len() <= AIPP_PREVIEW_MAX_BYTES => {
            metadata.len()
        }
        _ => {
            return aipp_api_error(StatusCode::NOT_FOUND, "aipp_preview_not_found").into_response();
        }
    };
    let mut response = axum::response::Response::new(axum::body::Body::from_stream(
        tokio_util::io::ReaderStream::new(file),
    ));
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
    if let Ok(value) = axum::http::HeaderValue::from_str(&content_length.to_string()) {
        response
            .headers_mut()
            .insert(axum::http::header::CONTENT_LENGTH, value);
    }
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

fn prepare_aipp_bundle_asset(
    state: &AppState,
    headers: &HeaderMap,
    skill_name: &str,
    asset_path: &str,
) -> AippAssetResolution {
    require_ui_admin(state, headers)?;
    let active = match active_aipp_package(state, skill_name) {
        Ok(Some(active)) => active,
        _ => return Err(aipp_api_error(StatusCode::NOT_FOUND, "aipp_not_available")),
    };
    if !aipp_is_installed(state, skill_name) {
        return Err(aipp_api_error(StatusCode::NOT_FOUND, "aipp_not_installed"));
    }
    verify_aipp_bundle_artifacts(state, skill_name, &active)
        .map_err(|error| aipp_api_error(StatusCode::NOT_FOUND, &error))?;
    resolve_aipp_bundle_asset(&active, asset_path)
        .map_err(|error| aipp_api_error(StatusCode::NOT_FOUND, &error))
}

async fn get_aipp_bundle_asset(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath((skill_name, asset_path)): AxumPath<(String, String)>,
) -> axum::response::Response {
    let resolved = tokio::task::spawn_blocking(move || {
        prepare_aipp_bundle_asset(&state, &headers, &skill_name, &asset_path)
    })
    .await;
    let (path, content_type) = match resolved {
        Ok(Ok(value)) => value,
        Ok(Err(response)) => return response.into_response(),
        Err(error) => {
            tracing::warn!(%error, "aipp_bundle_read_join_failed");
            return aipp_api_error(StatusCode::INTERNAL_SERVER_ERROR, "aipp_read_task_failed")
                .into_response();
        }
    };
    let file = match tokio::fs::File::open(path).await {
        Ok(file) => file,
        Err(_) => {
            return aipp_api_error(StatusCode::NOT_FOUND, "aipp_bundle_asset_not_found")
                .into_response();
        }
    };
    let content_length = match file.metadata().await {
        Ok(metadata) if metadata.is_file() && metadata.len() <= AIPP_BUNDLE_ASSET_MAX_BYTES => {
            metadata.len()
        }
        _ => {
            return aipp_api_error(StatusCode::NOT_FOUND, "aipp_bundle_asset_not_found")
                .into_response();
        }
    };
    let mut response = axum::response::Response::new(axum::body::Body::from_stream(
        tokio_util::io::ReaderStream::new(file),
    ));
    apply_aipp_bundle_headers(&mut response, content_type);
    if let Ok(value) = axum::http::HeaderValue::from_str(&content_length.to_string()) {
        response
            .headers_mut()
            .insert(axum::http::header::CONTENT_LENGTH, value);
    }
    response
}
