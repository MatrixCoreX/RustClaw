//! Shared read-only decision for UI observations and transactional admission.
//! An observation is not consent, permission, provider readiness or a write lock.
use rusqlite::Connection;
use serde::Serialize;

use super::availability::ByteMeasurement;
use super::capacity::StoragePressure;
use super::storage_budget::{ReadError, Snapshot};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum GrowthDecision {
    Available,
    StorageStateMissing,
    StorageStateUnknown,
    StorageStateUnavailable,
    StoragePolicyMissing,
    StoragePolicyInvalid,
    StoragePolicyUnavailable,
    MeasurementUnavailable,
    DomainBudgetReached,
    ExplicitStopUnbound,
}

impl GrowthDecision {
    fn code(self) -> Option<&'static str> {
        Some(match self {
            Self::Available => return None,
            Self::StorageStateMissing => "memory_storage_state_missing",
            Self::StorageStateUnknown => "memory_storage_state_unknown",
            Self::StorageStateUnavailable => "memory_storage_state_unavailable",
            Self::StoragePolicyMissing => ReadError::Missing.code(),
            Self::StoragePolicyInvalid => ReadError::Invalid.code(),
            Self::StoragePolicyUnavailable => ReadError::Unavailable.code(),
            Self::MeasurementUnavailable => "memory_storage_measurement_unavailable",
            Self::DomainBudgetReached | Self::ExplicitStopUnbound => "memory_storage_write_blocked",
        })
    }
}

#[derive(Clone, Copy)]
pub(super) enum ProbePhase {
    BeforeGrowth,
    AfterGrowth,
}

pub(super) struct GrowthProbe {
    pub(super) decision: GrowthDecision,
    pub(super) snapshot: Option<Snapshot>,
}

impl GrowthProbe {
    pub(super) fn validated_snapshot(&self) -> anyhow::Result<Snapshot> {
        if matches!(
            self.decision,
            GrowthDecision::StorageStateMissing
                | GrowthDecision::StorageStateUnknown
                | GrowthDecision::StorageStateUnavailable
                | GrowthDecision::StoragePolicyMissing
                | GrowthDecision::StoragePolicyInvalid
                | GrowthDecision::StoragePolicyUnavailable
        ) {
            anyhow::bail!(self
                .decision
                .code()
                .unwrap_or(ReadError::Unavailable.code()));
        }
        self.snapshot
            .ok_or_else(|| anyhow::anyhow!(ReadError::Unavailable))
    }

    pub(super) fn admit(self) -> anyhow::Result<Snapshot> {
        let snapshot = self.validated_snapshot()?;
        if let Some(code) = self.decision.code() {
            anyhow::bail!(code);
        }
        Ok(snapshot)
    }
}

/// Sampling is lazy: damaged state/policy never causes a full btree scan just
/// to reject growth. An availability view may reuse its existing measurement.
pub(super) fn probe(
    db: &Connection,
    phase: ProbePhase,
    measure: impl FnOnce() -> ByteMeasurement,
) -> GrowthProbe {
    let state = match StoragePressure::load(db) {
        Ok(StoragePressure::Missing) => {
            return GrowthProbe {
                decision: GrowthDecision::StorageStateMissing,
                snapshot: None,
            }
        }
        Ok(StoragePressure::Unknown) => {
            return GrowthProbe {
                decision: GrowthDecision::StorageStateUnknown,
                snapshot: None,
            }
        }
        Err(_) => {
            return GrowthProbe {
                decision: GrowthDecision::StorageStateUnavailable,
                snapshot: None,
            }
        }
        Ok(state) => state,
    };
    let snapshot = match Snapshot::load(db) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            return GrowthProbe {
                decision: match error {
                    ReadError::Missing => GrowthDecision::StoragePolicyMissing,
                    ReadError::Invalid => GrowthDecision::StoragePolicyInvalid,
                    ReadError::Unavailable => GrowthDecision::StoragePolicyUnavailable,
                },
                snapshot: None,
            }
        }
    };
    let decision = if state == StoragePressure::ExplicitWriteBlocked {
        match db.query_row(
            "SELECT revision FROM memory_storage_pressure WHERE singleton_id=1",
            [],
            |row| row.get::<_, i64>(0),
        ) {
            Ok(revision) if revision == snapshot.pressure_revision => None,
            Ok(_) => Some(GrowthDecision::ExplicitStopUnbound),
            Err(_) => Some(GrowthDecision::StorageStateUnknown),
        }
    } else {
        None
    };
    let decision = decision.unwrap_or_else(|| match measure() {
        ByteMeasurement::Measured { bytes } => {
            let reached = match phase {
                ProbePhase::BeforeGrowth => bytes >= snapshot.limit,
                ProbePhase::AfterGrowth => bytes > snapshot.limit,
            };
            if reached {
                GrowthDecision::DomainBudgetReached
            } else {
                GrowthDecision::Available
            }
        }
        _ => GrowthDecision::MeasurementUnavailable,
    });
    GrowthProbe {
        decision,
        snapshot: Some(snapshot),
    }
}
