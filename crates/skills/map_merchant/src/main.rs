//! Merchant recommendation skill backed by map providers.

use std::cmp::Ordering;
use std::collections::HashSet;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::thread::sleep;
use std::time::Duration;

use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

mod config;
mod formatting;

use config::{resolve_runtime_config, RuntimeConfig};
use formatting::{round3, round6, utf8_safe_prefix};

const AMAP_GEOCODE_URL: &str = "https://restapi.amap.com/v3/geocode/geo";
const AMAP_AROUND_URL: &str = "https://restapi.amap.com/v3/place/around";
const GOOGLE_GEOCODE_URL: &str = "https://maps.googleapis.com/maps/api/geocode/json";
const GOOGLE_TEXT_SEARCH_URL: &str = "https://places.googleapis.com/v1/places:searchText";
const DEFAULT_PROVIDER: &str = "amap";
const DEFAULT_KEYWORD: &str = "merchant";
const MIN_RADIUS_METERS: u32 = 500;
const MAX_RADIUS_METERS: u32 = 50_000;
const DEFAULT_TIMEOUT_SECONDS: u64 = 20;
const DEFAULT_CONNECT_TIMEOUT_SECONDS: u64 = 8;
const MAX_TOP_K: usize = 10;
const MAX_FETCH_CANDIDATES: usize = 20;
const HTTP_RETRY_ATTEMPTS: usize = 5;
const HTTP_RETRY_BASE_DELAY_MS: u64 = 600;
const HTTP_RETRY_MAX_DELAY_MS: u64 = 5_000;
const SKILL_NAME: &str = "map_merchant";

#[derive(Debug, Deserialize)]
struct Req {
    request_id: String,
    args: Value,
    #[serde(default)]
    context: Option<Value>,
}

