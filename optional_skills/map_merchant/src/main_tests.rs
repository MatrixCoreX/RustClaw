use super::*;
use std::path::Path;

#[test]
fn preview_recommendation_uses_no_external_provider() {
    let cfg = config::resolve_runtime_config(Path::new("/nonexistent-agent-runtime-test-root"));
    let req = Req {
        request_id: "preview-map".to_string(),
        args: json!({
            "action": "preview_recommend",
            "provider": "amap",
            "latitude": 31.2304,
            "longitude": 121.4737,
            "keyword": "coffee",
            "top_k": 3
        }),
        context: None,
    };

    let (_, extra) = execute(&req, &cfg).unwrap();

    assert_eq!(extra["action"], "preview_recommend");
    assert_eq!(extra["provider_id"], "amap");
    assert_eq!(extra["anchor"]["source"], "coordinates");
    assert_eq!(extra["query"]["top_k"], 3);
    assert_eq!(extra["would_execute"], false);
    assert_eq!(extra["external_call_count"], 0);
}

#[test]
fn preview_recommendation_rejects_partial_coordinates() {
    let cfg = config::resolve_runtime_config(Path::new("/nonexistent-agent-runtime-test-root"));
    let req = Req {
        request_id: "preview-map-invalid".to_string(),
        args: json!({
            "action": "preview_recommend",
            "latitude": 31.2304,
            "keyword": "coffee"
        }),
        context: None,
    };

    assert_eq!(
        execute(&req, &cfg).unwrap_err(),
        "code=incomplete_coordinates required=latitude+longitude"
    );
}

#[test]
fn error_extra_exposes_machine_contract() {
    let extra = error_extra(
        "code=missing_anchor required_any=latitude_longitude,city,district,address,place",
    );

    assert_eq!(extra["schema_version"], 1);
    assert_eq!(extra["source_skill"], SKILL_NAME);
    assert_eq!(extra["status"], "error");
    assert_eq!(extra["error_code"], "missing_anchor");
    assert_eq!(extra["error_code"], "missing_anchor");
    assert_eq!(extra["message_key"], "skill.map_merchant.missing_anchor");
    assert_eq!(extra["retryable"], false);
    assert_eq!(extra["side_effect_applied"], false);
    assert_eq!(extra["failure_phase"], "pre_dispatch");
    assert_eq!(extra["recovery_action"], "request_missing_argument");
    assert_eq!(extra["required_any"][0], json!(["latitude", "longitude"]));
}

#[test]
fn search_keyword_deduplicates_identical_machine_arguments() {
    let cfg = config::resolve_runtime_config(Path::new("/nonexistent-agent-runtime-test-root"));

    assert_eq!(
        build_search_keyword(
            Some("coffee".to_string()),
            Some("coffee".to_string()),
            None,
            &cfg,
        ),
        "coffee"
    );
}

#[test]
fn amap_poi_parser_tolerates_array_fields() {
    let body = json!({
        "status": "1",
        "pois": [
            {
                "name": "测试店",
                "address": ["上海市黄浦区人民大道1号"],
                "type": "餐饮服务;中餐厅;川菜馆",
                "typecode": "050117",
                "distance": 321,
                "location": "121.473700,31.230400",
                "tel": ["021-12345678"],
                "biz_ext": {
                    "rating": "4.7",
                    "cost": ["88"]
                }
            }
        ]
    });

    let pois = amap_pois_from_value(&body);
    assert_eq!(pois.len(), 1);
    assert_eq!(pois[0].name, "测试店");
    assert_eq!(
        normalized_address_value(&pois[0].address).as_deref(),
        Some("上海市黄浦区人民大道1号")
    );
    assert_eq!(
        optional_string_value(&pois[0].tel).as_deref(),
        Some("021-12345678")
    );
    assert_eq!(
        pois[0]
            .biz_ext
            .as_ref()
            .and_then(|biz| json_value_to_f64(&biz.cost)),
        Some(88.0)
    );
}

#[test]
fn amap_geocode_parser_tolerates_numeric_like_shapes() {
    let body = json!({
        "status": "1",
        "geocodes": [
            {
                "formatted_address": "上海市黄浦区人民广场",
                "location": "121.475233,31.228818"
            }
        ]
    });

    let (label, location) = first_amap_geocode(&body).expect("geocode");
    assert_eq!(label, "上海市黄浦区人民广场");
    assert_eq!(location, "121.475233,31.228818");
}

#[test]
fn retry_backoff_delay_grows_and_caps() {
    assert_eq!(retry_backoff_delay_ms(1), 600);
    assert_eq!(retry_backoff_delay_ms(2), 1_200);
    assert_eq!(retry_backoff_delay_ms(3), 2_400);
    assert_eq!(retry_backoff_delay_ms(4), 4_800);
    assert_eq!(retry_backoff_delay_ms(5), 5_000);
    assert_eq!(retry_backoff_delay_ms(6), 5_000);
}
