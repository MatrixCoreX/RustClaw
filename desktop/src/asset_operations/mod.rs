pub(crate) mod client;
#[cfg(test)]
mod client_tests;
#[cfg(feature = "gui")]
pub mod commands;
pub(crate) mod direct;
#[cfg(test)]
mod direct_tests;
pub(crate) mod history;
pub(crate) mod node_selection;
pub(crate) mod nodes;
pub(crate) mod owner_transport;
pub mod protocol;
#[cfg(feature = "gui")]
pub mod standalone;
#[cfg(test)]
#[path = "tests.rs"]
mod tests;
use crate::{
    wallet::{files, Account},
    Result,
};
use protocol::{Capabilities, Intent, Payload, Service};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

pub(crate) fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|n| n.as_secs() as i64)
        .unwrap_or(0)
}

#[derive(Clone)]
pub struct Pending {
    pub generation: u64,
    pub session_id: Uuid,
    pub profile_id: Uuid,
    pub account: Account,
    pub cap: Capabilities,
    pub intent: Intent,
    pub payload: Payload,
    pub bytes: String,
    pub device_label: String,
    pub origin: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub operation_id: Uuid,
    pub profile_id: Uuid,
    pub account_id: Uuid,
    pub public_key: String,
    pub ledger_id: String,
    pub node_url: String,
    pub service: Service,
    pub kind: String,
    pub status: String,
    pub created_at_unix: i64,
    pub receipt_id: Option<String>,
}
pub struct Operations {
    pub pending: Option<Pending>,
    pub records: Vec<Record>,
    path: PathBuf,
}
impl Operations {
    pub fn new(path: PathBuf) -> Result<Self> {
        let records: Vec<Record> = if path.exists() {
            serde_json::from_slice(&files::read_private(&path)?)
                .map_err(|_| "wallet_history_invalid")?
        } else {
            vec![]
        };
        if records.len() > 500 {
            return Err("wallet_history_invalid".into());
        }
        Ok(Self {
            pending: None,
            records,
            path,
        })
    }
    pub fn persist(&self) -> Result<()> {
        files::write(
            &self.path,
            &serde_json::to_vec(&self.records).map_err(|_| "wallet_history_invalid")?,
        )
    }
    pub fn mark_submitted(&mut self, p: &Pending) -> Result<()> {
        // Keep every unresolved operation; prune only settled local summaries.
        if self.records.len() >= 500 {
            if let Some(index) = self.records.iter().position(|r| r.status != "pending") {
                self.records.remove(index);
            } else {
                return Err("wallet_history_limit".into());
            }
        }
        self.records.push(Record {
            operation_id: p.payload.operation_id,
            profile_id: p.profile_id,
            account_id: p.account.id,
            public_key: p.account.public_key.clone(),
            ledger_id: p.cap.ledger_id.clone(),
            node_url: p.cap.node_url.clone(),
            service: p.cap.service,
            kind: p.intent.action().into(),
            status: "pending".into(),
            created_at_unix: now(),
            receipt_id: None,
        });
        self.persist()
    }
}
