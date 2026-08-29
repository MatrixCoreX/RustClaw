use super::*;

#[test]
fn history_projects_transfers_trades_and_system_issuance() {
    let owner = generate_nni_owner_key_pair().public_key;
    let sender = generate_nni_owner_key_pair().public_key;
    let recipient = generate_nni_owner_key_pair().public_key;
    let payload = json!({
        "schema_version": 1,
        "status": "explorer_transactions",
        "per_page": 100,
        "page": 1,
        "total": 3,
        "total_pages": 1,
        "filter": {
            "transaction_kind": null,
            "transaction_class": null,
            "direction": null,
        },
        "transactions": [
            {
                "transaction_id": "asset-transfer-outgoing",
                "transaction_kind": "asset_transfer",
                "transaction_class": "peer_transfer",
                "created_at_unix": 1_700_000_200,
                "memo": "invoice 7",
                "flows": [{
                    "flow_index": 0,
                    "asset": "AIC",
                    "amount_units": "125000000",
                    "amount": "1.25000000",
                    "from": {"account_kind": "asset_owner", "address": owner},
                    "to": {"account_kind": "asset_owner", "address": recipient},
                }],
            },
            {
                "transaction_id": "asset-transfer-incoming",
                "transaction_kind": "asset_transfer",
                "transaction_class": "peer_transfer",
                "created_at_unix": 1_700_000_100,
                "memo": null,
                "flows": [{
                    "flow_index": 0,
                    "asset": "USD",
                    "amount_units": "250000000",
                    "amount": "2.50000000",
                    "from": {"account_kind": "asset_owner", "address": sender},
                    "to": {"account_kind": "asset_owner", "address": owner},
                }],
            },
            {
                "transaction_id": "reward-incoming",
                "transaction_kind": "heartbeat_reward_credit",
                "transaction_class": "system_issuance",
                "created_at_unix": 1_700_000_000,
                "memo": null,
                "flows": [{
                    "flow_index": 0,
                    "asset": "AIC",
                    "amount_units": "500000000000",
                    "amount": "5000.00000000",
                    "from": {"account_kind": "system", "address": null},
                    "to": {"account_kind": "asset_owner", "address": owner},
                }],
            },
        ],
    });

    let normalized = normalize_asset_transfer_history_response(
        &payload,
        &owner,
        NniAssetHistorySourceFilter::All,
        NniAssetHistoryDirectionFilter::All,
    )
        .expect("valid address history should be projected");
    assert_eq!(normalized["status"], "asset_transfer_history");
    assert_eq!(normalized["owner_pubkey"], owner);
    assert_eq!(normalized["transactions"].as_array().unwrap().len(), 3);
    assert_eq!(
        normalized["transactions"][0]["flows"][0]["from"]["address"],
        owner
    );
    assert_eq!(
        normalized["transactions"][1]["flows"][0]["to"]["address"],
        owner
    );
    assert_eq!(
        normalized["transactions"][2]["transaction_class"],
        "system_issuance"
    );
    assert_eq!(
        normalized["transactions"][2]["flows"][0]["from"]["account_kind"],
        "system"
    );
}

#[test]
fn history_remote_query_filters_before_pagination() {
    assert_eq!(
        nni_asset_transfer_history_remote_query(
            "owner",
            2,
            100,
            NniAssetHistorySourceFilter::Trade,
            NniAssetHistoryDirectionFilter::Incoming,
        ),
        vec![
            ("address", "owner".to_string()),
            ("page", "2".to_string()),
            ("per_page", "100".to_string()),
            ("transaction_class", "market_trade".to_string()),
            ("direction", "incoming".to_string()),
        ]
    );
    assert_eq!(
        nni_asset_transfer_history_remote_query(
            "owner",
            1,
            100,
            NniAssetHistorySourceFilter::All,
            NniAssetHistoryDirectionFilter::All,
        ),
        vec![
            ("address", "owner".to_string()),
            ("page", "1".to_string()),
            ("per_page", "100".to_string()),
        ]
    );
}

