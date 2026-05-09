//! invest_copy：单行 JSON stdin → 单行 JSON stdout。
//! 默认通过 **与 clawd 一致的 OpenAI 兼容端点**（`OPENAI_*` 环境变量，由 skill-runner 注入）
//! 调用主程序当前 `openai_compat` 模型生成投教向文稿；可选 `use_heuristic=true` 走离线规则摘要。

mod llm_client;

use anyhow::Context;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::sync::OnceLock;

const MAX_DATA_CHARS: usize = 12_000;
const MIN_DATA_CHARS: usize = 10;
const SHORT_SUMMARY_MAX: usize = 4;
const ARTICLE_SUMMARY_MAX: usize = 7;

static PERSONAS_TOML: &str = include_str!("../personas.toml");

#[derive(Debug, Deserialize, Clone)]
struct PersonaToml {
    slug: String,
    aliases: Vec<String>,
    #[serde(default)]
    display_name_zh: String,
    #[serde(default)]
    display_name_en: String,
    #[serde(default)]
    one_liner_zh: String,
    #[serde(default)]
    facets_zh: Vec<String>,
    #[serde(default)]
    prefer_zh: Vec<String>,
    #[serde(default)]
    avoid_zh: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct PersonaRegistry {
    personas: Vec<PersonaToml>,
}

#[derive(Debug, Deserialize)]
struct SkillReqLine {
    request_id: String,
    #[serde(default)]
    args: Value,
}

#[derive(Debug, Serialize)]
struct SkillResp {
    request_id: String,
    status: &'static str,
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    extra: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error_text: Option<String>,
}

fn sentence_split_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"[。\.\!?;；\n\r\t]+").expect("regex"))
}

fn main() -> anyhow::Result<()> {
    let registry: PersonaRegistry =
        toml::from_str(PERSONAS_TOML).context("parse embedded personas.toml")?;
    let lookup = build_persona_lookup(&registry.personas);
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    for line in stdin.lock().lines() {
        let line = line?;
        let trimmed = line.trim_end_matches(['\n', '\r']);
        if trimmed.is_empty() {
            continue;
        }
        let resp = match serde_json::from_str::<SkillReqLine>(trimmed) {
            Ok(r) => handle_request(r, &lookup, &registry.personas),
            Err(e) => SkillResp {
                request_id: "unknown".to_string(),
                status: "error",
                text: String::new(),
                extra: None,
                error_text: Some(format!("invalid request JSON: {e}")),
            },
        };
        writeln!(stdout, "{}", serde_json::to_string(&resp)?)?;
        stdout.flush()?;
    }
    Ok(())
}

fn build_persona_lookup(personas: &[PersonaToml]) -> HashMap<String, usize> {
    let mut m = HashMap::new();
    for (idx, p) in personas.iter().enumerate() {
        m.entry(norm_key(&p.slug)).or_insert(idx);
        for a in &p.aliases {
            m.entry(norm_key(a)).or_insert(idx);
        }
    }
    m
}

fn norm_key(s: &str) -> String {
    s.trim().chars().flat_map(|c| c.to_lowercase()).collect()
}

fn handle_request(
    req: SkillReqLine,
    lookup: &HashMap<String, usize>,
    personas: &[PersonaToml],
) -> SkillResp {
    let rid = req.request_id.clone();
    let args = &req.args;
    let action = args
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or("draft")
        .trim()
        .to_ascii_lowercase();
    match action.as_str() {
        "list_investors" => list_investors(rid, personas, args),
        "draft" => draft(rid, args, lookup, personas),
        _ => SkillResp {
            request_id: rid,
            status: "error",
            text: String::new(),
            extra: None,
            error_text: Some(format!(
                "不支持 action `{}`，请使用 draft 或 list_investors",
                action
            )),
        },
    }
}

fn is_en(locale: Option<&str>) -> bool {
    locale
        .map(|s| s.trim().to_ascii_lowercase())
        .map(|s| s.starts_with("en"))
        .unwrap_or(false)
}

fn list_investors(request_id: String, personas: &[PersonaToml], args: &Value) -> SkillResp {
    let locale = args
        .get("locale")
        .or_else(|| args.get("language"))
        .or_else(|| args.get("lang"))
        .and_then(Value::as_str);
    let en = is_en(locale);
    let mut lines = vec![if en {
        "Built-in personas (education-only; not endorsement):".to_string()
    } else {
        "内置人物（仅供投教文风参考，不构成背书）：".to_string()
    }];
    for p in personas {
        let name = if en && !p.display_name_en.is_empty() {
            p.display_name_en.as_str()
        } else {
            p.display_name_zh.as_str()
        };
        lines.push(if en {
            format!("- **{}**: slug=`{}`; {}", name, p.slug, p.one_liner_zh)
        } else {
            format!(
                "- **{}**（`{}`）：{}",
                p.display_name_zh, p.slug, p.one_liner_zh
            )
        });
    }
    let text = lines.join("\n");
    SkillResp {
        request_id,
        status: "ok",
        text,
        extra: Some(json!({ "action": "list_investors", "count": personas.len() })),
        error_text: None,
    }
}

