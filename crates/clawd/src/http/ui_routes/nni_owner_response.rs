use super::RequestEnvelope;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    ok: bool,
    data: Data,
    error: Option<()>,
}

#[derive(Deserialize, Serialize)]
#[serde(untagged)]
enum Data {
    Capabilities(Capabilities),
    Challenge(Challenge),
    Read(Read),
    Outcome(Outcome),
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Capabilities {
    schema_version: u32,
    protocol: String,
    ledger_id: String,
    node_url: String,
    service: String,
    actions: Vec<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Challenge {
    signing_payload: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Read {
    account: String,
    ledger_id: String,
    node_url: String,
    aic_balance_units: String,
    usd_balance_units: String,
    page: u32,
    total_pages: u32,
    records: Vec<Entry>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    operation_id: Uuid,
    kind: String,
    asset: String,
    amount_units: String,
    counterparty: Option<String>,
    created_at_unix: i64,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Outcome {
    operation_id: Uuid,
    account: String,
    ledger_id: String,
    status: String,
    receipt_id: Option<String>,
}

fn units(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 19
        && value.bytes().all(|b| b.is_ascii_digit())
        && (value == "0" || !value.starts_with('0'))
        && value.parse::<i64>().is_ok()
}

pub(super) fn project(
    bytes: &[u8],
    suffix: &str,
    service: &str,
    origin: &str,
    request: Option<&RequestEnvelope>,
) -> Result<Vec<u8>, ()> {
    let envelope: Envelope = serde_json::from_slice(bytes).map_err(|_| ())?;
    if !envelope.ok {
        return Err(());
    }
    let valid = match &envelope.data {
        Data::Capabilities(cap) => {
            suffix == "capabilities"
                && cap.schema_version == 1
                && cap.protocol == "asset_owner_v1"
                && !cap.ledger_id.is_empty()
                && cap.ledger_id.len() <= 128
                && !cap.ledger_id.chars().any(char::is_control)
                && cap.node_url == origin
                && cap.service == service
                && !cap.actions.is_empty()
                && cap.actions.len() <= 5
                && cap.actions.iter().enumerate().all(|(index, action)| {
                    !cap.actions[..index].contains(action)
                        && (matches!(action.as_str(), "balances" | "history" | "operation_status")
                            || (service == "assets" && action == "transfer")
                            || (service == "bancor" && action == "bancor_trade"))
                })
        }
        Data::Challenge(challenge) => {
            suffix.ends_with("/request")
                && !challenge.signing_payload.is_empty()
                && challenge.signing_payload.len() <= 8192
        }
        Data::Read(read) => {
            matches!(suffix, "read/verify" | "read/public")
                && request
                    .is_some_and(|r| r.account == read.account && r.ledger_id == read.ledger_id)
                && read.node_url == origin
                && (1..=100_000).contains(&read.page)
                && (1..=100_000).contains(&read.total_pages)
                && units(&read.aic_balance_units)
                && units(&read.usd_balance_units)
                && read.records.len() <= 20
                && read.records.iter().all(|entry| {
                    !entry.operation_id.is_nil()
                        && entry.created_at_unix >= 0
                        && matches!(
                            entry.kind.as_str(),
                            "transfer_in" | "transfer_out" | "bancor_buy" | "bancor_sell"
                        )
                        && matches!(entry.asset.as_str(), "AIC" | "USD")
                        && units(&entry.amount_units)
                        && entry.amount_units != "0"
                        && entry.counterparty.as_ref().is_none_or(|key| {
                            super::super::normalize_nni_owner_public_key(key).is_ok()
                        })
                })
        }
        Data::Outcome(outcome) => {
            (suffix.ends_with("/verify") || suffix == "read/public")
                && request.is_some_and(|r| {
                    r.account == outcome.account
                        && r.ledger_id == outcome.ledger_id
                        && (matches!(suffix, "read/verify" | "read/public") || r.operation_id == outcome.operation_id)
                })
                && !outcome.operation_id.is_nil()
                && matches!(
                    outcome.status.as_str(),
                    "succeeded" | "failed" | "pending" | "expired"
                )
                && (outcome.status == "succeeded") == outcome.receipt_id.is_some()
                && outcome.receipt_id.as_ref().is_none_or(|id| {
                    !id.is_empty() && id.len() <= 128 && !id.chars().any(char::is_control)
                })
        }
    };
    if !valid {
        return Err(());
    }
    // Only the envelope is serialized. The challenge remains the identical
    // decoded UTF-8 string; the desktop validates and signs those original bytes.
    serde_json::to_vec(&envelope).map_err(|_| ())
}
