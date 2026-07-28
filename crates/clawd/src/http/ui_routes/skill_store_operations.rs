fn skill_store_operation_stage(phase: &str) -> rustclaw_skill_sdk::OperationStage {
    use rustclaw_skill_sdk::OperationStage;
    match phase {
        "preflight" | "manifest" | "toolchain" | "precompiled_verify" => {
            OperationStage::Preflight
        }
        "dependencies" | "prepare_environment" => OperationStage::Dependencies,
        "build" | "artifact" | "copy_source" | "source_digest" | "precompiled_copy" => {
            OperationStage::Build
        }
        "protocol_smoke" => OperationStage::Smoke,
        "activate" => OperationStage::Activate,
        "configure" => OperationStage::Configure,
        "remove" => OperationStage::Remove,
        "rollback" => OperationStage::Rollback,
        _ => OperationStage::Preflight,
    }
}

fn transition_skill_store_operation(
    store: &rustclaw_skill_sdk::SkillOperationStore,
    operation_id: &str,
    status: rustclaw_skill_sdk::OperationStatus,
    stage: rustclaw_skill_sdk::OperationStage,
    failure: Option<rustclaw_skill_sdk::OperationFailure>,
    result: Option<Value>,
) {
    if let Err(error) = store.transition(operation_id, status, stage, failure, result) {
        tracing::warn!(
            operation_id,
            error_code = %error.code,
            diagnostic = %error.detail,
            "skill_store_operation_transition_failed"
        );
    }
}

fn skill_store_install_control(
    state: &AppState,
    operation_id: &str,
) -> rustclaw_skill_sdk::InstallControl {
    let cancelled = Arc::new(AtomicBool::new(false));
    let store = skill_store_operation_store(state);
    let progress_operation_id = operation_id.to_string();
    let progress = Arc::new(move |phase: &str| {
        let stage = skill_store_operation_stage(phase);
        let should_record = store
            .get(&progress_operation_id)
            .map(|operation| {
                !operation.status.is_terminal()
                    && (operation.status != rustclaw_skill_sdk::OperationStatus::Running
                        || operation.stage != stage)
            })
            .unwrap_or(false);
        if should_record {
            transition_skill_store_operation(
                &store,
                &progress_operation_id,
                rustclaw_skill_sdk::OperationStatus::Running,
                stage,
                None,
                None,
            );
        }
    });
    let control = rustclaw_skill_sdk::InstallControl::with_progress(cancelled, progress);
    register_skill_store_control(state, operation_id, control.clone());
    control
}

