//! Read-only observations. This view does not initialize policy, admit work,
//! change thresholds or claim model/provider readiness.
use rusqlite::{types::ValueRef, Connection, OptionalExtension};
use serde::Serialize;

use super::capacity::{GenerationAvailability, PrincipalQuotaSnapshot, StoragePressure};
use super::settings::MemoryEffectiveSettings;

#[path = "availability_measurements.rs"]
mod measurements;
pub(crate) use measurements::{filesystem_available, ByteMeasurement, MeasurementUnavailable};

#[derive(Debug, Clone, Serialize)]
pub(crate) struct MemoryAvailability {
    pub(crate) schema_version: u32,
    pub(crate) sampled_at_ts: i64,
    pub(crate) settings: MemoryEffectiveSettings,
    pub(crate) capacity_decision: GenerationAvailability,
    pub(crate) quota: QuotaOverview,
    pub(crate) pressure: PressureObservation,
    pub(crate) measurements: Measurements,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct QuotaOverview {
    pub(crate) scope: &'static str,
    pub(crate) decision: GenerationAvailability,
    pub(crate) counters: Option<QuotaCounters>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct QuotaCounters {
    #[serde(serialize_with = "measurements::decimal")]
    pub(crate) used_rows: i64,
    #[serde(serialize_with = "measurements::decimal")]
    pub(crate) max_rows: i64,
    #[serde(serialize_with = "measurements::decimal")]
    pub(crate) used_bytes: i64,
    #[serde(serialize_with = "measurements::decimal")]
    pub(crate) max_bytes: i64,
    #[serde(serialize_with = "measurements::decimal")]
    pub(crate) spent_and_pending_logical_microunits: i64,
    #[serde(serialize_with = "measurements::decimal")]
    pub(crate) max_logical_microunits: i64,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct PressureObservation {
    pub(crate) state: StoragePressure,
    pub(crate) measurement_scope: &'static str,
    pub(crate) observed_at_ts: Option<i64>,
    pub(crate) revision: Option<i64>,
    pub(crate) observed_bytes: ByteMeasurement,
    pub(crate) configured_soft_limit_bytes: ByteMeasurement,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct Measurements {
    pub(crate) own_source_transcript_payload_bytes: ByteMeasurement,
    pub(crate) main_sqlite_allocated_page_bytes: ByteMeasurement,
    pub(crate) memory_domain_assigned_btree_bytes: ByteMeasurement,
    pub(crate) sqlite_freelist_page_bytes: ByteMeasurement,
    pub(crate) database_path_filesystem_available_bytes: ByteMeasurement,
}

/// The caller owns a read transaction and an authenticated principal. Return
/// the path separately so it cannot enter JSON; sample the filesystem only
/// after releasing the transaction. Global metrics are administrator-only.
pub(crate) fn observe_sqlite(
    db: &Connection,
    settings: MemoryEffectiveSettings,
    is_admin: bool,
    soft_limit: u64,
    sampled_at_ts: i64,
) -> anyhow::Result<(MemoryAvailability, Option<String>)> {
    super::ux::require_memory_ux_schema(db)?;
    let owner = &settings.target_principal_id;
    let quota = super::capacity::principal_quota_snapshot(db, owner)?;
    let quota_decision = quota.availability();
    let counters = match quota {
        PrincipalQuotaSnapshot::Known(
            [used_rows, max_rows, used_bytes, max_bytes, spent, max_cost],
        ) => Some(QuotaCounters {
            used_rows,
            max_rows,
            used_bytes,
            max_bytes,
            spent_and_pending_logical_microunits: spent,
            max_logical_microunits: max_cost,
        }),
        _ => None,
    };
    let state = StoragePressure::load(db)?;
    let storage_decision = state.generation_availability();
    let capacity_decision = if storage_decision == GenerationAvailability::Available {
        quota_decision
    } else {
        storage_decision
    };
    let recorded = db.query_row(
        "SELECT observed_bytes,revision,updated_at_ts FROM memory_storage_pressure WHERE singleton_id=1", [],
        |row| {
            let integer = |index| -> rusqlite::Result<Option<i64>> {
                Ok(match row.get_ref(index)? { ValueRef::Integer(value) if value >= 0 => Some(value), _ => None })
            };
            Ok((integer(0)?,integer(1)?,integer(2)?))
        },
    ).optional()?;
    let (bytes, revision, at) = recorded.unwrap_or((None, None, None));
    let source_bytes = match db.query_row(
        "SELECT COALESCE(SUM(LENGTH(CAST(content AS BLOB))),0) FROM memories WHERE principal_id=?1",
        [owner],
        |row| row.get::<_, u64>(0),
    ) {
        Ok(bytes) => ByteMeasurement::measured(bytes),
        Err(_) => ByteMeasurement::unavailable(MeasurementUnavailable::QueryFailed),
    };
    let unavailable = || ByteMeasurement::unavailable(MeasurementUnavailable::InvalidValue);
    let (observed_bytes, limit, main, domain, freelist, host, path) = if is_admin {
        (
            bytes
                .map(|value| ByteMeasurement::measured(value as u64))
                .unwrap_or_else(unavailable),
            ByteMeasurement::measured(soft_limit),
            measurements::allocated_pages(db, "PRAGMA page_count"),
            measurements::memory_btrees(db),
            measurements::allocated_pages(db, "PRAGMA freelist_count"),
            ByteMeasurement::unavailable(MeasurementUnavailable::NotSampled),
            measurements::database_path(db),
        )
    } else {
        (
            ByteMeasurement::AdminOnly,
            ByteMeasurement::AdminOnly,
            ByteMeasurement::AdminOnly,
            ByteMeasurement::AdminOnly,
            ByteMeasurement::AdminOnly,
            ByteMeasurement::AdminOnly,
            None,
        )
    };
    Ok((
        MemoryAvailability {
            schema_version: 1,
            sampled_at_ts,
            settings,
            capacity_decision,
            quota: QuotaOverview {
                scope: "source_transcript_content_rows_and_bytes",
                decision: quota_decision,
                counters,
            },
            pressure: PressureObservation {
                state,
                measurement_scope:
                    "main_sqlite_allocated_pages_not_memory_domain_or_host_free_space",
                observed_at_ts: at.filter(|value| *value > 0),
                revision,
                observed_bytes,
                configured_soft_limit_bytes: limit,
            },
            measurements: Measurements {
                own_source_transcript_payload_bytes: source_bytes,
                main_sqlite_allocated_page_bytes: main,
                memory_domain_assigned_btree_bytes: domain,
                sqlite_freelist_page_bytes: freelist,
                database_path_filesystem_available_bytes: host,
            },
        },
        path,
    ))
}

#[cfg(test)]
#[path = "availability_tests.rs"]
mod tests;