fn extract_text_field<'a>(args: &'a Value, keys: &[&str]) -> Option<&'a str> {
    for k in keys {
        if let Some(Value::String(s)) = args.get(*k) {
            let t = s.trim();
            if !t.is_empty() {
                return Some(t);
            }
        }
    }
    None
}

fn draft(
    request_id: String,
    args: &Value,
    lookup: &HashMap<String, usize>,
    personas: &[PersonaToml],
) -> SkillResp {
    let locale = args
        .get("locale")
        .or_else(|| args.get("language"))
        .or_else(|| args.get("lang"))
        .and_then(Value::as_str);
    let en = is_en(locale);
    let data_raw = extract_text_field(args, &["data", "material", "user_data"]);
    let brief = extract_text_field(args, &["brief", "focus"]).unwrap_or("");
    let source_note = extract_text_field(args, &["source_note", "data_source"]);

    let person_raw = args
        .get("person")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("")
        .to_string();

    if person_raw.is_empty() {
        return SkillResp {
            request_id,
            status: "error",
            text: String::new(),
            extra: None,
            error_text: Some("缺少必选参数 args.person（人物 slug 或别名）".to_string()),
        };
    }

    let pdata = match data_raw {
        Some(s) if s.chars().count() >= MIN_DATA_CHARS => s,
        Some(s) => {
            return SkillResp {
                request_id,
                status: "error",
                text: String::new(),
                extra: None,
                error_text: Some(format!(
                    "args.data/material 有效长度过短（当前 {} 字，至少需要 {} 字）",
                    s.chars().count(),
                    MIN_DATA_CHARS
                )),
            };
        }
        None => {
            return SkillResp {
                request_id,
                status: "error",
                text: String::new(),
                extra: None,
                error_text: Some(
                    "缺少必选参数 args.data（或通过 material/user_data 传入同一正文）".to_string(),
                ),
            };
        }
    };

    if forbidden_peddlers(pdata, brief) {
        return SkillResp {
            request_id,
            status: "error",
            text: String::new(),
            extra: None,
            error_text: Some(
                "材料或侧重点表述包含易被误解为喊单/保本保收益的内容；请改写为学习与信息梳理语境后再试"
                    .to_string(),
            ),
        };
    }

    let nk = norm_key(&person_raw);
    let persona_idx = match lookup.get(&nk) {
        Some(i) => *i,
        None => {
            return SkillResp {
                request_id,
                status: "error",
                text: String::new(),
                extra: None,
                error_text: Some(format!(
                    "未知人物 `{}`，请先使用 action=list_investors 查看可用 slug/别名",
                    person_raw
                )),
            };
        }
    };

    let p = personas[persona_idx].clone();
    let channel = args
        .get("channel")
        .and_then(Value::as_str)
        .unwrap_or("article")
        .trim()
        .to_ascii_lowercase();
    let max_bullets = if channel == "short" {
        SHORT_SUMMARY_MAX
    } else {
        ARTICLE_SUMMARY_MAX
    };

    let compliance = args
        .get("compliance")
        .and_then(Value::as_str)
        .unwrap_or("standard")
        .trim()
        .to_ascii_lowercase();
    let compliance = if compliance == "light" {
        "light"
    } else {
        "standard"
    };

    let use_heuristic = args
        .get("use_heuristic")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let (owned_body, truncated) = truncate_owned(pdata);
    let body_for_summary = owned_body.as_str();

    if use_heuristic {
        let bullets = summarize_bullets(body_for_summary, max_bullets);
        let text = assemble_report(en, &p, &bullets, brief, source_note, truncated, compliance);
        let word_count = text.chars().count();
        return SkillResp {
            request_id,
            status: "ok",
            text,
            extra: Some(json!({
                "action": "draft",
                "person_slug": p.slug,
                "summary_mode": "heuristic",
                "data_truncated": truncated,
                "summary_bullet_count": bullets.len(),
                "compliance": compliance,
                "word_count": word_count
            })),
            error_text: None,
        };
    }

    let system = build_llm_system_prompt(en, compliance);
    let user = build_llm_user_prompt(
        en,
        &p,
        body_for_summary,
        brief,
        source_note,
        truncated,
        &channel,
        max_bullets,
    );

    let generated = match llm_client::chat_completion_default(&system, &user) {
        Ok(t) => t,
        Err(e) => {
            return SkillResp {
                request_id,
                status: "error",
                text: String::new(),
                extra: None,
                error_text: Some(format!("调用默认 LLM 失败：{e}")),
            };
        }
    };

    if forbidden_peddlers(&generated.text, "") {
        return SkillResp {
            request_id,
            status: "error",
            text: String::new(),
            extra: None,
            error_text: Some(
                "模型生成内容触发了合规敏感词校验；请调整材料措辞后重试，或使用 use_heuristic=true"
                    .to_string(),
            ),
        };
    }

    let header_zh = "【以下内容使用系统当前配置的 OpenAI 兼容大模型生成，仅供信息与投教阅读；不构成投资建议，亦不暗示公众人物背书】\n\n";
    let header_en = "_Generated using the system's default OpenAI-compatible model. Educational context only; not investment advice; no endorsement implied._\n\n";
    let text = format!(
        "{}{}",
        if en { header_en } else { header_zh },
        generated.text.trim()
    );
    let word_count = text.chars().count();

    SkillResp {
        request_id,
        status: "ok",
        text,
        extra: Some(json!({
            "action": "draft",
            "person_slug": p.slug,
            "summary_mode": "llm",
            "llm": {
                "credential_source": generated.source,
                "model": generated.model,
            },
            "data_truncated": truncated,
            "compliance": compliance,
            "word_count": word_count
        })),
        error_text: None,
    }
}