fn start_skill_store_heartbeat(
    state: &AppState,
    operation_id: &str,
) -> tokio::task::JoinHandle<()> {
    let store = skill_store_operation_store(state);
    let operation_id = operation_id.to_string();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            match store.get(&operation_id) {
                Ok(operation) if operation.status.is_terminal() => break,
                Ok(_) => {
                    if let Err(error) = store.heartbeat(&operation_id) {
                        tracing::warn!(
                            operation_id,
                            error_code = %error.code,
                            "skill_store_operation_heartbeat_failed"
                        );
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    })
}

fn operation_cancelled(
    store: &rustclaw_skill_sdk::SkillOperationStore,
    operation_id: &str,
    control: &rustclaw_skill_sdk::InstallControl,
) -> bool {
    control.is_cancelled()
        || store
            .get(operation_id)
            .map(|operation| operation.cancel_requested)
            .unwrap_or(false)
}

fn finish_skill_store_failure(
    store: &rustclaw_skill_sdk::SkillOperationStore,
    operation_id: &str,
    error: SkillStoreOperationError,
    cancelled: bool,
) {
    if cancelled {
        transition_skill_store_operation(
            store,
            operation_id,
            rustclaw_skill_sdk::OperationStatus::Cancelled,
            rustclaw_skill_sdk::OperationStage::Cancelled,
            None,
            None,
        );
        return;
    }
    tracing::warn!(
        operation_id,
        error_code = error.code.as_str(),
        diagnostic = %error.diagnostic,
        "skill_store_background_operation_failed"
    );
    transition_skill_store_operation(
        store,
        operation_id,
        rustclaw_skill_sdk::OperationStatus::Failure,
        rustclaw_skill_sdk::OperationStage::Failure,
        Some(rustclaw_skill_sdk::OperationFailure {
            error_code: error.code.as_str().to_string(),
            message_key: format!("skill_store.{}", error.code.as_str()),
            phase: error.phase,
            retryable: true,
            diagnostic: Some(
                rustclaw_skill_sdk::redact_diagnostics(&error.diagnostic)
                    .chars()
                    .take(8 * 1024)
                    .collect(),
            ),
        }),
        None,
    );
}

fn accepted_skill_store_operation(
    operation: rustclaw_skill_sdk::SkillOperation,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    (
        StatusCode::ACCEPTED,
        Json(ApiResponse {
            ok: true,
            data: Some(json!({"operation": operation})),
            error: None,
        }),
    )
}

fn inferred_install_action(
    state: &AppState,
    skill_name: &str,
) -> rustclaw_skill_sdk::OperationAction {
    if rustclaw_skill_sdk::SkillRuntimeResolver::new(skill_package_root(state))
        .resolve(skill_name)
        .is_ok()
    {
        return rustclaw_skill_sdk::OperationAction::Update;
    }
    let configured = read_skill_config_file(state)
        .map(|(_, parsed)| !collect_uninstalled_skills(&parsed, state).contains(skill_name))
        .unwrap_or(false);
    if configured {
        rustclaw_skill_sdk::OperationAction::Repair
    } else {
        rustclaw_skill_sdk::OperationAction::Install
    }
}

async fn start_skill_store_install(
    state: AppState,
    headers: HeaderMap,
    request: SkillStoreMutationRequest,
    requested_action: Option<rustclaw_skill_sdk::OperationAction>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(response) = require_ui_identity(&state, &headers) {
        return response;
    }
    if let Err(error) = initialize_skill_store_operations(&state) {
        return skill_store_error_response(error);
    }
    let skill_name = match validate_skill_store_mutation(&state, &request.skill_name) {
        Ok(name) => name,
        Err(error) => return skill_store_error_response(error),
    };
    let spec = match skill_store_install_spec(&state, &skill_name) {
        Ok(spec) => spec,
        Err(error) => return skill_store_error_response(error),
    };
    let allow_network = request.allow_network.unwrap_or(false);
    if spec.as_ref().is_some_and(|value| {
        value.network_policy == rustclaw_skill_sdk::BuildNetworkPolicy::ApprovalRequired
            && !allow_network
    }) {
        return skill_store_error_response(SkillStoreOperationError::new(
            StatusCode::CONFLICT,
            SkillStoreErrorCode::NetworkApprovalRequired,
            format!("skill={skill_name} build_network=approval_required"),
        ));
    }
    let mutation_guard = match begin_skill_store_mutation(&state, &skill_name) {
        Ok(guard) => guard,
        Err(error) => return skill_store_error_response(error),
    };
    let action = requested_action.unwrap_or_else(|| inferred_install_action(&state, &skill_name));
    let operation = match skill_store_operation_store(&state).create(&skill_name, action) {
        Ok(operation) => operation,
        Err(error) => {
            return skill_store_error_response(SkillStoreOperationError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                SkillStoreErrorCode::OperationStateFailed,
                error,
            ));
        }
    };
    let control = skill_store_install_control(&state, &operation.operation_id);
    let worker_state = state.clone();
    let worker_operation_id = operation.operation_id.clone();
    tokio::spawn(async move {
        run_skill_store_install_operation(
            worker_state,
            worker_operation_id,
            spec,
            action,
            control,
            allow_network,
            mutation_guard,
        )
        .await;
    });
    accepted_skill_store_operation(operation)
}

async fn run_skill_store_install_operation(
    state: AppState,
    operation_id: String,
    spec: Option<SkillStoreInstallSpec>,
    action: rustclaw_skill_sdk::OperationAction,
    control: rustclaw_skill_sdk::InstallControl,
    allow_network: bool,
    _mutation_guard: SkillStoreMutationGuard,
) {
    let store = skill_store_operation_store(&state);
    let _heartbeat = start_skill_store_heartbeat(&state, &operation_id);
    let skill_name = store
        .get(&operation_id)
        .map(|operation| operation.skill_name)
        .unwrap_or_default();
    let receipt_store = rustclaw_skill_sdk::InstallReceiptStore::new(skill_package_root(&state));
    let pointer_before = receipt_store.current_pointer(&skill_name).ok();
    let result = run_skill_store_install_operation_inner(
        &state,
        &store,
        &operation_id,
        spec.as_ref(),
        &control,
        allow_network,
    )
    .await;
    match result {
        Ok(data) => transition_skill_store_operation(
            &store,
            &operation_id,
            rustclaw_skill_sdk::OperationStatus::Success,
            rustclaw_skill_sdk::OperationStage::Success,
            None,
            Some(data),
        ),
        Err(error) => {
            let pointer_changed = pointer_before.as_ref().is_some_and(|before| {
                receipt_store
                    .current_pointer(&skill_name)
                    .map(|current| current != *before)
                    .unwrap_or(false)
            });
            if action == rustclaw_skill_sdk::OperationAction::Update && pointer_changed {
                let _ = receipt_store.rollback(&skill_name);
            }
            finish_skill_store_failure(
                &store,
                &operation_id,
                error,
                operation_cancelled(&store, &operation_id, &control),
            );
        }
    }
    remove_skill_store_control(&state, &operation_id);
}

