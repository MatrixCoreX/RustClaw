use super::login_csrf_token;
use serde_json::{json, Value};

#[test]
fn login_csrf_matches_the_server_and_browser_contract() {
    let cases: Vec<Value> =
        serde_json::from_str(include_str!("../tests/fixtures/webd-csrf.json")).unwrap();
    for (index, case) in cases.iter().enumerate() {
        let response = json!({"ok":true,"data":{"csrf_token":case["token"]}});
        let result = login_csrf_token(&response);
        assert_eq!(
            result.is_ok(),
            case["valid"].as_bool().unwrap(),
            "case {index}"
        );
        if let Ok(token) = result {
            assert_eq!(Some(token), case["token"].as_str());
        }
    }
    assert_eq!(
        login_csrf_token(&json!({"ok":true,"data":{}})).unwrap_err(),
        "csrf_missing"
    );
}