#[derive(Debug, Serialize)]
struct Resp {
    request_id: String,
    status: String,
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    extra: Option<Value>,
    error_text: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum MapProvider {
    Amap,
    Google,
}

#[derive(Debug, Clone)]
struct MerchantQuery {
    provider: MapProvider,
    anchor_lat: f64,
    anchor_lon: f64,
    anchor_text: String,
    anchor_source: String,
    city: Option<String>,
    district: Option<String>,
    address: Option<String>,
    keyword: String,
    category: Option<String>,
    cuisine: Option<String>,
    price_pref: PricePreference,
    sort_by: SortBy,
    radius_meters: u32,
    top_k: usize,
    fetch_candidates: usize,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum SortBy {
    Balanced,
    Distance,
    Rating,
    Price,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
enum PricePreference {
    Any,
    Cheap,
    Mid,
    Premium,
}

#[derive(Debug, Serialize)]
struct RankedMerchant {
    provider: MapProvider,
    name: String,
    address: Option<String>,
    distance_meters: Option<u32>,
    rating: Option<f64>,
    average_cost: Option<f64>,
    score: f64,
    reason_codes: Vec<String>,
    category: Option<String>,
    phone: Option<String>,
    location: Option<Value>,
    navigation_links: Option<Value>,
    place_url: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
struct AmapPoi {
    #[serde(default)]
    name: String,
    #[serde(default)]
    address: Value,
    #[serde(default, rename = "type")]
    type_: String,
    #[serde(default, rename = "typecode")]
    type_code: String,
    #[serde(default)]
    distance: String,
    #[serde(default)]
    location: String,
    #[serde(default)]
    tel: Value,
    #[serde(default)]
    biz_ext: Option<AmapBizExt>,
}

#[derive(Debug, Deserialize, Clone, Default)]
struct AmapBizExt {
    #[serde(default)]
    rating: Value,
    #[serde(default)]
    cost: Value,
}

#[derive(Debug, Deserialize)]
struct GoogleGeocodeResponse {
    status: String,
    #[serde(default)]
    error_message: Option<String>,
    #[serde(default)]
    results: Vec<GoogleGeocodeResult>,
}

#[derive(Debug, Deserialize)]
struct GoogleGeocodeResult {
    #[serde(default)]
    formatted_address: String,
    geometry: GoogleGeometry,
}

#[derive(Debug, Deserialize)]
struct GoogleGeometry {
    location: GoogleLatLng,
}

#[derive(Debug, Deserialize, Clone, Copy)]
struct GoogleLatLng {
    lat: f64,
    lng: f64,
}

#[derive(Debug, Deserialize)]
struct GoogleTextSearchResponse {
    #[serde(default)]
    places: Vec<GooglePlace>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct GooglePlace {
    #[serde(default)]
    display_name: Option<GoogleDisplayText>,
    #[serde(default)]
    formatted_address: Option<String>,
    #[serde(default)]
    location: Option<GoogleLatLng>,
    #[serde(default)]
    rating: Option<f64>,
    #[serde(default)]
    price_level: Option<String>,
    #[serde(default)]
    primary_type: Option<String>,
    #[serde(default)]
    primary_type_display_name: Option<GoogleDisplayText>,
    #[serde(default)]
    national_phone_number: Option<String>,
    #[serde(default)]
    google_maps_uri: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
struct GoogleDisplayText {
    #[serde(default)]
    text: String,
}

fn main() -> anyhow::Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let workspace_root = std::env::var("WORKSPACE_ROOT")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    for line in stdin.lock().lines() {
        let line = line?;
        let parsed: Result<Req, _> = serde_json::from_str(&line);
        let resp = match parsed {
            Ok(req) => {
                let cfg = resolve_runtime_config(&workspace_root);
                match execute(&req, &cfg) {
                    Ok((text, extra)) => Resp {
                        request_id: req.request_id,
                        status: "ok".to_string(),
                        text,
                        extra: Some(extra),
                        error_text: None,
                    },
                    Err(err) => Resp {
                        request_id: req.request_id,
                        status: "error".to_string(),
                        text: String::new(),
                        extra: Some(error_extra("execution_failed")),
                        error_text: Some(err),
                    },
                }
            }
            Err(err) => Resp {
                request_id: "unknown".to_string(),
                status: "error".to_string(),
                text: String::new(),
                extra: Some(error_extra("invalid_input")),
                error_text: Some(format!("code=invalid_input detail={err}")),
            },
        };
        writeln!(stdout, "{}", serde_json::to_string(&resp)?)?;
        stdout.flush()?;
    }
    Ok(())
}

fn error_extra(error_kind: &str) -> Value {
    json!({
        "schema_version": 1,
        "source_skill": SKILL_NAME,
        "status": "error",
        "error_kind": error_kind,
        "message_key": format!("skill.{}.{}", SKILL_NAME, error_kind),
        "retryable": false,
    })
}

fn execute(req: &Req, cfg: &RuntimeConfig) -> Result<(String, Value), String> {
    let args = req
        .args
        .as_object()
        .ok_or_else(|| "code=args_not_object".to_string())?;
    let action = args
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or("recommend")
        .trim()
        .to_ascii_lowercase();
    if action != "recommend" {
        return Err(format!(
            "code=unsupported_action action={action} expected=recommend"
        ));
    }

    let ctx_provider = context_string(req.context.as_ref(), &["provider"]);
    let provider_input = args
        .get("provider")
        .and_then(Value::as_str)
        .or(ctx_provider.as_deref());
    let provider = provider_input
        .and_then(|raw| parse_provider(Some(raw)).or_else(|| cfg.provider_for_alias(raw)))
        .unwrap_or(cfg.default_provider);
    ensure_provider_ready(provider, cfg)?;

    let client = Client::builder()
        .connect_timeout(Duration::from_secs(DEFAULT_CONNECT_TIMEOUT_SECONDS))
        .timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECONDS))
        .build()
        .map_err(|e| format!("code=http_client_build_failed detail={e}"))?;

    let query = build_query(args, req.context.as_ref(), cfg, provider, &client)?;
    let merchants = match provider {
        MapProvider::Amap => fetch_amap_merchants(&client, cfg, &query)?,
        MapProvider::Google => fetch_google_merchants(&client, cfg, &query)?,
    };
    if merchants.is_empty() {
        return Err(format!(
            "code=no_matching_merchants anchor_label={} keyword={}",
            query.anchor_text, query.keyword
        ));
    }
    let top_list: Vec<RankedMerchant> = merchants.into_iter().take(query.top_k).collect();
    let text = render_text(&query, &top_list);
    let extra = json!({
        "schema_version": 1,
        "source_skill": SKILL_NAME,
        "status": "ok",
        "message_key": "skill.map_merchant.recommendation_ready",
        "action": "recommend",
        "mode": "merchant_recommendation",
        "provider": query.provider,
        "provider_token": provider_token(query.provider),
        "anchor": {
            "source": query.anchor_source,
            "label": query.anchor_text,
            "latitude": round6(query.anchor_lat),
            "longitude": round6(query.anchor_lon),
        },
        "query": {
            "keyword": query.keyword,
            "category": query.category,
            "cuisine": query.cuisine,
            "price_level": query.price_pref,
            "sort_by": query.sort_by,
            "radius_meters": query.radius_meters,
            "top_k": query.top_k,
            "city": query.city,
            "district": query.district,
            "address": query.address,
        },
        "returned": top_list.len(),
        "candidates": top_list,
    });
    Ok((text, extra))
}

fn build_query(
    args: &Map<String, Value>,
    context: Option<&Value>,
    cfg: &RuntimeConfig,
    provider: MapProvider,
    client: &Client,
) -> Result<MerchantQuery, String> {
    let category = get_trimmed(args, &["category"]);
    let cuisine = get_trimmed(args, &["cuisine"]);
    let keyword = get_trimmed(args, &["keyword"]);
    let city = get_trimmed(args, &["city"]);
    let district = get_trimmed(args, &["district"]);
    let address = get_trimmed(args, &["address"]);
    let place = get_trimmed(args, &["place", "location", "q"]);
    let keyword_text = build_search_keyword(keyword, category.clone(), cuisine.clone(), cfg);
    let price_pref = parse_price_pref(args.get("price_level"));
    let ctx_sort = context_string(context, &["sort_by"]);
    let sort_by = parse_sort_by(
        args.get("sort_by")
            .and_then(Value::as_str)
            .or(ctx_sort.as_deref()),
        &cfg.default_sort_by,
    );
    let radius_meters = args
        .get("max_distance_meters")
        .or_else(|| args.get("radius"))
        .and_then(json_to_u32)
        .unwrap_or(cfg.default_radius_meters)
        .clamp(MIN_RADIUS_METERS, MAX_RADIUS_METERS);
    let top_k = args
        .get("top_k")
        .or_else(|| args.get("topK"))
        .and_then(json_to_usize)
        .unwrap_or(cfg.default_top_k)
        .clamp(1, MAX_TOP_K);
    let fetch_candidates = cfg
        .max_fetch_candidates
        .max(top_k)
        .clamp(1, MAX_FETCH_CANDIDATES);

    if let (Some(lat), Some(lon)) = (
        args.get("latitude").and_then(json_to_f64),
        args.get("longitude").and_then(json_to_f64),
    ) {
        return Ok(MerchantQuery {
            provider,
            anchor_lat: lat,
            anchor_lon: lon,
            anchor_text: format!("lat={:.4} lon={:.4}", lat, lon),
            anchor_source: "coordinates".to_string(),
            city,
            district,
            address: address.or(place),
            keyword: keyword_text,
            category,
            cuisine,
            price_pref,
            sort_by,
            radius_meters,
            top_k,
            fetch_candidates,
        });
    }

    let anchor_query = join_parts(&[
        city.as_deref(),
        district.as_deref(),
        address.as_deref(),
        place.as_deref(),
    ]);
    if anchor_query.is_empty() {
        return Err(
            "code=missing_anchor required_any=latitude_longitude,city,district,address,place"
                .to_string(),
        );
    }
    let (anchor_lon, anchor_lat, anchor_text) = match provider {
        MapProvider::Amap => {
            geocode_amap_anchor(client, &cfg.amap.api_key, &anchor_query, city.as_deref())?
        }
        MapProvider::Google => geocode_google_anchor(client, &cfg.google.api_key, &anchor_query)?,
    };

    Ok(MerchantQuery {
        provider,
        anchor_lat,
        anchor_lon,
        anchor_text,
        anchor_source: "geocode".to_string(),
        city,
        district,
        address: address.or(place),
        keyword: keyword_text,
        category,
        cuisine,
        price_pref,
        sort_by,
        radius_meters,
        top_k,
        fetch_candidates,
    })
}

fn fetch_amap_merchants(
    client: &Client,
    cfg: &RuntimeConfig,
    query: &MerchantQuery,
) -> Result<Vec<RankedMerchant>, String> {
    let params = vec![
        ("key".to_string(), cfg.amap.api_key.clone()),
        (
            "location".to_string(),
            format!("{:.6},{:.6}", query.anchor_lon, query.anchor_lat),
        ),
        ("keywords".to_string(), query.keyword.clone()),
        ("radius".to_string(), query.radius_meters.to_string()),
        ("offset".to_string(), query.fetch_candidates.to_string()),
        ("page".to_string(), "1".to_string()),
        ("extensions".to_string(), "all".to_string()),
        ("sortrule".to_string(), "distance".to_string()),
    ];
    let res = send_with_retry(
        || client.get(AMAP_AROUND_URL).query(&params).send(),
        "amap_nearby_request_failed",
    )?;
    if !res.status().is_success() {
        let status = res.status();
        let preview = res.text().unwrap_or_default();
        return Err(format!(
            "code=amap_nearby_http_status status={} preview={}",
            status,
            utf8_safe_prefix(&preview, 200)
        ));
    }
    let body = parse_json_response(res, "amap_nearby_json_parse_failed")?;
    ensure_amap_success_value(&body)?;
    let pois = amap_pois_from_value(&body);

    let tokens = keyword_tokens(
        &query.keyword,
        query.category.as_deref(),
        query.cuisine.as_deref(),
    );
    let mut ranked = Vec::new();
    for poi in pois {
        if poi.name.trim().is_empty() {
            continue;
        }
        let distance_meters = parse_u32(&poi.distance);
        if let Some(distance) = distance_meters {
            if distance > query.radius_meters {
                continue;
            }
        }
        let rating = poi
            .biz_ext
            .as_ref()
            .and_then(|v| json_value_to_f64(&v.rating))
            .filter(|v| *v > 0.0);
        let average_cost = poi
            .biz_ext
            .as_ref()
            .and_then(|v| json_value_to_f64(&v.cost))
            .filter(|v| *v > 0.0);
        let distance_score = distance_component(distance_meters, query.radius_meters);
        let rating_score = rating_component(rating);
        let price_score = price_component(average_cost, &query.price_pref);
        let keyword_score = amap_keyword_component(&poi, &tokens);
        let total_score = composite_score(
            query.sort_by,
            cfg,
            distance_score,
            rating_score,
            price_score,
            keyword_score,
        );
        let category = display_category(&poi.type_);
        let reason_codes = build_reasons(
            distance_meters,
            rating,
            average_cost,
            &query.price_pref,
            keyword_score,
            &tokens,
            category.as_deref(),
        );
        let name = poi.name.clone();
        let navigation_links = build_amap_navigation_links(&name, &poi.location);
        ranked.push(RankedMerchant {
            provider: MapProvider::Amap,
            name,
            address: normalized_address_value(&poi.address),
            distance_meters,
            rating,
            average_cost,
            score: round3(total_score),
            reason_codes,
            category,
            phone: optional_string_value(&poi.tel),
            location: parse_location_value(&poi.location),
            navigation_links,
            place_url: None,
        });
    }
    sort_ranked_merchants(&mut ranked);
    if ranked.is_empty() {
        return Err("code=amap_no_usable_candidates".to_string());
    }
    Ok(ranked)
}

fn fetch_google_merchants(
    client: &Client,
    cfg: &RuntimeConfig,
    query: &MerchantQuery,
) -> Result<Vec<RankedMerchant>, String> {
    let text_query = if query.anchor_source == "coordinates" {
        query.keyword.clone()
    } else {
        format!("{} near {}", query.keyword, query.anchor_text)
    };
    let res = client
        .post(GOOGLE_TEXT_SEARCH_URL)
        .header("X-Goog-Api-Key", &cfg.google.api_key)
        .header(
            "X-Goog-FieldMask",
            "places.displayName,places.formattedAddress,places.location,places.rating,places.priceLevel,places.primaryType,places.primaryTypeDisplayName,places.nationalPhoneNumber,places.googleMapsUri",
        )
        .json(&json!({
            "textQuery": text_query,
            "maxResultCount": query.fetch_candidates.min(20),
            "languageCode": cfg.google_language_code,
            "rankPreference": if matches!(query.sort_by, SortBy::Distance) { "DISTANCE" } else { "RELEVANCE" },
            "locationBias": {
                "circle": {
                    "center": {
                        "latitude": query.anchor_lat,
                        "longitude": query.anchor_lon
                    },
                    "radius": query.radius_meters as f64
                }
            }
        }))
        .send()
        .map_err(|e| format!("code=google_places_request_failed detail={e}"))?;
    if !res.status().is_success() {
        let status = res.status();
        let preview = res.text().unwrap_or_default();
        return Err(format!(
            "code=google_places_http_status status={} preview={}",
            status,
            utf8_safe_prefix(&preview, 200)
        ));
    }
    let body: GoogleTextSearchResponse = res
        .json()
        .map_err(|e| format!("code=google_places_json_parse_failed detail={e}"))?;

    let tokens = keyword_tokens(
        &query.keyword,
        query.category.as_deref(),
        query.cuisine.as_deref(),
    );
    let mut ranked = Vec::new();
    for place in body.places {
        let Some(location) = place.location else {
            continue;
        };
        let name = place
            .display_name
            .as_ref()
            .map(|v| v.text.trim())
            .filter(|v| !v.is_empty())
            .unwrap_or("");
        if name.is_empty() {
            continue;
        }
        let distance_meters = Some(haversine_meters(
            query.anchor_lat,
            query.anchor_lon,
            location.lat,
            location.lng,
        ));
        if let Some(distance) = distance_meters {
            if distance > query.radius_meters {
                continue;
            }
        }
        let rating = place.rating.filter(|v| *v > 0.0);
        let average_cost = google_price_level_to_cost(place.price_level.as_deref());
        let distance_score = distance_component(distance_meters, query.radius_meters);
        let rating_score = rating_component(rating);
        let price_score = price_component(average_cost, &query.price_pref);
        let keyword_score = google_keyword_component(&place, &tokens);
        let total_score = composite_score(
            query.sort_by,
            cfg,
            distance_score,
            rating_score,
            price_score,
            keyword_score,
        );
        let category = place
            .primary_type_display_name
            .as_ref()
            .map(|v| v.text.trim().to_string())
            .filter(|v| !v.is_empty())
            .or_else(|| place.primary_type.clone());
        let reason_codes = build_reasons(
            distance_meters,
            rating,
            average_cost,
            &query.price_pref,
            keyword_score,
            &tokens,
            category.as_deref(),
        );
        ranked.push(RankedMerchant {
            provider: MapProvider::Google,
            name: name.to_string(),
            address: place
                .formatted_address
                .as_deref()
                .and_then(normalized_address),
            distance_meters,
            rating,
            average_cost,
            score: round3(total_score),
            reason_codes,
            category,
            phone: place
                .national_phone_number
                .as_deref()
                .and_then(optional_string),
            location: Some(json!({
                "longitude": round6(location.lng),
                "latitude": round6(location.lat),
            })),
            navigation_links: Some(build_google_navigation_links(
                name,
                location.lat,
                location.lng,
            )),
            place_url: place.google_maps_uri,
        });
    }
    sort_ranked_merchants(&mut ranked);
    Ok(ranked)
}

fn sort_ranked_merchants(ranked: &mut [RankedMerchant]) {
    ranked.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| cmp_option_u32(a.distance_meters, b.distance_meters))
            .then_with(|| b.rating.partial_cmp(&a.rating).unwrap_or(Ordering::Equal))
    });
}