#[test]
fn history_rejects_an_unfiltered_transaction_kind() {
    let owner = generate_nni_owner_key_pair().public_key;
    let payload = json!({
        "schema_version": 1,
        "status": "explorer_transactions",
        "page": 1,
        "per_page": 100,
        "total": 1,
        "total_pages": 1,
        "filter": {
            "transaction_kind": null,
            "transaction_class": "peer_transfer",
            "direction": null,
        },
        "transactions": [{
            "transaction_id": "reward-not-a-transfer",
            "transaction_kind": "heartbeat_reward_credit",
            "transaction_class": "system_issuance",
            "created_at_unix": 1_700_000_000,
            "memo": null,
            "flows": [],
        }],
    });

    assert_eq!(
        normalize_asset_transfer_history_response(
            &payload,
            &owner,
            NniAssetHistorySourceFilter::Transfer,
            NniAssetHistoryDirectionFilter::All,
        ),
        Err("nni_asset_transfer_history_contract_invalid")
    );
}

#[test]
fn history_rejects_a_transfer_that_does_not_include_the_requested_owner() {
    let owner = generate_nni_owner_key_pair().public_key;
    let sender = generate_nni_owner_key_pair().public_key;
    let recipient = generate_nni_owner_key_pair().public_key;
    let payload = json!({
        "schema_version": 1,
        "status": "explorer_transactions",
        "page": 1,
        "per_page": 100,
        "total": 1,
        "total_pages": 1,
        "filter": {
            "transaction_kind": null,
            "transaction_class": null,
            "direction": null,
        },
        "transactions": [{
            "transaction_id": "asset-transfer-unrelated",
            "transaction_kind": "asset_transfer",
            "transaction_class": "peer_transfer",
            "created_at_unix": 1_700_000_000,
            "memo": null,
            "flows": [{
                "flow_index": 0,
                "asset": "AIC",
                "amount_units": "100000000",
                "amount": "1.00000000",
                "from": {"account_kind": "asset_owner", "address": sender},
                "to": {"account_kind": "asset_owner", "address": recipient},
            }],
        }],
    });

    assert_eq!(
        normalize_asset_transfer_history_response(
            &payload,
            &owner,
            NniAssetHistorySourceFilter::All,
            NniAssetHistoryDirectionFilter::All,
        ),
        Err("nni_asset_transfer_history_contract_invalid")
    );
}

#[test]
fn history_direction_projects_only_matching_trade_flows() {
    let owner = generate_nni_owner_key_pair().public_key;
    let pool = generate_nni_owner_key_pair().public_key;
    let fee = generate_nni_owner_key_pair().public_key;
    let payload = json!({
        "schema_version": 1,
        "status": "explorer_transactions",
        "page": 1,
        "per_page": 100,
        "total": 1,
        "total_pages": 1,
        "filter": {
            "transaction_kind": null,
            "transaction_class": "market_trade",
            "direction": "incoming",
        },
        "transactions": [{
            "transaction_id": "trade-buy",
            "transaction_kind": "bancor_buy",
            "transaction_class": "market_trade",
            "created_at_unix": 1_700_000_000,
            "memo": null,
            "flows": [{
                "flow_index": 2,
                "asset": "AIC",
                "amount_units": "100000000",
                "amount": "1.00000000",
                "from": {"account_kind": "pool", "address": pool},
                "to": {"account_kind": "asset_owner", "address": owner},
            }, {
                "flow_index": 1,
                "asset": "USD",
                "amount_units": "1000000",
                "amount": "0.01000000",
                "from": {"account_kind": "asset_owner", "address": owner},
                "to": {"account_kind": "fee", "address": fee},
            }],
        }],
    });

    let normalized = normalize_asset_transfer_history_response(
        &payload,
        &owner,
        NniAssetHistorySourceFilter::Trade,
        NniAssetHistoryDirectionFilter::Incoming,
    )
    .expect("the incoming side of a market trade should remain visible");
    assert_eq!(normalized["transactions"][0]["flows"].as_array().unwrap().len(), 1);
    assert_eq!(normalized["transactions"][0]["flows"][0]["from"]["account_kind"], "pool");
}