async fn run_skill_store_install_operation_inner(
    state: &AppState,
    store: &rustclaw_skill_sdk::SkillOperationStore,
    operation_id: &str,
    spec: Option<&SkillStoreInstallSpec>,
    control: &rustclaw_skill_sdk::InstallControl,
    allow_network: bool,
) -> SkillStoreOperationResult<Value> {
    if operation_cancelled(store, operation_id, control) {
        return Err(SkillStoreOperationError::new(
            StatusCode::CONFLICT,
            SkillStoreErrorCode::OperationStateFailed,
            "cancelled_before_preflight",
        ));
    }
    transition_skill_store_operation(
        store,
        operation_id,
        rustclaw_skill_sdk::OperationStatus::Running,
        rustclaw_skill_sdk::OperationStage::Preflight,
        None,
        None,
    );
    let install_outcome = if let Some(spec) = spec {
        let Some(_build_permit) = skill_store_build_permit(state, control).await else {
            return Err(SkillStoreOperationError::new(
                StatusCode::CONFLICT,
                SkillStoreErrorCode::OperationStateFailed,
                "cancelled_while_queued",
            ));
        };
        if operation_cancelled(store, operation_id, control) {
            return Err(SkillStoreOperationError::new(
                StatusCode::CONFLICT,
                SkillStoreErrorCode::OperationStateFailed,
                "cancelled_while_queued",
            ));
        }
        Some(install_skill_store_package(state, spec, control.clone(), allow_network).await?)
    } else {
        None
    };
    if operation_cancelled(store, operation_id, control) {
        return Err(SkillStoreOperationError::new(
            StatusCode::CONFLICT,
            SkillStoreErrorCode::OperationStateFailed,
            "cancelled_before_configuration",
        ));
    }
    transition_skill_store_operation(
        store,
        operation_id,
        rustclaw_skill_sdk::OperationStatus::Running,
        rustclaw_skill_sdk::OperationStage::Configure,
        None,
        None,
    );
    let skill_name = store
        .get(operation_id)
        .map_err(|error| {
            SkillStoreOperationError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                SkillStoreErrorCode::OperationStateFailed,
                error,
            )
        })?
        .skill_name;
    let _config_permit = skill_store_config_permit(state).await;
    let mut data = update_skill_store_installation(state, &skill_name, true)?;
    let (_, existing_config_files) = skill_config_state(state, &skill_name);
    if let Some(object) = data.as_object_mut() {
        object.insert("package_installed".to_string(), json!(spec.is_some()));
        object.insert(
            "adapter".to_string(),
            json!(install_outcome
                .as_ref()
                .map(|value| value.adapter.as_token())),
        );
        object.insert(
            "install_origin".to_string(),
            json!(install_outcome.as_ref().map(|value| value.origin)),
        );
        object.insert(
            "installed_version".to_string(),
            json!(install_outcome.as_ref().map(|value| value.version.as_str())),
        );
        object.insert(
            "receipt_digest".to_string(),
            json!(install_outcome
                .as_ref()
                .map(|value| value.receipt_digest.as_str())),
        );
        object.insert(
            "install_reused".to_string(),
            json!(install_outcome
                .as_ref()
                .map(|value| value.reused)
                .unwrap_or(false)),
        );
        object.insert(
            "install_phases".to_string(),
            json!(install_outcome.as_ref().map(|value| &value.phases)),
        );
        object.insert(
            "reused_config_files".to_string(),
            json!(existing_config_files),
        );
    }
    Ok(data)
}

async fn install_skill_store_item(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<SkillStoreMutationRequest>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    start_skill_store_install(state, headers, request, None).await
}

async fn update_skill_store_item(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<SkillStoreMutationRequest>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    start_skill_store_install(
        state,
        headers,
        request,
        Some(rustclaw_skill_sdk::OperationAction::Update),
    )
    .await
}

async fn repair_skill_store_item(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<SkillStoreMutationRequest>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    start_skill_store_install(
        state,
        headers,
        request,
        Some(rustclaw_skill_sdk::OperationAction::Repair),
    )
    .await
}