fn ensure_provider_ready(provider: MapProvider, cfg: &RuntimeConfig) -> Result<(), String> {
    let p = match provider {
        MapProvider::Amap => &cfg.amap,
        MapProvider::Google => &cfg.google,
    };
    if !p.enabled {
        return Err(format!(
            "code=provider_disabled provider={}",
            provider_token(provider)
        ));
    }
    if p.api_key.trim().is_empty() {
        return Err(format!(
            "code=provider_api_key_missing provider={} config=configs/map_merchant.toml",
            provider_token(provider)
        ));
    }
    Ok(())
}

fn geocode_amap_anchor(
    client: &Client,
    api_key: &str,
    address: &str,
    city: Option<&str>,
) -> Result<(f64, f64, String), String> {
    let mut params = vec![
        ("key".to_string(), api_key.to_string()),
        ("address".to_string(), address.to_string()),
    ];
    if let Some(city_name) = city {
        params.push(("city".to_string(), city_name.to_string()));
    }
    let res = send_with_retry(
        || client.get(AMAP_GEOCODE_URL).query(&params).send(),
        "amap_geocode_request_failed",
    )?;
    if !res.status().is_success() {
        let status = res.status();
        let preview = res.text().unwrap_or_default();
        return Err(format!(
            "code=amap_geocode_http_status status={} preview={}",
            status,
            utf8_safe_prefix(&preview, 200)
        ));
    }
    let body = parse_json_response(res, "amap_geocode_json_parse_failed")?;
    ensure_amap_success_value(&body)?;
    let (formatted_address, location_text) = first_amap_geocode(&body)
        .ok_or_else(|| format!("code=amap_geocode_not_found address={address}"))?;
    let (lon, lat) = parse_lon_lat(&location_text)
        .ok_or_else(|| format!("code=amap_geocode_invalid_location raw={location_text}"))?;
    Ok((
        lon,
        lat,
        if formatted_address.trim().is_empty() {
            address.to_string()
        } else {
            formatted_address
        },
    ))
}

