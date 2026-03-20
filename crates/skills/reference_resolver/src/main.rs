use anyhow::Result;
use regex::Regex;
use serde::Serialize;
use serde_json::{json, Value};
use std::cmp::Ordering;
use std::collections::HashSet;
use std::io::{self, BufRead, Write};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum CandidateKind {
    Reply,
    Task,
    File,
    Dependency,
    Generic,
}

impl CandidateKind {
    fn from_target_type(raw: &str) -> Self {
        match raw.trim().to_lowercase().as_str() {
            "reply" => Self::Reply,
            "task" => Self::Task,
            "file" => Self::File,
            "dependency" => Self::Dependency,
            _ => Self::Generic,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Reply => "reply",
            Self::Task => "task",
            Self::File => "file",
            Self::Dependency => "dependency",
            Self::Generic => "generic",
        }
    }
}

#[derive(Clone, Debug)]
struct Candidate {
    kind: CandidateKind,
    id: String,
    turn_index: Option<i64>,
    source: String,
    text: String,
    recency_rank: usize,
    score: f64,
    recency_score: f64,
    role_score: f64,
    keyword_score: f64,
    semantic_score: f64,
}

#[derive(Clone, Debug)]
struct ResolveInput {
    request_id: String,
    action: String,
    request_text: String,
    recent_turns: Vec<Value>,
    recent_results: Vec<Value>,
    target_type: CandidateKind,
    language_hint: Option<String>,
    max_candidates: usize,
    include_trace: bool,
}

#[derive(Serialize)]
struct CandidateOut {
    kind: String,
    id: String,
    turn_index: Option<i64>,
    source: String,
    score: f64,
    confidence: f64,
    preview: String,
}

#[derive(Serialize)]
struct ResolvedRef {
    kind: String,
    id: String,
    turn_index: Option<i64>,
    source: String,
    score: f64,
    preview: String,
}

#[derive(Serialize)]
struct ResolvePayload {
    status: String,
    confidence: f64,
    resolved_ref: Option<ResolvedRef>,
    top_candidates: Vec<CandidateOut>,
    clarify_question: Option<String>,
    message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    resolution_trace: Option<Value>,
}

fn main() -> Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    for line in stdin.lock().lines() {
        let line = line?;
        let request: Value = serde_json::from_str(&line).unwrap_or_else(|_| json!({"request_id":"unknown"}));
        let parsed = parse_input(&request);
        let request_id = parsed.request_id.clone();
        let action = parsed.action.clone();
        let payload = if action != "resolve_reference" {
            ResolvePayload {
                status: "not_found".to_string(),
                confidence: 0.0,
                resolved_ref: None,
                top_candidates: vec![],
                clarify_question: None,
                message: Some(format!("unsupported action: {action}")),
                resolution_trace: parsed.include_trace.then(|| json!({"reason":"unsupported_action"})),
            }
        } else {
            resolve_reference(parsed)
        };

        let out = json!({
            "request_id": request_id,
            "status": "ok",
            "text": serde_json::to_string(&payload)?,
            "error_text": Value::Null,
            "extra": { "action": "resolve_reference" }
        });
        writeln!(stdout, "{}", serde_json::to_string(&out)?)?;
        stdout.flush()?;
    }

    Ok(())
}

fn parse_input(v: &Value) -> ResolveInput {
    let args = v.get("args").unwrap_or(v);
    let request_id = v
        .get("request_id")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let action = pick_str(args, &["action"])
        .or_else(|| pick_str(v, &["action"]))
        .unwrap_or_else(|| "resolve_reference".to_string());
    let request_text = pick_str(args, &["request_text", "text", "query"]).unwrap_or_default();
    let recent_turns = args
        .get("recent_turns")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let recent_results = args
        .get("recent_results")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let target_type_raw = pick_str(args, &["target_type"]).unwrap_or_else(|| "generic".to_string());
    let language_hint = pick_str(args, &["language_hint"]);
    let max_candidates = args
        .get("max_candidates")
        .and_then(Value::as_u64)
        .map(|n| n as usize)
        .unwrap_or(5)
        .clamp(1, 10);
    let include_trace = args.get("include_trace").and_then(Value::as_bool).unwrap_or(false);

    ResolveInput {
        request_id,
        action,
        request_text,
        recent_turns,
        recent_results,
        target_type: CandidateKind::from_target_type(&target_type_raw),
        language_hint,
        max_candidates,
        include_trace,
    }
}

