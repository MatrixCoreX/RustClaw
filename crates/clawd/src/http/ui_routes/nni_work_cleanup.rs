fn nni_cleanup_terminal_lease_error(code: &str) -> bool {
    matches!(
        code,
        "nni_work_task_not_found"
            | "nni_work_lease_not_active"
            | "nni_work_lease_expired"
            | "nni_work_lease_token_invalid"
            | "nni_work_lease_token_mismatch"
    )
}

fn nni_cleanup_enrollment(
    state: &AppState,
    assignment: &NniWorkAssignment,
) -> Option<crate::task_result_retention::DeliveryCleanupEnrollment> {
    let NniWorkPayload::Capability(payload) = nni_validate_work_assignment(assignment).ok()? else {
        return None;
    };
    let policy = nni_capability_policy(state, &payload).ok()?;
    let delivery = payload.artifact_delivery.as_ref()?;
    if !policy
        .artifact_cleanup_grants
        .iter()
        .any(|grant| grant.origin == delivery.origin && grant.capability == payload.capability)
    {
        return None;
    }
    Some(crate::task_result_retention::DeliveryCleanupEnrollment {
        schema_version: 1,
        work_id: assignment.task_id.clone(),
        attempt: assignment.attempt,
        capability: payload.capability,
        origin: delivery.origin.clone(),
        job_id: delivery.job_id.clone(),
        payload_sha256: assignment.payload_sha256.clone(),
    })
}

fn nni_cleanup_binding_valid(
    local: &NniLocalWorkState,
    enrollment: &crate::task_result_retention::DeliveryCleanupEnrollment,
) -> bool {
    let Ok(payload) =
        serde_json::from_str::<NniWorkCapabilityPayload>(&local.assignment.payload_canonical)
    else {
        return false;
    };
    let Some(delivery) = payload.artifact_delivery else {
        return false;
    };
    local.assignment.task_type == "capability_v1"
        && enrollment.schema_version == 1
        && enrollment.work_id == local.assignment.task_id
        && enrollment.attempt == local.assignment.attempt
        && enrollment.capability == payload.capability
        && enrollment.origin == delivery.origin
        && enrollment.job_id == delivery.job_id
        && nni_validate_artifact_delivery(&delivery)
        && enrollment.payload_sha256 == local.assignment.payload_sha256
        && nni_work_sha256(&local.assignment.payload_canonical) == enrollment.payload_sha256
}

fn nni_recover_enrolled_task(
    state: &AppState,
    enrollment: &crate::task_result_retention::DeliveryCleanupEnrollment,
) -> anyhow::Result<Option<String>> {
    let row = state
        .core
        .db
        .get()?
        .query_row(
            "SELECT task_id, payload_json FROM tasks WHERE idempotency_key=?1 AND kind='run_skill'",
            [enrollment.idempotency_key()],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?;
    let Some((task_id, payload)) = row else {
        return Ok(None);
    };
    let payload: Value = serde_json::from_str(&payload)?;
    if enrollment.matches_payload(&payload)
        && crate::task_result_retention::read_enrollment(&state.skill_rt.workspace_root, &task_id)?
            .as_ref()
            == Some(enrollment)
    {
        return Ok(Some(task_id));
    }
    Ok(None)
}

fn nni_report_matches_local(local: &NniLocalWorkState, report: &NniHeartbeatWorkReport) -> bool {
    report.task_id == local.assignment.task_id
        && report.attempt == local.assignment.attempt
        && report.lease_token == local.assignment.lease_token
        && local.result.as_ref().is_some_and(|result| {
            serde_json::to_vec(result).ok() == serde_json::to_vec(&report.result).ok()
        })
}

fn nni_acknowledge_cleanup(
    state: &AppState,
    local: &NniLocalWorkState,
    submitted_report: Option<&NniHeartbeatWorkReport>,
) -> anyhow::Result<()> {
    // This is called only after Core's control signature and this poll's unique
    // heartbeat/device binding have been verified. Also bind its actual request.
    anyhow::ensure!(
        local.status == "completed_pending_report"
            && local.result.is_some()
            && submitted_report.is_some_and(|r| nni_report_matches_local(local, r)),
        "work_cleanup_ack_binding_invalid"
    );
    let enrollment = local
        .cleanup
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("work_cleanup_not_enrolled"))?;
    anyhow::ensure!(
        nni_cleanup_binding_valid(local, enrollment),
        "work_cleanup_assignment_binding_invalid"
    );
    let mut pending = local.clone();
    if pending.local_task_id.is_none() {
        pending.local_task_id = nni_recover_enrolled_task(state, enrollment)?;
    }
    if let Some(task_id) = pending.local_task_id.as_deref() {
        anyhow::ensure!(
            crate::task_result_retention::read_enrollment(&state.skill_rt.workspace_root, task_id)?
                .as_ref()
                == Some(enrollment),
            "work_cleanup_enrollment_missing"
        );
        crate::repo::distributed_task_cleanup::validate_task(
            &*state.core.db.get()?,
            task_id,
            enrollment,
        )?;
    }
    pending.cleanup_plan = None;
    pending.status = "cleanup_pending".to_string();
    if let Some(result) = pending.result.as_mut() {
        result.stdout.clear();
        result.stderr.clear();
    }
    // Persist ACK before waiting for in-flight producers. Retirement separately
    // persists the complete file plan under its exclusive gate before deletion.
    // A restart can resume locally without upload, model or Core.
    write_nni_local_work_state(state, &pending)?;
    nni_finish_acknowledged_cleanup(state, &pending)
}