fn build_llm_system_prompt(en: bool, compliance: &str) -> String {
    if en {
        return format!(
            "You help produce investor-education copy (NOT personalized advice).\n\
Rules:\n\
1) Faithfully summarize only what appears in USER_MATERIAL; do not invent facts, symbols, codes, prices, or performance guarantees.\n\
2) Style section borrows publicly known *thinking lenses* tied to the named persona — do not impersonate them or imply endorsement.\n\
3) No buy/sell/hold directions, no ticker picks, no return promises.\n\
4) Output Markdown with headings exactly: ### Data highlights, ### Style-inspired commentary, ### Limitations, ### Disclaimer\n\
5) compliance={compliance}: if `standard`, mention third-party excerpt risks (terms/copyright) briefly in Disclaimer."
        );
    }
    format!(
        "你是投教文稿助手（不是投顾）。必须遵守：\n\
1) 仅根据用户材料做摘要与讨论，不得编造材料中不存在的数字、标的或结论。\n\
2) 「风格化解读」只借用该人物公开思想中常见的**观察角度**，不得声称本人撰写或背书。\n\
3) 禁止给出买卖/加减仓/具体证券代码建议，不得承诺或暗示收益。\n\
4) 使用 Markdown，且必须依次包含小节标题（与下列一致）：### 数据摘要、### 风格化解读（灵感来源）、### 局限与未覆盖、### 风险提示与免责声明\n\
5) 当前免责强度 compliance={compliance}：`standard` 时免责段需提示第三方摘录的版权与网站条款风险；`light` 时可较短。"
    )
}

fn build_llm_user_prompt(
    en: bool,
    p: &PersonaToml,
    data: &str,
    brief: &str,
    source_note: Option<&str>,
    truncated: bool,
    channel: &str,
    _max_bullets_hint: usize,
) -> String {
    let facets = p.facets_zh.join(" / ");
    if en {
        let disp_en = if p.display_name_en.trim().is_empty() {
            p.display_name_zh.as_str()
        } else {
            p.display_name_en.as_str()
        };
        return format!(
            "Persona display: {}\nSlug: {}\nOne-liner: {}\nFacets:\n{}\nPreferred wording hints: {}\nChannel preference: {}\nTruncated_input: {}\nFocus (optional): {}\nSource note (optional): {}\n\nUSER_MATERIAL:\n{}",
            disp_en,
            p.slug,
            p.one_liner_zh,
            facets,
            p.prefer_zh.join(", "),
            channel,
            truncated,
            brief,
            source_note.unwrap_or("(none)"),
            data
        );
    }
    format!(
        "人物展示名：{}\nSlug：`{}`\n定位：{}\n观察维度（参考）：\n{}\n措辞偏好：{}\n篇幅偏好 channel：{}\n材料是否被截断：{}\n用户侧重（可空）：{}\n来源备注（可空）：{}\n\n—— 用户材料 ——\n{}",
        p.display_name_zh,
        p.slug,
        p.one_liner_zh,
        facets,
        p.prefer_zh.join("、"),
        channel,
        truncated,
        brief,
        source_note.unwrap_or("无"),
        data
    )
}