fn resolve_reference(input: ResolveInput) -> ResolvePayload {
    let mut candidates = collect_candidates(&input);
    if candidates.is_empty() {
        return ResolvePayload {
            status: "not_found".to_string(),
            confidence: 0.0,
            resolved_ref: None,
            top_candidates: vec![],
            clarify_question: None,
            message: Some("no bindable reference found in recent context".to_string()),
            resolution_trace: input.include_trace.then(|| json!({"candidate_count":0})),
        };
    }

    score_candidates(&mut candidates, &input);
    candidates.sort_by(compare_candidate);

    let top = &candidates[0];
    let second_score = candidates.get(1).map(|c| c.score).unwrap_or(0.0);
    let gap = (top.score - second_score).max(0.0);
    let confidence = confidence_from(top.score, gap);

    let top_candidates: Vec<CandidateOut> = candidates
        .iter()
        .take(input.max_candidates)
        .map(candidate_out)
        .collect();

    let ambiguous = is_ambiguous(top.score, gap, candidates.get(1));
    let forced_not_found = confidence < 0.38 && !ambiguous;

    if forced_not_found {
        return ResolvePayload {
            status: "not_found".to_string(),
            confidence: round2(confidence),
            resolved_ref: None,
            top_candidates,
            clarify_question: None,
            message: Some("reference target not found with enough confidence".to_string()),
            resolution_trace: trace_value(&input, &candidates, top.score, gap),
        };
    }

    if ambiguous {
        return ResolvePayload {
            status: "ambiguous".to_string(),
            confidence: round2(confidence),
            resolved_ref: None,
            top_candidates,
            clarify_question: Some(build_clarify_question(&input, &candidates)),
            message: Some("multiple plausible references found".to_string()),
            resolution_trace: trace_value(&input, &candidates, top.score, gap),
        };
    }

    ResolvePayload {
        status: "resolved".to_string(),
        confidence: round2(confidence),
        resolved_ref: Some(ResolvedRef {
            kind: top.kind.as_str().to_string(),
            id: top.id.clone(),
            turn_index: top.turn_index,
            source: top.source.clone(),
            score: round2(top.score),
            preview: preview(&top.text, 180),
        }),
        top_candidates,
        clarify_question: None,
        message: None,
        resolution_trace: trace_value(&input, &candidates, top.score, gap),
    }
}

fn collect_candidates(input: &ResolveInput) -> Vec<Candidate> {
    let mut out = vec![];

    for (i, turn) in input.recent_turns.iter().rev().enumerate() {
        let role = pick_str(turn, &["role", "speaker"]).unwrap_or_else(|| "unknown".to_string());
        let role_l = role.to_lowercase();
        let text = pick_str(turn, &["text", "content", "message", "short_preview"]).unwrap_or_default();
        let id = pick_str(turn, &["id", "turn_id"]).unwrap_or_else(|| format!("turn_{}", i + 1));
        let turn_index = turn.get("turn_index").and_then(Value::as_i64);

        if !text.trim().is_empty() {
            let kind = if role_l.contains("assistant") {
                CandidateKind::Reply
            } else if role_l.contains("tool") {
                CandidateKind::Task
            } else {
                CandidateKind::Generic
            };
            out.push(Candidate {
                kind,
                id,
                turn_index,
                source: "recent_turns".to_string(),
                text,
                recency_rank: i,
                score: 0.0,
                recency_score: 0.0,
                role_score: 0.0,
                keyword_score: 0.0,
                semantic_score: 0.0,
            });
        }

        if let Some(path) = pick_str(turn, &["path", "file", "file_path"]) {
            if looks_like_path(&path) {
                out.push(Candidate {
                    kind: CandidateKind::File,
                    id: format!("file:{path}"),
                    turn_index,
                    source: "recent_turns".to_string(),
                    text: path,
                    recency_rank: i,
                    score: 0.0,
                    recency_score: 0.0,
                    role_score: 0.0,
                    keyword_score: 0.0,
                    semantic_score: 0.0,
                });
            }
        }

        if let Some(dep_arr) = turn.get("dependencies").and_then(Value::as_array) {
            for dep in dep_arr {
                if let Some(dep_name) = dep.as_str() {
                    out.push(Candidate {
                        kind: CandidateKind::Dependency,
                        id: format!("dependency:{dep_name}"),
                        turn_index,
                        source: "recent_turns".to_string(),
                        text: dep_name.to_string(),
                        recency_rank: i,
                        score: 0.0,
                        recency_score: 0.0,
                        role_score: 0.0,
                        keyword_score: 0.0,
                        semantic_score: 0.0,
                    });
                }
            }
        }
    }

    for (i, item) in input.recent_results.iter().rev().enumerate() {
        let text = pick_str(item, &["text", "output", "summary", "result"]).unwrap_or_default();
        let id = pick_str(item, &["id", "task_id", "request_id"]).unwrap_or_else(|| format!("result_{}", i + 1));
        if !text.trim().is_empty() {
            out.push(Candidate {
                kind: CandidateKind::Task,
                id,
                turn_index: item.get("turn_index").and_then(Value::as_i64),
                source: "recent_results".to_string(),
                text,
                recency_rank: i,
                score: 0.0,
                recency_score: 0.0,
                role_score: 0.0,
                keyword_score: 0.0,
                semantic_score: 0.0,
            });
        }

        if let Some(path) = pick_str(item, &["path", "file", "file_path"]) {
            if looks_like_path(&path) {
                out.push(Candidate {
                    kind: CandidateKind::File,
                    id: format!("file:{path}"),
                    turn_index: item.get("turn_index").and_then(Value::as_i64),
                    source: "recent_results".to_string(),
                    text: path,
                    recency_rank: i,
                    score: 0.0,
                    recency_score: 0.0,
                    role_score: 0.0,
                    keyword_score: 0.0,
                    semantic_score: 0.0,
                });
            }
        }
    }

    dedup_candidates(out)
}

