// The authenticated host selects editions using the declaring skill's execution evidence.
// Only explicitly projected results and public source fields cross the data contract.
fn skill_edition_page(
    db: &rusqlite::Connection,
    actor: &claw_core::types::AuthIdentity,
    skill: &str,
    action_refs: &[String],
    query: &AippMediaQuery,
) -> anyhow::Result<Value> {
    let limit = query.limit.unwrap_or(12).clamp(1, 20);
    let cursor = query.cursor_sequence.map(i64::try_from).transpose()?;
    let mut statement = db.prepare(include_str!("aipp_editions.sql"))?;
    let rows = statement.query_map(rusqlite::named_params! {
        ":refs": serde_json::to_string(action_refs)?,
        ":skill": skill, ":principal": &actor.principal_id, ":admin": actor.role.eq_ignore_ascii_case("admin"),
        ":trigger": &query.trigger, ":cursor": cursor, ":limit": (limit+1) as i64
    }, |r| {
        let sequence: i64 = r.get(0)?;
        let text: Option<String> = r.get(8)?;
        let evidence: Option<String> = r.get(9)?;
        let sources = edition_sources(evidence.as_deref(), skill, action_refs);
        Ok((sequence,json!({
            "task_id":r.get::<_,String>(1)?, "channel":r.get::<_,String>(2)?,
            "status":r.get::<_,String>(3)?, "created_at":r.get::<_,String>(4)?,
            "trigger":r.get::<_,String>(5)?, "delivery_status":r.get::<_,Option<String>>(6)?,
            "error_code":r.get::<_,Option<String>>(7)?,
            "text":text.as_deref().map(crate::visible_text::sanitize_user_visible_text),
            "sources":sources, "request_title":r.get::<_,Option<String>>(10)?.as_deref().map(crate::visible_text::sanitize_user_visible_text),
        })))
    })?;
    let mut items = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    let more = items.len() > limit;
    items.truncate(limit);
    let next = items.last().filter(|_| more).map(|(id, _)| *id);
    Ok(json!({"items":items.into_iter().map(|(_,v)|v).collect::<Vec<_>>(),"next_cursor":next}))
}

fn edition_sources(raw: Option<&str>, skill: &str, action_refs: &[String]) -> Vec<Value> {
    let results: Vec<Value> = raw
        .and_then(|v| serde_json::from_str(v).ok())
        .unwrap_or_default();
    let mut seen = BTreeSet::new();
    let mut sources = Vec::new();
    for result in results {
        let cap = result["capability"].as_str().unwrap_or_default();
        let action = result["action"].as_str().unwrap_or_default();
        let extra = &result["data"]["extra"];
        let binding = extra
            .pointer("/execution_binding/skill_name")
            .and_then(Value::as_str);
        if !(binding == Some(skill) || (binding.is_none() && cap == skill))
            || !(action_refs.iter().any(|r| r == cap)
                || (cap == skill && action_refs.contains(&format!("{skill}.{action}"))))
        {
            continue;
        }
        let Some(items) = extra["items"].as_array() else {
            continue;
        };
        for item in items {
            let Some(link) = item["link"].as_str().filter(|s| s.len() <= 4096) else {
                continue;
            };
            let Ok(url) = reqwest::Url::parse(link) else {
                continue;
            };
            if !matches!(url.scheme(), "http" | "https")
                || !url.has_host()
                || !url.username().is_empty()
                || url.password().is_some()
                || !seen.insert(url.to_string())
            {
                continue;
            }
            sources.push(json!({
                "url": url.as_str(),
                "title": bounded_aipp_optional_text(item.get("title"), 512),
                "source": bounded_aipp_optional_text(item.get("source"), 256),
                "date": bounded_aipp_optional_text(item.get("date"), 80),
                "excerpt": bounded_aipp_optional_text(item.get("excerpt"), 1200)
            }));
            if sources.len() == 64 {
                return sources;
            }
        }
    }
    sources
}