fn geocode_google_anchor(
    client: &Client,
    api_key: &str,
    address: &str,
) -> Result<(f64, f64, String), String> {
    let res = client
        .get(GOOGLE_GEOCODE_URL)
        .query(&[("address", address), ("key", api_key)])
        .send()
        .map_err(|e| format!("code=google_geocode_request_failed detail={e}"))?;
    if !res.status().is_success() {
        return Err(format!(
            "code=google_geocode_http_status status={}",
            res.status()
        ));
    }
    let body: GoogleGeocodeResponse = res
        .json()
        .map_err(|e| format!("code=google_geocode_json_parse_failed detail={e}"))?;
    if body.status != "OK" {
        return Err(format!(
            "code=google_geocode_status status={} message={}",
            body.status,
            body.error_message.as_deref().unwrap_or("")
        ));
    }
    let first = body
        .results
        .into_iter()
        .next()
        .ok_or_else(|| format!("code=google_geocode_not_found address={address}"))?;
    Ok((
        first.geometry.location.lng,
        first.geometry.location.lat,
        if first.formatted_address.trim().is_empty() {
            address.to_string()
        } else {
            first.formatted_address
        },
    ))
}

fn ensure_amap_success(
    status: &Option<String>,
    info: &Option<String>,
    infocode: &Option<String>,
) -> Result<(), String> {
    if status.as_deref() == Some("1") {
        return Ok(());
    }
    Err(format!(
        "code=amap_api_status info={} infocode={}",
        info.as_deref().unwrap_or("unknown error"),
        infocode.as_deref().unwrap_or("-")
    ))
}