fn truncate_owned(s: &str) -> (String, bool) {
    let count = s.chars().count();
    if count <= MAX_DATA_CHARS {
        return (s.to_string(), false);
    }
    (s.chars().take(MAX_DATA_CHARS).collect::<String>(), true)
}

fn forbidden_peddlers(data: &str, brief: &str) -> bool {
    let pack = format!("{}{}", data, brief);
    let zh = [
        "保证收益",
        "保本保收益",
        "稳赚",
        "稳赚不赔",
        "一夜暴富",
        "必买",
        "闭眼买",
        "满仓干",
        "加杠杆满仓",
        "坐庄",
        "内幕消息",
        "无风险高收益",
    ];
    if zh.iter().any(|p| pack.contains(p)) {
        return true;
    }
    let low = pack.to_ascii_lowercase();
    ["guaranteed return", "risk-free return", "must buy ticker"]
        .iter()
        .any(|p| low.contains(p))
}

fn split_sentences(s: &str) -> Vec<String> {
    sentence_split_regex()
        .split(s)
        .map(str::trim)
        .filter(|x| !x.is_empty())
        .filter(|x| x.chars().count() > 10)
        .map(ToString::to_string)
        .collect()
}

fn score_sentence(s: &str) -> i32 {
    let mut score = 0_i32;
    if s.contains('%')
        || s.contains('％')
        || s.contains("pct")
        || s.chars().any(|c| c.is_ascii_digit())
    {
        score += 3;
    }
    if s.contains('？') || s.contains('?') {
        score += 1;
    }
    if s.contains('元') || s.contains("万元") || s.contains('亿') {
        score += 2;
    }
    score
}

fn summarize_bullets(text: &str, max_items: usize) -> Vec<String> {
    let mut candidates: Vec<String> = Vec::new();
    for paragraph in text.split('\n').map(str::trim).filter(|p| !p.is_empty()) {
        if paragraph.len() <= 15 {
            continue;
        }
        if let Some(fs) = paragraph
            .split(|c| c == '。' || c == '.' || c == '；' || c == ';')
            .next()
            .map(str::trim)
            .filter(|x| x.chars().count() > 10)
        {
            candidates.push(fs.to_string());
        }
    }
    for s in split_sentences(text) {
        candidates.push(s);
    }

    let mut seen = std::collections::HashSet::<String>::new();
    let mut uniq: Vec<String> = Vec::new();
    for c in candidates {
        let k = norm_space(&c);
        if seen.insert(k.clone()) {
            uniq.push(c);
        }
    }
    let mut scored: Vec<(i32, usize, String)> = uniq
        .into_iter()
        .enumerate()
        .map(|(i, s)| (-score_sentence(&s), i, s))
        .collect();
    scored.sort();
    scored
        .into_iter()
        .take(max_items.max(1))
        .map(|(_, _, s)| s)
        .collect()
}

