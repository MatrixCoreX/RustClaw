use std::path::Path;

use rusqlite::Connection;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum ByteMeasurement {
    Measured {
        #[serde(serialize_with = "decimal")]
        bytes: u64,
    },
    Unavailable {
        reason: MeasurementUnavailable,
    },
    AdminOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MeasurementUnavailable {
    QueryFailed,
    InvalidValue,
    UnsupportedDbstat,
    InMemoryDatabase,
    FilesystemUnavailable,
    NotSampled,
}

impl ByteMeasurement {
    pub(super) fn measured(bytes: u64) -> Self {
        Self::Measured { bytes }
    }
    pub(super) fn unavailable(reason: MeasurementUnavailable) -> Self {
        Self::Unavailable { reason }
    }
}

// JSON numbers cannot preserve all SQLite i64/u64 observations in JavaScript.
// A decimal string is an observation format, not a new capacity limit.
pub(super) fn decimal<T: std::fmt::Display, S: serde::Serializer>(
    value: &T,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.collect_str(value)
}

pub(super) fn allocated_pages(db: &Connection, count_pragma: &str) -> ByteMeasurement {
    let value = || -> rusqlite::Result<Option<u64>> {
        let count: u64 = db.query_row(count_pragma, [], |row| row.get(0))?;
        let size: u64 = db.query_row("PRAGMA page_size", [], |row| row.get(0))?;
        Ok(count.checked_mul(size))
    };
    match value() {
        Ok(Some(bytes)) => ByteMeasurement::measured(bytes),
        Ok(None) => ByteMeasurement::unavailable(MeasurementUnavailable::InvalidValue),
        Err(_) => ByteMeasurement::unavailable(MeasurementUnavailable::QueryFailed),
    }
}

/// Aggregate-mode DBSTAT scans the memory-domain btrees, including their
/// indexes/FTS shadows. It excludes freelists, WAL/SHM and non-btree overhead;
/// this is neither per-owner physical usage nor filesystem free space.
pub(crate) fn memory_btrees(db: &Connection) -> ByteMeasurement {
    let supported = db.query_row(
        "SELECT sqlite_compileoption_used('ENABLE_DBSTAT_VTAB')",
        [],
        |row| row.get::<_, bool>(0),
    );
    match supported {
        Ok(false) => {
            return ByteMeasurement::unavailable(MeasurementUnavailable::UnsupportedDbstat)
        }
        Err(_) => return ByteMeasurement::unavailable(MeasurementUnavailable::QueryFailed),
        Ok(true) => {}
    }
    let value = || -> rusqlite::Result<Option<u64>> {
        let mut statement = db.prepare(
            "SELECT pgsize FROM dbstat('main',1) WHERE name IN (
                 SELECT name FROM sqlite_schema WHERE
                     substr(tbl_name,1,7)='memory_'
                     OR tbl_name IN ('memories','long_term_memories','user_preferences')
             )",
        )?;
        let mut rows = statement.query([])?;
        let mut bytes = 0_u64;
        while let Some(row) = rows.next()? {
            let Some(total) = bytes.checked_add(row.get::<_, u64>(0)?) else {
                return Ok(None);
            };
            bytes = total;
        }
        Ok(Some(bytes))
    };
    match value() {
        Ok(Some(bytes)) => ByteMeasurement::measured(bytes),
        Ok(None) => ByteMeasurement::unavailable(MeasurementUnavailable::InvalidValue),
        Err(_) => ByteMeasurement::unavailable(MeasurementUnavailable::QueryFailed),
    }
}

pub(super) fn database_path(db: &Connection) -> Option<String> {
    db.query_row(
        "SELECT file FROM pragma_database_list WHERE name='main'",
        [],
        |row| row.get(0),
    )
    .ok()
}

/// Called only after releasing the SQLite read transaction/connection. This
/// measures the filesystem currently containing the database path; it is not
/// descriptor-pinned admission authority and never falls back to `/`.
pub(crate) fn filesystem_available(path: Option<&str>) -> ByteMeasurement {
    match path {
        Some("") => ByteMeasurement::unavailable(MeasurementUnavailable::InMemoryDatabase),
        Some(path) => match fs2::available_space(Path::new(path)) {
            Ok(bytes) => ByteMeasurement::measured(bytes),
            Err(_) => ByteMeasurement::unavailable(MeasurementUnavailable::FilesystemUnavailable),
        },
        None => ByteMeasurement::unavailable(MeasurementUnavailable::QueryFailed),
    }
}