fn ensure_amap_success_value(body: &Value) -> Result<(), String> {
    let status = body.get("status").and_then(value_to_string_lossy);
    let info = body.get("info").and_then(value_to_string_lossy);
    let infocode = body.get("infocode").and_then(value_to_string_lossy);
    ensure_amap_success(&status, &info, &infocode)
}

fn render_text(query: &MerchantQuery, merchants: &[RankedMerchant]) -> String {
    format!(
        "message_key=skill.map_merchant.recommendation_ready provider={} returned={} anchor_source={} radius_meters={} sort_by={} keyword={}",
        provider_token(query.provider),
        merchants.len(),
        query.anchor_source,
        query.radius_meters,
        sort_token(query.sort_by),
        query.keyword
    )
}

fn build_reasons(
    distance_meters: Option<u32>,
    rating: Option<f64>,
    average_cost: Option<f64>,
    price_pref: &PricePreference,
    keyword_score: f64,
    tokens: &[String],
    category: Option<&str>,
) -> Vec<String> {
    let mut reasons = Vec::new();
    if let Some(distance) = distance_meters {
        if distance <= 800 {
            reasons.push("distance.very_near".to_string());
        } else if distance <= 2000 {
            reasons.push("distance.near".to_string());
        }
    }
    if let Some(value) = rating {
        if value >= 4.5 {
            reasons.push("rating.high".to_string());
        } else if value >= 4.0 {
            reasons.push("rating.stable".to_string());
        }
    }
    if let Some(cost) = average_cost {
        match price_pref {
            PricePreference::Cheap if cost <= 50.0 => reasons.push("price.cheap_match".to_string()),
            PricePreference::Mid if (40.0..=120.0).contains(&cost) => {
                reasons.push("price.mid_match".to_string())
            }
            PricePreference::Premium if cost >= 120.0 => {
                reasons.push("price.premium_match".to_string())
            }
            PricePreference::Any => {}
            _ => {}
        }
    }
    if keyword_score >= 0.85 {
        reasons.push("keyword.strong".to_string());
    } else if keyword_score >= 0.55 && !tokens.is_empty() {
        reasons.push("keyword.partial".to_string());
    }
    if reasons.is_empty() && category.map(str::trim).is_some_and(|v| !v.is_empty()) {
        reasons.push("category.present".to_string());
    }
    reasons
}

