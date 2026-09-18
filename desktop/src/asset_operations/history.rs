use super::protocol::{format_units, units};
use crate::{wallet::keys, Result};
use serde_json::{json, Value};

pub struct HistoryRequest {
    pub owner: String,
    page: u64,
    source: String,
    direction: String,
}
fn invalid<T>() -> Result<T> {
    Err("wallet_response_invalid".into())
}
impl HistoryRequest {
    pub fn parse(path: &str) -> Result<Option<Self>> {
        if !path.starts_with("/v1/nni/assets/transfers?") {
            return Ok(None);
        }
        let url = reqwest::Url::parse(&format!("http://localhost{path}"))
            .map_err(|_| "wallet_intent_invalid")?;
        let pairs: std::collections::HashMap<String, String> = url
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        if pairs.len() != 5
            || url.query_pairs().count() != 5
            || url.fragment().is_some()
            || pairs.get("limit").map(String::as_str) != Some("100")
        {
            return Err("wallet_intent_invalid".into());
        }
        let owner = pairs
            .get("owner_pubkey")
            .ok_or("wallet_intent_invalid")?
            .clone();
        keys::validate_public(&owner)?;
        let page = pairs
            .get("page")
            .ok_or("wallet_intent_invalid")?
            .parse::<u64>()
            .map_err(|_| "wallet_intent_invalid")?;
        let source = pairs.get("source").ok_or("wallet_intent_invalid")?.clone();
        let direction = pairs
            .get("direction")
            .ok_or("wallet_intent_invalid")?
            .clone();
        if !(1..=100_000).contains(&page)
            || !matches!(source.as_str(), "all" | "transfer" | "trade" | "issuance")
            || !matches!(direction.as_str(), "all" | "incoming" | "outgoing")
        {
            return Err("wallet_intent_invalid".into());
        }
        Ok(Some(Self {
            owner,
            page,
            source,
            direction,
        }))
    }
    fn class(&self) -> Option<&'static str> {
        match self.source.as_str() {
            "transfer" => Some("peer_transfer"),
            "trade" => Some("market_trade"),
            "issuance" => Some("system_issuance"),
            _ => None,
        }
    }
    pub fn path(&self) -> String {
        let mut url =
            reqwest::Url::parse("http://localhost/v1/nni/server/explorer/transactions").unwrap();
        url.query_pairs_mut()
            .append_pair("address", &self.owner)
            .append_pair("page", &self.page.to_string())
            .append_pair("per_page", "100");
        if let Some(class) = self.class() {
            url.query_pairs_mut()
                .append_pair("transaction_class", class);
        }
        if self.direction != "all" {
            url.query_pairs_mut()
                .append_pair("direction", &self.direction);
        }
        format!("{}?{}", url.path(), url.query().unwrap_or_default())
    }
    pub fn project(&self, data: &Value) -> Result<Value> {
        let total = data["total"]
            .as_u64()
            .filter(|n| *n <= 9_007_199_254_740_991)
            .ok_or("wallet_response_invalid")?;
        if data["schema_version"] != 1
            || data["status"] != "explorer_transactions"
            || data["page"] != self.page
            || data["per_page"] != 100
            || data["total_pages"].as_u64() != Some(total.div_ceil(100).max(1))
        {
            return invalid();
        }
        let filter = data.get("filter").ok_or("wallet_response_invalid")?;
        let expected_class = self.class().unwrap_or("");
        let expected_direction = if self.direction == "all" {
            ""
        } else {
            &self.direction
        };
        if filter
            .get("transaction_class")
            .and_then(Value::as_str)
            .unwrap_or("")
            != expected_class
            || filter
                .get("direction")
                .and_then(Value::as_str)
                .unwrap_or("")
                != expected_direction
        {
            return invalid();
        }
        let rows = data["transactions"]
            .as_array()
            .filter(|r| r.len() <= 100)
            .ok_or("wallet_response_invalid")?;
        let mut transactions = Vec::new();
        for row in rows {
            let id = row["transaction_id"]
                .as_str()
                .filter(|v| {
                    !v.is_empty()
                        && v.len() <= 160
                        && v.bytes()
                            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b':' | b'_' | b'-'))
                })
                .ok_or("wallet_response_invalid")?;
            let kind = row["transaction_kind"]
                .as_str()
                .filter(|s| {
                    !s.is_empty()
                        && s.len() <= 64
                        && s.bytes()
                            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
                })
                .ok_or("wallet_response_invalid")?;
            let class = match kind {
                "asset_transfer" => "peer_transfer",
                "bancor_buy" | "bancor_sell" => "market_trade",
                "heartbeat_reward_credit" | "admin_usd_credit" | "market_bootstrap" => {
                    "system_issuance"
                }
                _ => "other",
            };
            if row["transaction_class"] != class || self.class().is_some_and(|c| c != class) {
                return invalid();
            }
            let timestamp = row["created_at_unix"]
                .as_u64()
                .filter(|t| *t <= 8_640_000_000)
                .ok_or("wallet_response_invalid")?;
            let memo = match row.get("memo") {
                None | Some(Value::Null) => None,
                Some(Value::String(s)) if s.len() <= 256 => Some(s),
                _ => return invalid(),
            };
            let raw_flows = row["flows"]
                .as_array()
                .filter(|r| !r.is_empty() && r.len() <= 8)
                .ok_or("wallet_response_invalid")?;
            let mut flows = Vec::new();
            for flow in raw_flows {
                let index = flow["flow_index"]
                    .as_u64()
                    .filter(|n| *n <= 100)
                    .ok_or("wallet_response_invalid")?;
                let asset = flow["asset"]
                    .as_str()
                    .filter(|s| matches!(*s, "AIC" | "USD"))
                    .ok_or("wallet_response_invalid")?;
                let amount = flow["amount_units"]
                    .as_str()
                    .ok_or("wallet_response_invalid")?;
                units(amount)?;
                if flow["amount"] != format_units(amount) {
                    return invalid();
                }
                let from = account(&flow["from"])?;
                let to = account(&flow["to"])?;
                let outgoing = from["address"] == self.owner;
                let incoming = to["address"] == self.owner;
                if match self.direction.as_str() {
                    "incoming" => incoming,
                    "outgoing" => outgoing,
                    _ => outgoing || incoming,
                } {
                    flows.push(json!({"flow_index":index,"asset":asset,"amount_units":amount,"amount":format_units(amount),"from":from,"to":to}));
                }
            }
            if flows.is_empty() {
                return invalid();
            }
            transactions.push(json!({"transaction_id":id,"transaction_kind":kind,"transaction_class":class,"created_at_unix":timestamp,"memo":memo,"flows":flows}));
        }
        Ok(
            json!({"schema_version":1,"status":"asset_transfer_history","owner_pubkey":self.owner,
            "page":self.page,"per_page":100,"total_transactions":total,"total_pages":data["total_pages"],
            "source_filter":self.source,"direction_filter":self.direction,"transactions":transactions}),
        )
    }
}
fn account(value: &Value) -> Result<Value> {
    let kind = value["account_kind"]
        .as_str()
        .ok_or("wallet_response_invalid")?;
    if kind == "system" && value["address"].is_null() {
        return Ok(json!({"account_kind":"system","address":null}));
    }
    if !matches!(kind, "asset_owner" | "pool" | "fee") {
        return invalid();
    }
    let address = value["address"].as_str().ok_or("wallet_response_invalid")?;
    keys::validate_public(address)?;
    Ok(json!({"account_kind":kind,"address":address}))
}