fn norm_space(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn assemble_report(
    en: bool,
    p: &PersonaToml,
    bullets: &[String],
    brief: &str,
    source_note: Option<&str>,
    truncated: bool,
    compliance: &str,
) -> String {
    let name_display = if en && !p.display_name_en.is_empty() {
        p.display_name_en.as_str()
    } else {
        p.display_name_zh.as_str()
    };
    let h_summary = if en {
        "### Data highlights"
    } else {
        "### 数据摘要（来自您提供的材料）"
    };
    let h_style = if en {
        "### Style-inspired commentary (education only)"
    } else {
        "### 风格化解读（灵感来源）"
    };
    let h_gap = if en {
        "### Limitations"
    } else {
        "### 局限与未覆盖信息"
    };
    let h_risk = if en {
        "### Disclaimer"
    } else {
        "### 风险提示与免责声明"
    };

    let mut out = Vec::new();
    if en {
        out.push(format!(
            "_Style inspiration:_ **{}**. This text is algorithmically composed for readability and investor education tone; it **does not** represent the cited figure, imply endorsement, or provide personalized investment advice.",
            name_display
        ));
    } else {
        out.push(format!(
            "【说明】以下内容按「{}」的公众表述与常见思考维度做结构化扩写（非该人士撰写或背书），仅供信息与投教语境下的阅读。**不构成**任何投资建议或可执行交易结论。",
            name_display
        ));
    }
    if truncated {
        out.push(if en {
            "- Note: Input was truncated to the maximum supported length.".to_string()
        } else {
            "- **提示**：您提供的正文较长，已对输入截取后再做摘要。「数据摘要」条目仅覆盖剩余可见片段中的重要句子。" .to_string()
        });
    }
    if bullets.is_empty() {
        out.push(if en {
            "- Insufficient distinctive sentences extracted; consider providing denser factual content.".to_string()
        } else {
            "- （信息不足以形成明确要点罗列；以下为极为有限的概括占位。）可补充更具体的事实句或数据来源说明。"
                .to_string()
        });
    } else {
        out.push(h_summary.to_string());
        for b in bullets {
            out.push(format!("- {}", b));
        }
    }

    let mut style_body = Vec::new();
    if !brief.trim().is_empty() {
        style_body.push(if en {
            format!("User focus: {}", brief)
        } else {
            format!(
                "您希望侧重的方向：**{}**。以下解读围绕该侧重点与材料对齐，但仍停留在方法与框架层面。",
                brief
            )
        });
    }
    let facet_take = if en {
        (3usize).min(p.facets_zh.len()).max(1)
    } else {
        p.facets_zh.len().min(4).max(1)
    };
    for (i, facet) in p.facets_zh.iter().take(facet_take).enumerate() {
        let ref_line = bullets
            .get(i % bullets.len().max(1))
            .map(|s| truncate_line(s, 120))
            .unwrap_or_else(|| "(no bullet)".to_string());
        if en {
            style_body.push(format!(
                "- **Perspective {}**: `{}` — relate this lens to observable phrases (example snippet: {}). Stay descriptive; avoid buy/sell instructions.",
                i + 1,
                facet,
                ref_line
            ));
        } else {
            style_body.push(format!(
                "- **{}**\n在您提供的材料中可以优先对照这样一句话或一类信息：「{}」。下文仅用于帮助读者把零散表述串成可被反复核对的问题清单，而非替您做账户层面的判断。",
                facet, ref_line
            ));
        }
    }
    if !p.prefer_zh.is_empty() && !en {
        let mut line = format!(
            "（可与该框架协同的措辞习惯包括但不限于：{}。",
            p.prefer_zh.join("、")
        );
        if !p.avoid_zh.is_empty() {
            line.push_str(&format!(
                " 建议避免情绪化或促销式用词：{}。",
                p.avoid_zh.join("、")
            ));
        }
        line.push_str("）");
        style_body.push(line);
    }
    out.push(format!("{}\n{}", h_style, style_body.join("\n")));

    let mut gap = Vec::new();
    gap.push(if en {
        "- This skill does not verify third-party excerpts; treat numbers and claims as potentially incomplete.".to_string()
    } else {
        "- 本技能不会对第三方摘录做独立核查；请自行核对数据来源与时效。".to_string()
    });
    if let Some(sn) = source_note {
        gap.push(if en {
            format!("Source note (user/agent supplied): {}", sn)
        } else {
            format!("材料来源备注（用户提供或编排填入）：{}", sn)
        });
    }
    out.push(format!("{}\n{}", h_gap, gap.join("\n")));

    let disc = disclaimer_block(en, compliance);
    out.push(format!("{}\n{}", h_risk, disc));

    out.join("\n\n")
}

fn truncate_line(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut t = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i >= max {
            break;
        }
        t.push(ch);
    }
    t.push('…');
    t
}

fn disclaimer_block(en: bool, compliance: &str) -> String {
    let base_zh =
        "市场有风险，入市需谨慎。以上为教学与信息组织用途，不提供证券买卖、仓位或收益承诺。";
    let base_en = "Markets involve risk; this output is educational and organizational only. It does not provide buy/sell instructions, position advice, or return guarantees.";
    if en {
        return if compliance == "light" {
            base_en.to_string()
        } else {
            format!(
                "{} If you rely on scraped content, additionally respect site terms/copyright/trading constraints applicable to your region.",
                base_en
            )
        };
    }
    if compliance == "light" {
        base_zh.to_string()
    } else {
        format!(
            "{} **若正文来自网页/第三方抓取**：请一并遵守数据来源网站的使用条款与不侵权要求；本输出不取代律师、合规顾问或投顾意见。",
            base_zh
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bullets_non_empty_from_sample() {
        let sample = "本公司2024年一季度营收同比上升12%。毛利率改善。\n风险提示：海外市场波动可能影响出口业务。";
        let b = summarize_bullets(sample, 5);
        assert!(!b.is_empty());
    }
}
