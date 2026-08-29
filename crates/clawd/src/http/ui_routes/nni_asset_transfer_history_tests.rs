use super::*;

#[test]
fn history_projects_both_outgoing_and_incoming_transfers() {
    let owner = generate_nni_owner_key_pair().public_key;
    let sender = generate_nni_owner_key_pair().public_key;
    let recipient = generate_nni_owner_key_pair().public_key;
    let payload = json!({
        "schema_version": 1,
        "status": "explorer_transactions",
        "pagination_mode": "cursor",
        "per_page": 100,
        "total": 3,
        "has_more": false,
        "next_cursor": null,
        "transactions": [
            {
                "transaction_id": "asset-transfer-outgoing",
                "transaction_kind": "asset_transfer",
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
                "transaction_id": "bancor-trade",
                "transaction_kind": "bancor_trade",
                "created_at_unix": 1_700_000_000,
                "memo": null,
                "flows": [],
            },
        ],
    });

    let normalized = normalize_asset_transfer_history_response(&payload, &owner, 10)
        .expect("valid address history should be projected");
    assert_eq!(normalized["status"], "asset_transfer_history");
    assert_eq!(normalized["owner_pubkey"], owner);
    assert_eq!(normalized["transactions"].as_array().unwrap().len(), 2);
    assert_eq!(
        normalized["transactions"][0]["flows"][0]["from"]["address"],
        owner
    );
    assert_eq!(
        normalized["transactions"][1]["flows"][0]["to"]["address"],
        owner
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
        "total": 1,
        "has_more": false,
        "transactions": [{
            "transaction_id": "asset-transfer-unrelated",
            "transaction_kind": "asset_transfer",
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
        normalize_asset_transfer_history_response(&payload, &owner, 10),
        Err("nni_asset_transfer_history_contract_invalid")
    );
}
