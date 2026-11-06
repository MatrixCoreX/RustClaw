//! Read-only capacity decisions. Initialization belongs to the startup/write
//! paths; a missing policy row is never repaired or treated as an allow here.
use rusqlite::{types::ValueRef, Connection, OptionalExtension};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StoragePressure {
    Normal,
    DerivedCleanup,
    BackfillPaused,
    AutomaticGenerationPaused,
    ExplicitWriteBlocked,
    Missing,
    Unknown,
}

impl StoragePressure {
    pub(crate) fn load(db: &Connection) -> anyhow::Result<Self> {
        let state = db
            .query_row(
                "SELECT state FROM memory_storage_pressure WHERE singleton_id = 1",
                [],
                |row| {
                    let persisted = row.get_ref(0)?;
                    Ok(match persisted.as_str().ok() {
                        Some("normal") => Self::Normal,
                        Some("derived_cleanup") => Self::DerivedCleanup,
                        Some("backfill_paused") => Self::BackfillPaused,
                        Some("automatic_generation_paused") => Self::AutomaticGenerationPaused,
                        Some("explicit_write_blocked") => Self::ExplicitWriteBlocked,
                        _ => Self::Unknown,
                    })
                },
            )
            .optional()?;
        Ok(state.unwrap_or(Self::Missing))
    }

    pub(crate) fn generation_availability(self) -> GenerationAvailability {
        match self {
            Self::Normal | Self::DerivedCleanup | Self::BackfillPaused => {
                GenerationAvailability::Available
            }
            Self::AutomaticGenerationPaused | Self::ExplicitWriteBlocked => {
                GenerationAvailability::StoragePaused
            }
            Self::Missing => GenerationAvailability::StorageStateMissing,
            Self::Unknown => GenerationAvailability::StorageStateUnknown,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum GenerationAvailability {
    Available,
    StoragePaused,
    StorageStateMissing,
    StorageStateUnknown,
    PrincipalQuotaMissing,
    PrincipalQuotaInvalid,
    PrincipalRowsExhausted,
    PrincipalBytesExhausted,
    PrincipalBackgroundCostExhausted,
}

/// Rows/bytes retain their existing `memories` source-text scope. Background
/// cost is the existing logical work counter, not provider-token/USD billing.
/// Neither is a measurement of all memory-domain data or host free space.
pub(crate) fn automatic_generation_availability(
    db: &Connection,
    principal_id: &str,
) -> anyhow::Result<GenerationAvailability> {
    let storage = StoragePressure::load(db)?.generation_availability();
    if storage != GenerationAvailability::Available {
        return Ok(storage);
    }
    principal_generation_availability(db, principal_id)
}

pub(super) fn principal_generation_availability(
    db: &Connection,
    principal_id: &str,
) -> anyhow::Result<GenerationAvailability> {
    let quota = db
        .query_row(
            "SELECT used_rows, max_rows, used_bytes, max_bytes,
                    used_background_cost_microunits + COALESCE((
                        SELECT SUM(background_cost_pending_microunits) FROM memory_jobs
                        WHERE memory_jobs.principal_id = memory_principal_quotas.principal_id
                          AND background_cost_pending_microunits > 0
                    ), 0), max_background_cost_microunits
             FROM memory_principal_quotas WHERE principal_id = ?1",
            [principal_id],
            |row| {
                let mut values = [0_i64; 6];
                for (index, value) in values.iter_mut().enumerate() {
                    let ValueRef::Integer(number) = row.get_ref(index)? else {
                        return Ok(None);
                    };
                    *value = number;
                }
                Ok(Some(values))
            },
        )
        .optional()?;
    Ok(match quota {
        None => GenerationAvailability::PrincipalQuotaMissing,
        Some(None) => GenerationAvailability::PrincipalQuotaInvalid,
        Some(Some(values)) if values.iter().any(|value| *value < 0) => {
            GenerationAvailability::PrincipalQuotaInvalid
        }
        Some(Some([used, limit, ..])) if used >= limit => {
            GenerationAvailability::PrincipalRowsExhausted
        }
        Some(Some([_, _, used, limit, ..])) if used >= limit => {
            GenerationAvailability::PrincipalBytesExhausted
        }
        Some(Some([_, _, _, _, used, limit])) if used >= limit => {
            GenerationAvailability::PrincipalBackgroundCostExhausted
        }
        Some(_) => GenerationAvailability::Available,
    })
}

#[cfg(test)]
#[path = "capacity_decision_tests.rs"]
mod tests;