fn dedup_candidates(items: Vec<Candidate>) -> Vec<Candidate> {
    let mut seen = HashSet::new();
    let mut out = vec![];
    for c in items {
        let key = format!("{}|{}|{}", c.kind.as_str(), c.id, c.text);
        if seen.insert(key) {
            out.push(c);
        }
    }
    out
}

fn score_candidates(candidates: &mut [Candidate], input: &ResolveInput) {
    let request = input.request_text.to_lowercase();
    let tokens_req = tokenize(&request);

    let ordinal_last = contains_any(&request, &["上个", "上一条", "last reply", "previous reply", "previous response"]);
    let ordinal_prev2 = contains_any(&request, &["上上个", "上上条", "reply before that", "second last"]);
    let file_hint = contains_any(&request, &["文件", "file", "路径", "path"]);
    let dep_hint = contains_any(&request, &["依赖", "dependency", "package", "模块", "module"]);
    let task_hint = contains_any(&request, &["任务", "结果", "run", "执行", "output", "command result"]);

    for c in candidates {
        let recency = 1.0 / ((c.recency_rank + 1) as f64);
        c.recency_score = recency * 42.0;

        c.role_score = match input.target_type {
            CandidateKind::Generic => match c.kind {
                CandidateKind::Reply => 18.0,
                CandidateKind::Task => 14.0,
                CandidateKind::File => 14.0,
                CandidateKind::Dependency => 10.0,
                CandidateKind::Generic => 8.0,
            },
            t if t == c.kind => 24.0,
            _ => -14.0,
        };

        let mut keyword = 0.0;
        if ordinal_last && c.kind == CandidateKind::Reply && c.recency_rank == 0 {
            keyword += 24.0;
        }
        if ordinal_prev2 && c.kind == CandidateKind::Reply && c.recency_rank == 1 {
            keyword += 24.0;
        }
        if file_hint && c.kind == CandidateKind::File {
            keyword += 16.0;
        }
        if dep_hint && c.kind == CandidateKind::Dependency {
            keyword += 16.0;
        }
        if task_hint && c.kind == CandidateKind::Task {
            keyword += 14.0;
        }
        c.keyword_score = keyword;

        let sim = lexical_similarity(&tokens_req, &tokenize(&c.text.to_lowercase()));
        c.semantic_score = sim * 18.0;

        let raw_score = c.recency_score + c.role_score + c.keyword_score + c.semantic_score;
        c.score = raw_score.clamp(0.0, 100.0);
    }
}