async fn remove_skill_store_item(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<SkillStoreMutationRequest>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(response) = require_ui_identity(&state, &headers) {
        return response;
    }
    if let Err(error) = initialize_skill_store_operations(&state) {
        return skill_store_error_response(error);
    }
    let skill_name = match validate_skill_store_mutation(&state, &request.skill_name) {
        Ok(name) => name,
        Err(error) => return skill_store_error_response(error),
    };
    let spec = match skill_store_install_spec(&state, &skill_name) {
        Ok(spec) => spec,
        Err(error) => return skill_store_error_response(error),
    };
    let mutation_guard = match begin_skill_store_mutation(&state, &skill_name) {
        Ok(guard) => guard,
        Err(error) => return skill_store_error_response(error),
    };
    let operation = match skill_store_operation_store(&state)
        .create(&skill_name, rustclaw_skill_sdk::OperationAction::Remove)
    {
        Ok(operation) => operation,
        Err(error) => {
            return skill_store_error_response(SkillStoreOperationError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                SkillStoreErrorCode::OperationStateFailed,
                error,
            ));
        }
    };
    let control = skill_store_install_control(&state, &operation.operation_id);
    let worker_state = state.clone();
    let worker_operation_id = operation.operation_id.clone();
    let preserve_config = request.preserve_config.unwrap_or(true);
    let preserve_data = request.preserve_data.unwrap_or(true);
    tokio::spawn(async move {
        run_skill_store_remove_operation(
            worker_state,
            worker_operation_id,
            spec,
            preserve_config,
            preserve_data,
            control,
            mutation_guard,
        )
        .await;
    });
    accepted_skill_store_operation(operation)
}

async fn run_skill_store_remove_operation(
    state: AppState,
    operation_id: String,
    spec: Option<SkillStoreInstallSpec>,
    preserve_config: bool,
    preserve_data: bool,
    control: rustclaw_skill_sdk::InstallControl,
    _mutation_guard: SkillStoreMutationGuard,
) {
    let store = skill_store_operation_store(&state);
    let _heartbeat = start_skill_store_heartbeat(&state, &operation_id);
    let operation = match store.get(&operation_id) {
        Ok(operation) => operation,
        Err(error) => {
            tracing::warn!(operation_id, error_code = %error.code, "skill_store_remove_state_missing");
            return;
        }
    };
    if operation_cancelled(&store, &operation_id, &control) {
        finish_skill_store_failure(
            &store,
            &operation_id,
            SkillStoreOperationError::new(
                StatusCode::CONFLICT,
                SkillStoreErrorCode::OperationStateFailed,
                "cancelled_before_remove",
            ),
            true,
        );
        remove_skill_store_control(&state, &operation_id);
        return;
    }
    transition_skill_store_operation(
        &store,
        &operation_id,
        rustclaw_skill_sdk::OperationStatus::Running,
        rustclaw_skill_sdk::OperationStage::Configure,
        None,
        None,
    );
    let result = async {
        let _config_permit = skill_store_config_permit(&state).await;
        let mut data = update_skill_store_installation(&state, &operation.skill_name, false)?;
        transition_skill_store_operation(
            &store,
            &operation_id,
            rustclaw_skill_sdk::OperationStatus::Running,
            rustclaw_skill_sdk::OperationStage::Remove,
            None,
            None,
        );
        let package_removed = match spec.as_ref() {
            Some(spec) => remove_skill_store_package(&state, spec)?,
            None => false,
        };
        let deleted_config_files = if preserve_config {
            Vec::new()
        } else {
            delete_declared_skill_configs(&state, &operation.skill_name)?
        };
        let deleted_data = if preserve_data {
            None
        } else if state
            .get_skills_registry()
            .is_some_and(|registry| registry.storage(&operation.skill_name).is_some())
        {
            Some(
                state
                    .core
                    .skill_storage
                    .clear_skill_data(&operation.skill_name)
                    .map_err(|error| {
                        SkillStoreOperationError::new(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            SkillStoreErrorCode::DataRemoveFailed,
                            error,
                        )
                    })?,
            )
        } else {
            None
        };
        if let Some(object) = data.as_object_mut() {
            object.insert("package_removed".to_string(), json!(package_removed));
            object.insert("config_preserved".to_string(), json!(preserve_config));
            object.insert("data_preserved".to_string(), json!(preserve_data));
            object.insert(
                "deleted_config_files".to_string(),
                json!(deleted_config_files),
            );
            object.insert("deleted_private_data".to_string(), json!(deleted_data));
        }
        Ok::<_, SkillStoreOperationError>(data)
    }
    .await;
    match result {
        Ok(data) => transition_skill_store_operation(
            &store,
            &operation_id,
            rustclaw_skill_sdk::OperationStatus::Success,
            rustclaw_skill_sdk::OperationStage::Success,
            None,
            Some(data),
        ),
        Err(error) => finish_skill_store_failure(&store, &operation_id, error, false),
    }
    remove_skill_store_control(&state, &operation_id);
}

