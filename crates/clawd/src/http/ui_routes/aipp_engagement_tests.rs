use super::*;

#[test]
fn metrics_preserve_independent_counts_and_omit_unsafe_fields() {
    let record = json!({
        "platform": "example",
        "engagement": { "schema_version": 1, "platform": "example", "metrics": {
            "views": { "display": "104万" },
            "reads": { "display": "1178453", "value": 1178453, "source": "page_response:reads_count" },
            "shares": { "display": "11", "value": 11 },
            "reposts": { "display": "0", "value": 0 },
            "coins": { "display": "22", "value": 22, "secret": "must-not-leak" },
            "danmaku": { "display": "33", "value": 33 },
            "unknown": { "display": "must-not-leak" },
            "comments": { "display": "invalid\nvalue", "value": 3 },
            "likes": { "display": "1万", "value": 9007199254740992_u64, "source": "/private/path" },
        }}
    });
    let projected = aipp_engagement(&record).expect("engagement");
    let metrics = &projected["metrics"];
    assert_eq!(metrics["reads"]["value"], 1178453);
    assert_eq!(metrics["views"]["display"], "104万");
    assert!(metrics["views"]["value"].is_null());
    assert_eq!(metrics["reposts"]["value"], 0);
    assert_eq!(metrics["shares"]["value"], 11);
    assert_eq!(metrics["coins"]["value"], 22);
    assert_eq!(metrics["danmaku"]["value"], 33);
    assert_eq!(metrics["reads"]["source"], "page_response:reads_count");
    assert!(metrics["likes"]["value"].is_null());
    assert!(metrics["likes"]["source"].is_null());
    assert!(metrics.get("comments").is_none());
    assert!(metrics.get("unknown").is_none());
    assert!(metrics["coins"].get("secret").is_none());
}

#[test]
fn mismatched_platform_never_projects_metrics() {
    assert!(
        aipp_engagement(&json!({ "platform": "first", "engagement": {
            "schema_version": 1, "platform": "other", "metrics": {"likes":{"display":"50"}}
        }}))
        .is_none()
    );
}