fn composite_score(
    sort_by: SortBy,
    cfg: &RuntimeConfig,
    distance_score: f64,
    rating_score: f64,
    price_score: f64,
    keyword_score: f64,
) -> f64 {
    match sort_by {
        SortBy::Distance => {
            0.65 * distance_score + 0.15 * rating_score + 0.05 * price_score + 0.15 * keyword_score
        }
        SortBy::Rating => {
            0.20 * distance_score + 0.60 * rating_score + 0.05 * price_score + 0.15 * keyword_score
        }
        SortBy::Price => {
            0.20 * distance_score + 0.15 * rating_score + 0.50 * price_score + 0.15 * keyword_score
        }
        SortBy::Balanced => {
            cfg.distance_weight * distance_score
                + cfg.rating_weight * rating_score
                + cfg.price_weight * price_score
                + cfg.keyword_weight * keyword_score
        }
    }
}

fn distance_component(distance_meters: Option<u32>, radius_meters: u32) -> f64 {
    match distance_meters {
        Some(distance) => 1.0 - (distance as f64 / radius_meters as f64).min(1.0),
        None => 0.45,
    }
}

fn rating_component(rating: Option<f64>) -> f64 {
    rating.map(|v| (v / 5.0).clamp(0.0, 1.0)).unwrap_or(0.50)
}

fn price_component(cost: Option<f64>, pref: &PricePreference) -> f64 {
    match pref {
        PricePreference::Any => cost
            .map(|v| {
                if v <= 50.0 {
                    0.75
                } else if v <= 120.0 {
                    0.85
                } else {
                    0.70
                }
            })
            .unwrap_or(0.50),
        PricePreference::Cheap => cost
            .map(|v| {
                if v <= 50.0 {
                    1.0
                } else if v <= 90.0 {
                    0.55
                } else {
                    0.20
                }
            })
            .unwrap_or(0.45),
        PricePreference::Mid => cost
            .map(|v| {
                if (40.0..=120.0).contains(&v) {
                    1.0
                } else if v < 40.0 || v <= 150.0 {
                    0.55
                } else {
                    0.25
                }
            })
            .unwrap_or(0.45),
        PricePreference::Premium => cost
            .map(|v| {
                if v >= 120.0 {
                    1.0
                } else if v >= 80.0 {
                    0.60
                } else {
                    0.20
                }
            })
            .unwrap_or(0.45),
    }
}

fn amap_keyword_component(poi: &AmapPoi, tokens: &[String]) -> f64 {
    if tokens.is_empty() {
        return 0.60;
    }
    let haystack = format!(
        "{} {} {} {}",
        poi.name.to_lowercase(),
        poi.type_.to_lowercase(),
        normalized_address_value(&poi.address)
            .unwrap_or_default()
            .to_lowercase(),
        poi.type_code.to_lowercase()
    );
    keyword_match_score(&haystack, tokens)
}

fn google_keyword_component(place: &GooglePlace, tokens: &[String]) -> f64 {
    if tokens.is_empty() {
        return 0.60;
    }
    let haystack = format!(
        "{} {} {} {}",
        place
            .display_name
            .as_ref()
            .map(|v| v.text.to_lowercase())
            .unwrap_or_default(),
        place
            .formatted_address
            .as_deref()
            .unwrap_or("")
            .to_lowercase(),
        place
            .primary_type_display_name
            .as_ref()
            .map(|v| v.text.to_lowercase())
            .unwrap_or_default(),
        place.primary_type.as_deref().unwrap_or("").to_lowercase()
    );
    keyword_match_score(&haystack, tokens)
}

fn keyword_match_score(haystack: &str, tokens: &[String]) -> f64 {
    let mut hit = 0usize;
    for token in tokens {
        if haystack.contains(token) {
            hit += 1;
        }
    }
    (hit as f64 / tokens.len() as f64).clamp(0.0, 1.0)
}

fn keyword_tokens(keyword: &str, category: Option<&str>, cuisine: Option<&str>) -> Vec<String> {
    let mut set = HashSet::new();
    for source in [Some(keyword), category, cuisine] {
        if let Some(value) = source {
            for part in value.split([' ', '/', ',', '，', '、']) {
                let token = part.trim().to_lowercase();
                if token.len() >= 2 {
                    set.insert(token);
                }
            }
        }
    }
    let mut tokens: Vec<String> = set.into_iter().collect();
    tokens.sort();
    tokens
}

fn build_search_keyword(
    keyword: Option<String>,
    category: Option<String>,
    cuisine: Option<String>,
    cfg: &RuntimeConfig,
) -> String {
    let merged = join_parts(&[keyword.as_deref(), cuisine.as_deref(), category.as_deref()]);
    if merged.is_empty() {
        cfg.default_keyword.clone()
    } else {
        merged
    }
}

fn parse_sort_by(raw: Option<&str>, default_value: &str) -> SortBy {
    match raw
        .unwrap_or(default_value)
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "distance" => SortBy::Distance,
        "rating" => SortBy::Rating,
        "price" => SortBy::Price,
        "balanced" => SortBy::Balanced,
        _ => SortBy::Balanced,
    }
}

fn parse_provider(raw: Option<&str>) -> Option<MapProvider> {
    match raw?.trim().to_ascii_lowercase().as_str() {
        "amap" | "gaode" => Some(MapProvider::Amap),
        "google" | "google_maps" | "googlemaps" => Some(MapProvider::Google),
        _ => None,
    }
}

