use crate::{wallet::keys, Result};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const PROTOCOL: &str = "asset_owner_v1";
pub const API: &str = "/v1/nni/assets/owner";
pub const SCALE: u64 = 100_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Service {
    Assets,
    Bancor,
}
impl Service {
    pub fn name(self) -> &'static str {
        match self {
            Self::Assets => "assets",
            Self::Bancor => "bancor",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Intent {
    Balances,
    History {
        page: u32,
    },
    OperationStatus {
        operation_id: Uuid,
    },
    BancorTrade {
        side: String,
        input_units: String,
        slippage_bps: u16,
        max_fee_bps: u16,
    },
    Transfer {
        asset: String,
        amount_units: String,
        recipient: String,
        memo: String,
        max_fee_bps: u16,
    },
}
impl Intent {
    pub fn write(&self) -> bool {
        matches!(self, Self::BancorTrade { .. } | Self::Transfer { .. })
    }
    pub fn action(&self) -> &'static str {
        match self {
            Self::Balances => "balances",
            Self::History { .. } => "history",
            Self::OperationStatus { .. } => "operation_status",
            Self::BancorTrade { .. } => "bancor_trade",
            Self::Transfer { .. } => "transfer",
        }
    }
    pub fn validate(&self, service: Service, account: &str) -> Result<()> {
        match self {
            Self::Balances => {}
            Self::History { page } if *page > 0 && *page <= 100_000 => {}
            Self::OperationStatus { operation_id } if !operation_id.is_nil() => {}
            Self::BancorTrade {
                side,
                input_units,
                slippage_bps,
                max_fee_bps,
            } if service == Service::Bancor
                && matches!(side.as_str(), "buy" | "sell")
                && *slippage_bps <= 5_000
                && *max_fee_bps <= 5_000 =>
            {
                positive_units(input_units)?;
            }
            Self::Transfer {
                asset,
                amount_units,
                recipient,
                memo,
                max_fee_bps,
            } if service == Service::Assets
                && matches!(asset.as_str(), "AIC" | "USD")
                && memo.len() <= 256
                && *max_fee_bps <= 5_000
                && recipient != account =>
            {
                positive_units(amount_units)?;
                keys::validate_public(recipient)?;
            }
            _ => return Err("wallet_intent_invalid".into()),
        }
        Ok(())
    }
}

pub fn units(s: &str) -> Result<u64> {
    if s.is_empty()
        || s.len() > 19
        || !s.bytes().all(|b| b.is_ascii_digit())
        || (s.len() > 1 && s.starts_with('0'))
    {
        return Err("wallet_amount_invalid".into());
    }
    let n = s.parse::<u64>().map_err(|_| "wallet_amount_invalid")?;
    if n > i64::MAX as u64 {
        return Err("wallet_amount_invalid".into());
    }
    Ok(n)
}
pub fn positive_units(s: &str) -> Result<u64> {
    let n = units(s)?;
    if n == 0 {
        Err("wallet_amount_invalid".into())
    } else {
        Ok(n)
    }
}
pub fn format_units(s: &str) -> String {
    match units(s) {
        Ok(n) => format!("{}.{:08}", n / SCALE, n % SCALE),
        Err(_) => "—".into(),
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capabilities {
    pub schema_version: u32,
    pub protocol: String,
    pub ledger_id: String,
    pub node_url: String,
    pub service: Service,
    pub actions: Vec<String>,
}
impl Capabilities {
    pub fn validate(&self, service: Service, action: &str) -> Result<()> {
        if self.schema_version != 1
            || self.protocol != PROTOCOL
            || self.service != service
            || self.ledger_id.is_empty()
            || self.ledger_id.len() > 128
            || self.ledger_id.chars().any(char::is_control)
            || !self.actions.iter().any(|a| a == action)
            || self.actions.len() > 5
            || self.actions.iter().enumerate().any(|(index, value)| {
                self.actions[..index].contains(value)
                    || !matches!(
                        value.as_str(),
                        "balances" | "history" | "operation_status" | "transfer" | "bancor_trade"
                    )
                    || (value == "transfer" && service != Service::Assets)
                    || (value == "bancor_trade" && service != Service::Bancor)
            })
        {
            return Err("wallet_backend_unsupported".into());
        }
        let url = reqwest::Url::parse(&self.node_url).map_err(|_| "wallet_node_invalid")?;
        let loopback = matches!(url.host_str(), Some("127.0.0.1" | "[::1]" | "::1"));
        if !(url.scheme() == "https" || (url.scheme() == "http" && loopback))
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.path() != "/"
            || self.node_url != url.origin().ascii_serialization()
        {
            return Err("wallet_node_invalid".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Terms {
    Balances,
    History {
        page: u32,
    },
    OperationStatus {
        operation_id: Uuid,
    },
    BancorTrade {
        side: String,
        input_units: String,
        slippage_bps: u16,
        max_fee_bps: u16,
        fee_units: String,
        quoted_output_units: String,
        min_output_units: String,
    },
    Transfer {
        asset: String,
        amount_units: String,
        recipient: String,
        memo: String,
        max_fee_bps: u16,
        fee_units: String,
    },
}
impl Terms {
    fn validate(&self, intent: &Intent) -> Result<()> {
        let valid = match (self, intent) {
            (Self::Balances, Intent::Balances) => true,
            (Self::History { page: a }, Intent::History { page: b }) => a == b,
            (
                Self::OperationStatus { operation_id: a },
                Intent::OperationStatus { operation_id: b },
            ) => a == b,
            (
                Self::BancorTrade {
                    side,
                    input_units,
                    slippage_bps,
                    max_fee_bps,
                    fee_units,
                    quoted_output_units,
                    min_output_units,
                },
                Intent::BancorTrade {
                    side: s,
                    input_units: i,
                    slippage_bps: l,
                    max_fee_bps: f,
                },
            ) => {
                let input = positive_units(input_units)? as u128;
                let fee = units(fee_units)? as u128;
                let quote = positive_units(quoted_output_units)? as u128;
                let min = positive_units(min_output_units)? as u128;
                side == s
                    && input_units == i
                    && slippage_bps == l
                    && max_fee_bps == f
                    && fee <= input * (*f as u128) / 10_000
                    && fee < input
                    && min <= quote
                    && min >= quote * (10_000 - *l as u128) / 10_000
            }
            (
                Self::Transfer {
                    asset,
                    amount_units,
                    recipient,
                    memo,
                    max_fee_bps,
                    fee_units,
                },
                Intent::Transfer {
                    asset: a,
                    amount_units: n,
                    recipient: r,
                    memo: m,
                    max_fee_bps: f,
                },
            ) => {
                let amount = positive_units(amount_units)? as u128;
                let fee = units(fee_units)? as u128;
                asset == a
                    && amount_units == n
                    && recipient == r
                    && memo == m
                    && max_fee_bps == f
                    && fee <= amount * (*f as u128) / 10_000
                    && amount + fee <= i64::MAX as u128
            }
            _ => false,
        };
        if valid {
            Ok(())
        } else {
            Err("wallet_challenge_mismatch".into())
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Payload {
    pub schema_version: u32,
    pub protocol: String,
    pub ledger_id: String,
    pub node_url: String,
    pub service: Service,
    pub account: String,
    pub operation_id: Uuid,
    pub challenge_id: Uuid,
    pub nonce: String,
    pub expires_at_unix: i64,
    pub terms: Terms,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Challenge {
    pub signing_payload: String,
}
pub fn validate_challenge(
    raw: &str,
    cap: &Capabilities,
    account: &str,
    id: Uuid,
    intent: &Intent,
    now: i64,
) -> Result<Payload> {
    if raw.len() > 8192 {
        return Err("wallet_challenge_invalid".into());
    }
    // Deserialize the original string directly: duplicate and unknown fields fail.
    let p: Payload = serde_json::from_str(raw).map_err(|_| "wallet_challenge_invalid")?;
    if p.schema_version != 1
        || p.protocol != PROTOCOL
        || p.ledger_id != cap.ledger_id
        || p.node_url != cap.node_url
        || p.service != cap.service
        || p.account != account
        || p.operation_id != id
        || p.challenge_id.is_nil()
        || p.nonce.len() != 64
        || !p
            .nonce
            .bytes()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        || p.expires_at_unix <= now
        || p.expires_at_unix > now.saturating_add(300)
    {
        return Err("wallet_challenge_mismatch".into());
    }
    p.terms.validate(intent)?;
    Ok(p)
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryEntry {
    pub operation_id: Uuid,
    pub kind: String,
    pub asset: String,
    pub amount_units: String,
    pub counterparty: Option<String>,
    pub created_at_unix: i64,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadResult {
    pub account: String,
    pub ledger_id: String,
    pub node_url: String,
    pub aic_balance_units: String,
    pub usd_balance_units: String,
    pub page: u32,
    pub total_pages: u32,
    pub records: Vec<HistoryEntry>,
}
impl ReadResult {
    pub fn validate(&self, p: &Payload) -> Result<()> {
        let page = match p.terms {
            Terms::History { page } => page,
            _ => 1,
        };
        self.validate_context(&p.account, &p.ledger_id, &p.node_url, page)
    }
    pub fn validate_public(&self, cap: &Capabilities, account: &str, page: u32) -> Result<()> {
        self.validate_context(account, &cap.ledger_id, &cap.node_url, page)
    }
    fn validate_context(&self, account: &str, ledger: &str, node: &str, page: u32) -> Result<()> {
        if self.account != account
            || self.ledger_id != ledger
            || self.node_url != node
            || self.page != page
            || self.total_pages > 100_000
            || self.total_pages == 0
            || self.records.len() > 20
        {
            return Err("wallet_response_invalid".into());
        }
        units(&self.aic_balance_units)?;
        units(&self.usd_balance_units)?;
        for r in &self.records {
            if !matches!(
                r.kind.as_str(),
                "bancor_buy" | "bancor_sell" | "transfer_in" | "transfer_out"
            ) || !matches!(r.asset.as_str(), "AIC" | "USD")
                || r.operation_id.is_nil()
                || r.created_at_unix < 0
            {
                return Err("wallet_response_invalid".into());
            }
            positive_units(&r.amount_units)?;
            if matches!(r.kind.as_str(), "transfer_in" | "transfer_out")
                && r.counterparty.as_deref() == Some(&self.account)
            {
                return Err("wallet_response_invalid".into());
            }
            if let Some(public) = &r.counterparty {
                keys::validate_public(public)?;
            }
        }
        Ok(())
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outcome {
    pub operation_id: Uuid,
    pub account: String,
    pub ledger_id: String,
    pub status: String,
    pub receipt_id: Option<String>,
}
impl Outcome {
    pub fn validate(&self, id: Uuid, account: &str, ledger: &str) -> Result<()> {
        if self.operation_id != id
            || self.account != account
            || self.ledger_id != ledger
            || !matches!(
                self.status.as_str(),
                "succeeded" | "failed" | "pending" | "expired"
            )
            || self
                .receipt_id
                .as_ref()
                .is_some_and(|s| s.is_empty() || s.len() > 128 || s.chars().any(char::is_control))
            || (self.status == "succeeded") != self.receipt_id.is_some()
        {
            Err("wallet_response_invalid".into())
        } else {
            Ok(())
        }
    }
}