fn compare_candidate(a: &Candidate, b: &Candidate) -> Ordering {
    b.score
        .partial_cmp(&a.score)
        .unwrap_or(Ordering::Equal)
        .then_with(|| a.recency_rank.cmp(&b.recency_rank))
}

fn confidence_from(score: f64, gap: f64) -> f64 {
    let score_part = (score / 100.0).clamp(0.0, 1.0);
    let gap_part = (gap / 20.0).clamp(0.0, 1.0);
    (score_part * 0.78 + gap_part * 0.22).clamp(0.0, 1.0)
}

fn is_ambiguous(top_score: f64, gap: f64, second: Option<&Candidate>) -> bool {
    if top_score < 45.0 {
        return true;
    }
    if let Some(s2) = second {
        if (top_score - s2.score).abs() < 8.0 {
            return true;
        }
    }
    gap < 9.0
}

fn build_clarify_question(input: &ResolveInput, candidates: &[Candidate]) -> String {
    let kind_text = match input.target_type {
        CandidateKind::Reply => "reply",
        CandidateKind::Task => "task result",
        CandidateKind::File => "file",
        CandidateKind::Dependency => "dependency",
        CandidateKind::Generic => "reference target",
    };
    let a = candidates
        .first()
        .map(|c| preview(&c.text, 60))
        .unwrap_or_else(|| "candidate A".to_string());
    let b = candidates
        .get(1)
        .map(|c| preview(&c.text, 60))
        .unwrap_or_else(|| "candidate B".to_string());

    let zh = input
        .language_hint
        .as_deref()
        .map(|l| l.to_lowercase().starts_with("zh"))
        .unwrap_or_else(|| contains_any(&input.request_text, &["上个", "这个", "那个", "依赖", "文件"]));
    if zh {
        format!("我需要确认你指的是哪个{kind_text}：A) {a} 还是 B) {b}？")
    } else {
        format!("Please confirm which {kind_text} you mean: A) {a} or B) {b}?")
    }
}

fn trace_value(input: &ResolveInput, candidates: &[Candidate], top_score: f64, gap: f64) -> Option<Value> {
    if !input.include_trace {
        return None;
    }
    Some(json!({
        "target_type": input.target_type.as_str(),
        "candidate_count": candidates.len(),
        "top_score": round2(top_score),
        "gap_to_second": round2(gap),
        "scoring": candidates.iter().take(5).map(|c| json!({
            "id": c.id,
            "kind": c.kind.as_str(),
            "score": round2(c.score),
            "recency_score": round2(c.recency_score),
            "role_score": round2(c.role_score),
            "keyword_score": round2(c.keyword_score),
            "semantic_score": round2(c.semantic_score),
        })).collect::<Vec<_>>()
    }))
}

fn candidate_out(c: &Candidate) -> CandidateOut {
    CandidateOut {
        kind: c.kind.as_str().to_string(),
        id: c.id.clone(),
        turn_index: c.turn_index,
        source: c.source.clone(),
        score: round2(c.score),
        confidence: round2((c.score / 100.0).clamp(0.0, 1.0)),
        preview: preview(&c.text, 180),
    }
}

fn preview(s: &str, n: usize) -> String {
    let clean = s.replace('\n', " ").replace('\r', " ").trim().to_string();
    if clean.chars().count() <= n {
        return clean;
    }
    let out: String = clean.chars().take(n).collect();
    format!("{out}...")
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

fn pick_str(v: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|k| v.get(*k))
        .and_then(Value::as_str)
        .map(|s| s.to_string())
}

fn contains_any(text: &str, terms: &[&str]) -> bool {
    terms.iter().any(|t| text.contains(t))
}

fn looks_like_path(s: &str) -> bool {
    let path_re = Regex::new(r"(/|\\|[a-zA-Z]:\\|\.md$|\.txt$|\.rs$|\.py$|\.json$)").expect("valid regex");
    path_re.is_match(s)
}

fn tokenize(s: &str) -> HashSet<String> {
    let token_re = Regex::new(r"[[:alnum:]_./-]{2,}").expect("valid regex");
    token_re
        .find_iter(s)
        .map(|m| m.as_str().to_lowercase())
        .collect::<HashSet<_>>()
}

fn lexical_similarity(a: &HashSet<String>, b: &HashSet<String>) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let inter = a.intersection(b).count() as f64;
    let union = a.union(b).count() as f64;
    if union <= 0.0 {
        return 0.0;
    }
    inter / union
}