fn provider_token(provider: MapProvider) -> &'static str {
    match provider {
        MapProvider::Amap => "amap",
        MapProvider::Google => "google",
    }
}

fn sort_token(sort_by: SortBy) -> &'static str {
    match sort_by {
        SortBy::Balanced => "balanced",
        SortBy::Distance => "distance",
        SortBy::Rating => "rating",
        SortBy::Price => "price",
    }
}

fn parse_price_pref(value: Option<&Value>) -> PricePreference {
    let Some(value) = value else {
        return PricePreference::Any;
    };
    if let Some(num) = json_to_u32(value) {
        return match num {
            1 => PricePreference::Cheap,
            2 => PricePreference::Mid,
            3 | 4 => PricePreference::Premium,
            _ => PricePreference::Any,
        };
    }
    match value
        .as_str()
        .map(str::trim)
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "cheap" => PricePreference::Cheap,
        "mid" => PricePreference::Mid,
        "premium" => PricePreference::Premium,
        "any" => PricePreference::Any,
        _ => PricePreference::Any,
    }
}

fn get_trimmed(args: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| args.get(*key))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

fn context_string(context: Option<&Value>, keys: &[&str]) -> Option<String> {
    context.and_then(Value::as_object).and_then(|obj| {
        keys.iter().find_map(|key| {
            obj.get(*key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_string)
        })
    })
}

fn join_parts(parts: &[Option<&str>]) -> String {
    parts
        .iter()
        .flatten()
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn parse_lon_lat(raw: &str) -> Option<(f64, f64)> {
    let mut parts = raw.split(',');
    let lon = parts.next()?.trim().parse::<f64>().ok()?;
    let lat = parts.next()?.trim().parse::<f64>().ok()?;
    Some((lon, lat))
}

fn parse_location_value(raw: &str) -> Option<Value> {
    parse_lon_lat(raw).map(|(lon, lat)| {
        json!({
            "longitude": round6(lon),
            "latitude": round6(lat),
        })
    })
}

fn build_amap_navigation_links(name: &str, raw_location: &str) -> Option<Value> {
    let (lon, lat) = parse_lon_lat(raw_location)?;
    let encoded_name = urlencoding::encode(name.trim());
    Some(json!({
        "walk": format!("https://uri.amap.com/navigation?to={lon:.6},{lat:.6},{encoded_name}&mode=walk&policy=0&src=rustclaw&callnative=1"),
        "car": format!("https://uri.amap.com/navigation?to={lon:.6},{lat:.6},{encoded_name}&mode=car&policy=0&src=rustclaw&callnative=1"),
        "ride": format!("https://uri.amap.com/navigation?to={lon:.6},{lat:.6},{encoded_name}&mode=ride&policy=0&src=rustclaw&callnative=1"),
    }))
}

fn build_google_navigation_links(name: &str, lat: f64, lon: f64) -> Value {
    let encoded_name = urlencoding::encode(name.trim());
    json!({
        "walk": format!("https://www.google.com/maps/dir/?api=1&destination={lat:.6},{lon:.6}%20({encoded_name})&travelmode=walking"),
        "car": format!("https://www.google.com/maps/dir/?api=1&destination={lat:.6},{lon:.6}%20({encoded_name})&travelmode=driving"),
        "ride": format!("https://www.google.com/maps/dir/?api=1&destination={lat:.6},{lon:.6}%20({encoded_name})&travelmode=bicycling"),
    })
}

fn send_with_retry<F>(mut op: F, label: &str) -> Result<reqwest::blocking::Response, String>
where
    F: FnMut() -> Result<reqwest::blocking::Response, reqwest::Error>,
{
    let mut last_err = None;
    for attempt in 1..=HTTP_RETRY_ATTEMPTS {
        match op() {
            Ok(res) => return Ok(res),
            Err(err) => {
                last_err = Some(err);
                if attempt < HTTP_RETRY_ATTEMPTS {
                    sleep(Duration::from_millis(retry_backoff_delay_ms(attempt)));
                }
            }
        }
    }
    Err(format!(
        "code={} detail={}",
        label,
        last_err
            .map(|err| err.to_string())
            .unwrap_or_else(|| "unknown request error".to_string())
    ))
}

fn retry_backoff_delay_ms(attempt: usize) -> u64 {
    let exponent = attempt.saturating_sub(1).min(4) as u32;
    let factor = 1u64 << exponent;
    (HTTP_RETRY_BASE_DELAY_MS.saturating_mul(factor)).min(HTTP_RETRY_MAX_DELAY_MS)
}

fn parse_json_response(res: reqwest::blocking::Response, label: &str) -> Result<Value, String> {
    let body = res.text().map_err(|e| format!("code={label} detail={e}"))?;
    serde_json::from_str::<Value>(&body).map_err(|e| {
        format!(
            "code={label} detail={e} preview={}",
            utf8_safe_prefix(&body, 240)
        )
    })
}

fn first_amap_geocode(body: &Value) -> Option<(String, String)> {
    let first = body.get("geocodes")?.as_array()?.first()?.as_object()?;
    let formatted_address = first
        .get("formatted_address")
        .and_then(value_to_string_lossy)
        .unwrap_or_default();
    let location = first.get("location").and_then(value_to_string_lossy)?;
    Some((formatted_address, location))
}

fn amap_pois_from_value(body: &Value) -> Vec<AmapPoi> {
    body.get("pois")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(amap_poi_from_value)
        .collect()
}

fn amap_poi_from_value(value: &Value) -> Option<AmapPoi> {
    let obj = value.as_object()?;
    let name = obj
        .get("name")
        .and_then(value_to_string_lossy)
        .unwrap_or_default();
    let location = obj
        .get("location")
        .and_then(value_to_string_lossy)
        .unwrap_or_default();
    let distance = obj
        .get("distance")
        .and_then(value_to_string_lossy)
        .unwrap_or_default();
    let type_ = obj
        .get("type")
        .and_then(value_to_string_lossy)
        .unwrap_or_default();
    let type_code = obj
        .get("typecode")
        .and_then(value_to_string_lossy)
        .unwrap_or_default();
    let address = obj.get("address").cloned().unwrap_or(Value::Null);
    let tel = obj.get("tel").cloned().unwrap_or(Value::Null);
    let biz_ext = obj.get("biz_ext").and_then(|biz| {
        let biz_obj = biz.as_object()?;
        Some(AmapBizExt {
            rating: biz_obj.get("rating").cloned().unwrap_or(Value::Null),
            cost: biz_obj.get("cost").cloned().unwrap_or(Value::Null),
        })
    });

    Some(AmapPoi {
        name,
        address,
        type_,
        type_code,
        distance,
        location,
        tel,
        biz_ext,
    })
}

fn google_price_level_to_cost(level: Option<&str>) -> Option<f64> {
    match level.unwrap_or("").trim() {
        "PRICE_LEVEL_FREE" => Some(0.0),
        "PRICE_LEVEL_INEXPENSIVE" => Some(35.0),
        "PRICE_LEVEL_MODERATE" => Some(80.0),
        "PRICE_LEVEL_EXPENSIVE" => Some(160.0),
        "PRICE_LEVEL_VERY_EXPENSIVE" => Some(260.0),
        _ => None,
    }
}

fn haversine_meters(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> u32 {
    let r = 6_371_000.0_f64;
    let lat1r = lat1.to_radians();
    let lat2r = lat2.to_radians();
    let dlat = (lat2 - lat1).to_radians();
    let dlon = (lon2 - lon1).to_radians();
    let a = (dlat / 2.0).sin().powi(2) + lat1r.cos() * lat2r.cos() * (dlon / 2.0).sin().powi(2);
    let c = 2.0 * a.sqrt().asin();
    (r * c).round() as u32
}

fn parse_u32(raw: &str) -> Option<u32> {
    raw.trim().parse::<u32>().ok()
}

fn parse_f64(raw: &str) -> Option<f64> {
    raw.trim().parse::<f64>().ok()
}

fn json_value_to_f64(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_i64().map(|v| v as f64))
        .or_else(|| value.as_u64().map(|v| v as f64))
        .or_else(|| {
            value
                .as_array()
                .and_then(|items| items.iter().find_map(json_value_to_f64))
        })
        .or_else(|| value.as_str().and_then(parse_f64))
}

fn value_to_string_lossy(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(s) => optional_string(s),
        Value::Number(num) => Some(num.to_string()),
        Value::Bool(v) => Some(v.to_string()),
        Value::Array(items) => items
            .iter()
            .find_map(value_to_string_lossy)
            .and_then(|v| optional_string(&v)),
        Value::Object(_) => None,
    }
}