async fn rollback_skill_store_item(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<SkillStoreMutationRequest>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(response) = require_ui_identity(&state, &headers) {
        return response;
    }
    if let Err(error) = initialize_skill_store_operations(&state) {
        return skill_store_error_response(error);
    }
    let skill_name = match validate_skill_store_mutation(&state, &request.skill_name) {
        Ok(name) => name,
        Err(error) => return skill_store_error_response(error),
    };
    let mutation_guard = match begin_skill_store_mutation(&state, &skill_name) {
        Ok(guard) => guard,
        Err(error) => return skill_store_error_response(error),
    };
    let operation = match skill_store_operation_store(&state)
        .create(&skill_name, rustclaw_skill_sdk::OperationAction::Rollback)
    {
        Ok(operation) => operation,
        Err(error) => {
            return skill_store_error_response(SkillStoreOperationError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                SkillStoreErrorCode::OperationStateFailed,
                error,
            ));
        }
    };
    let worker_state = state.clone();
    let worker_operation_id = operation.operation_id.clone();
    tokio::spawn(async move {
        let _mutation_guard = mutation_guard;
        let store = skill_store_operation_store(&worker_state);
        transition_skill_store_operation(
            &store,
            &worker_operation_id,
            rustclaw_skill_sdk::OperationStatus::Running,
            rustclaw_skill_sdk::OperationStage::Rollback,
            None,
            None,
        );
        let receipt_store =
            rustclaw_skill_sdk::InstallReceiptStore::new(skill_package_root(&worker_state));
        let result = receipt_store.rollback(&skill_name).map_err(|error| {
            SkillStoreOperationError::new(
                StatusCode::CONFLICT,
                SkillStoreErrorCode::RollbackUnavailable,
                error,
            )
        });
        match result {
            Ok(pointer) => {
                let _config_permit = skill_store_config_permit(&worker_state).await;
                match update_skill_store_installation(&worker_state, &skill_name, true) {
                    Ok(config) => transition_skill_store_operation(
                        &store,
                        &worker_operation_id,
                        rustclaw_skill_sdk::OperationStatus::Success,
                        rustclaw_skill_sdk::OperationStage::Success,
                        None,
                        Some(json!({"pointer": pointer, "config": config})),
                    ),
                    Err(error) => {
                        let _ = receipt_store.rollback(&skill_name);
                        finish_skill_store_failure(&store, &worker_operation_id, error, false);
                    }
                }
            }
            Err(error) => finish_skill_store_failure(&store, &worker_operation_id, error, false),
        }
    });
    accepted_skill_store_operation(operation)
}

async fn get_skill_store_operation(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(operation_id): AxumPath<String>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(response) = require_ui_identity(&state, &headers) {
        return response;
    }
    if let Err(error) = initialize_skill_store_operations(&state) {
        return skill_store_error_response(error);
    }
    match skill_store_operation_store(&state).get(&operation_id) {
        Ok(operation) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(json!({"operation": operation})),
                error: None,
            }),
        ),
        Err(error) => skill_store_error_response(SkillStoreOperationError::new(
            StatusCode::NOT_FOUND,
            SkillStoreErrorCode::OperationNotFound,
            error,
        )),
    }
}

async fn cancel_skill_store_operation(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(operation_id): AxumPath<String>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    if let Err(response) = require_ui_identity(&state, &headers) {
        return response;
    }
    if let Err(error) = initialize_skill_store_operations(&state) {
        return skill_store_error_response(error);
    }
    match skill_store_operation_store(&state).request_cancel(&operation_id) {
        Ok(operation) => {
            request_live_skill_store_cancel(&state, &operation_id);
            (
                StatusCode::ACCEPTED,
                Json(ApiResponse {
                    ok: true,
                    data: Some(json!({"operation": operation})),
                    error: None,
                }),
            )
        }
        Err(error) => skill_store_error_response(SkillStoreOperationError::new(
            StatusCode::NOT_FOUND,
            SkillStoreErrorCode::OperationNotFound,
            error,
        )),
    }
}
