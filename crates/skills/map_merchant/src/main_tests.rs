use super::*;

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
        display_address_value(&pois[0].address),
        "上海市黄浦区人民大道1号"
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