fn nni_finish_acknowledged_cleanup(
    state: &AppState,
    local: &NniLocalWorkState,
) -> anyhow::Result<()> {
    static FINISH_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let _guard = FINISH_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| anyhow::anyhow!("work_cleanup_lock_failed"))?;
    let Some(mut current) = read_nni_local_work_state(state)? else {
        return Ok(());
    };
    anyhow::ensure!(
        current.status == "cleanup_pending"
            && current.assignment.task_id == local.assignment.task_id
            && current.assignment.attempt == local.assignment.attempt
            && current.local_task_id == local.local_task_id,
        "work_cleanup_current_binding_invalid"
    );
    let local = &mut current;
    anyhow::ensure!(
        local.status == "cleanup_pending",
        "work_cleanup_not_acknowledged"
    );
    let enrollment = local
        .cleanup
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("work_cleanup_not_enrolled"))?;
    anyhow::ensure!(
        nni_cleanup_binding_valid(local, enrollment),
        "work_cleanup_assignment_binding_invalid"
    );
    let Some(task_id) = local.local_task_id.as_deref() else {
        anyhow::ensure!(
            nni_recover_enrolled_task(state, enrollment)?.is_none(),
            "work_cleanup_task_recovery_required"
        );
        append_nni_work_audit(state, local)?;
        return nni_remove_local_work_state(state);
    };
    let _producer_guard = crate::task_result_retention::write_gate::begin_retiring(state, task_id)?;
    if local.cleanup_plan.is_none() {
        local.cleanup_plan = Some(crate::task_result_retention::files::prepare(
            &state.skill_rt.workspace_root,
            task_id,
        )?);
        write_nni_local_work_state(state, local)?;
    }
    let plan = local
        .cleanup_plan
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("work_cleanup_plan_missing"))?;
    anyhow::ensure!(
        local.local_task_id.as_deref() == Some(plan.task_id.as_str()),
        "work_cleanup_task_binding_invalid"
    );
    crate::repo::distributed_task_cleanup::retire_result(state, &plan.task_id, enrollment)?;
    crate::task_result_retention::files::execute(&state.skill_rt.workspace_root, plan)?;
    append_nni_work_audit(state, local)?;
    append_nni_log_event_best_effort(
        state,
        "distributed_work_cleanup_completed",
        json!({"task_id":local.assignment.task_id,"attempt":local.assignment.attempt,"local_task_id":plan.task_id,
            "result_retention":"removed_after_delivery_ack"}),
    );
    nni_remove_local_work_state(state)
}

#[cfg(test)]
#[path = "nni_work_cleanup_tests.rs"]
mod nni_work_cleanup_tests;