fn json_to_u32(value: &Value) -> Option<u32> {
    if let Some(v) = value.as_u64() {
        return u32::try_from(v).ok();
    }
    if let Some(v) = value.as_i64() {
        if v >= 0 {
            return u32::try_from(v).ok();
        }
        return None;
    }
    value.as_str().and_then(|v| v.trim().parse::<u32>().ok())
}

fn json_to_usize(value: &Value) -> Option<usize> {
    if let Some(v) = value.as_u64() {
        return usize::try_from(v).ok();
    }
    if let Some(v) = value.as_i64() {
        if v >= 0 {
            return usize::try_from(v).ok();
        }
        return None;
    }
    value.as_str().and_then(|v| v.trim().parse::<usize>().ok())
}

fn json_to_f64(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_i64().map(|v| v as f64))
        .or_else(|| value.as_u64().map(|v| v as f64))
        .or_else(|| value.as_str().and_then(|v| v.trim().parse::<f64>().ok()))
}

fn normalized_address(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn normalized_address_value(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => normalized_address(s),
        Value::Array(items) => items
            .iter()
            .find_map(|item| item.as_str())
            .and_then(normalized_address),
        _ => None,
    }
}

fn display_category(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(
            trimmed
                .split(';')
                .next()
                .unwrap_or(trimmed)
                .trim()
                .to_string(),
        )
    }
}

fn optional_string(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn optional_string_value(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => optional_string(s),
        Value::Array(items) => items
            .iter()
            .find_map(|item| item.as_str())
            .and_then(optional_string),
        _ => None,
    }
}

fn cmp_option_u32(left: Option<u32>, right: Option<u32>) -> Ordering {
    match (left, right) {
        (Some(a), Some(b)) => a.cmp(&b),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

fn clamp01(value: f64) -> f64 {
    if !value.is_finite() {
        0.0
    } else {
        value.clamp(0.0, 1.0)
    }
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
