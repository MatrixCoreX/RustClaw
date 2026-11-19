// Admin review is separate from isolated package installation and build networking.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SkillPolicyDecision {
    Approve,
    Revoke,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SkillPolicyReviewRequest {
    decision: SkillPolicyDecision,
    registry_generation: u64,
    manifest_digest: String,
    capability_request_digest: String,
    #[serde(default)]
    enable: bool,
}

fn external_policy_review_value(
    manifest: &skill_sdk::PackageManifest,
    generation: u64,
    state: skill_sdk::AdmissionState,
) -> Result<Value, String> {
    let request = manifest
        .effective_capability_request()
        .map_err(|e| e.to_string())?;
    // A proposed grant is not published here and grants no execution authority.
    let proposal = imported_host_policy_grant(manifest)?;
    Ok(json!({
        "schema_version": 1,
        "skill_name": manifest.package.name,
        "package_version": manifest.package.version,
        "manifest_digest": manifest.digest().map_err(|e| e.to_string())?,
        "capability_request_digest": manifest.capability_request_digest().map_err(|e| e.to_string())?,
        "registry_generation": generation,
        "admission_state": state,
        "permissions": request.permissions,
        "capabilities": request.capabilities,
        "risk_level": proposal.risk_level,
    }))
}

async fn get_external_skill_policy(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(name): AxumPath<String>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(response) = require_ui_admin(&state, &headers) {
        return response;
    }
    match tokio::task::spawn_blocking(move || {
        let service = admission_service(&state)?;
        let (manifest, generation, status) = service
            .external_policy_review(&name)
            .map_err(|e| e.code.to_string())?;
        external_policy_review_value(&manifest, generation, status)
    })
    .await
    {
        Ok(Ok(value)) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(value),
                error: None,
            }),
        ),
        Ok(Err(code)) => crate::api_err(StatusCode::CONFLICT, &code),
        Err(_) => crate::api_err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "skill_policy_review_unavailable",
        ),
    }
}

async fn update_external_skill_policy(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(name): AxumPath<String>,
    Json(request): Json<SkillPolicyReviewRequest>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(response) = require_ui_admin(&state, &headers) {
        return response;
    }
    match tokio::task::spawn_blocking(move || {
        let service = admission_service(&state)?;
        let (manifest, generation, _) = service
            .external_policy_review(&name)
            .map_err(|e| e.code.to_string())?;
        if generation != request.registry_generation
            || manifest.digest().map_err(|e| e.to_string())? != request.manifest_digest
            || manifest
                .capability_request_digest()
                .map_err(|e| e.to_string())?
                != request.capability_request_digest
        {
            return Err("skill_policy_review_stale".to_string());
        }
        let (status, grant) = match request.decision {
            SkillPolicyDecision::Approve => (
                if request.enable {
                    skill_sdk::AdmissionState::Enabled
                } else {
                    skill_sdk::AdmissionState::InstalledDisabled
                },
                Some(imported_host_policy_grant(&manifest)?),
            ),
            SkillPolicyDecision::Revoke => {
                (skill_sdk::AdmissionState::AwaitingPolicyApproval, None)
            }
        };
        let published = service
            .set_states(
                vec![crate::skill_admission::AdmissionStateChange {
                    skill_name: name,
                    state: status,
                    grant,
                }],
                Some(generation),
            )
            .map_err(|e| e.code.to_string())?;
        if let Err(error) = reload_skill_views(&state) {
            tracing::warn!(error = %error, "skill_policy_reload_failed");
            service
                .rollback_generation(published.generation)
                .map_err(|e| e.code.to_string())?;
            let _ = reload_skill_views(&state);
            return Err("skill_policy_reload_failed".to_string());
        }
        external_policy_review_value(&manifest, published.generation, status)
    })
    .await
    {
        Ok(Ok(value)) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(value),
                error: None,
            }),
        ),
        Ok(Err(code)) => crate::api_err(StatusCode::CONFLICT, &code),
        Err(_) => crate::api_err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "skill_policy_review_unavailable",
        ),
    }
}

async fn list_external_skill_policies(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(response) = require_ui_admin(&state, &headers) {
        return response;
    }
    match tokio::task::spawn_blocking(move || {
        let service = admission_service(&state)?;
        let snapshot = service.catalog_snapshot().map_err(|e| e.code.to_string())?;
        let names: Vec<_> = snapshot
            .sources
            .iter()
            .filter(|(name, source)| {
                **source == crate::skill_admission::SkillAdmissionSource::ExternalOverlay
                    && !snapshot.tombstoned.contains(*name)
            })
            .map(|(name, _)| name.clone())
            .collect();
        let mut reviews = Vec::new();
        for name in names.iter().take(256) {
            let (manifest, generation, status) = service
                .external_policy_review(name)
                .map_err(|e| e.code.to_string())?;
            reviews.push(external_policy_review_value(&manifest, generation, status)?);
        }
        Ok::<_, String>(
            json!({"reviews": reviews, "total": names.len(), "truncated": names.len() > 256}),
        )
    })
    .await
    {
        Ok(Ok(value)) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(value),
                error: None,
            }),
        ),
        Ok(Err(code)) => crate::api_err(StatusCode::CONFLICT, &code),
        Err(_) => crate::api_err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "skill_policy_review_unavailable",
        ),
    }
}
