//! Bind each external citation to text observed at that URL in this task.
use std::sync::LazyLock;

use claw_core::capability_result::CapabilityResultStatus;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::task_journal::TaskJournal;

static WEB_LINK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"https?://[^\s<>"\x5d]+"#).expect("constant URL expression"));

#[derive(Serialize)]
pub(super) struct Citation<'a> {
    index: usize,
    url: &'a str,
    excerpts: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct CitationCheck {
    index: usize,
    quote: String,
    supported: bool,
}

pub(super) fn citations<'a>(answer: &'a str, journal: &TaskJournal) -> Vec<Citation<'a>> {
    let mut citations: Vec<_> = WEB_LINK
        .find_iter(answer)
        .take(65)
        .enumerate()
        .map(|(index, hit)| {
            let mut url = hit
                .as_str()
                .trim_end_matches(['.', ',', ';', '。', '，', '）']);
            while url.ends_with(')')
                && url.bytes().filter(|b| *b == b')').count()
                    > url.bytes().filter(|b| *b == b'(').count()
            {
                url = &url[..url.len() - 1];
            }
            Citation {
                index,
                url,
                excerpts: Vec::new(),
            }
        })
        .collect();
    if citations.is_empty() {
        return citations;
    }
    let mut observed_sources = false;
    for result in &journal.capability_results {
        if result.status != CapabilityResultStatus::Ok {
            continue;
        }
        let observation = crate::capability_result::explicit_model_observation(&result.data)
            .unwrap_or(&result.data);
        collect(observation, &mut citations, &mut observed_sources, 0);
    }
    if !observed_sources {
        citations.clear();
    }
    citations
}

fn collect(
    value: &Value,
    citations: &mut [Citation<'_>],
    observed_sources: &mut bool,
    depth: usize,
) {
    if depth > 12 {
        return;
    }
    match value {
        Value::Array(items) => {
            for item in items {
                collect(item, citations, observed_sources, depth + 1);
            }
        }
        Value::Object(row) => {
            let urls = ["url", "link", "final_url", "canonical_url"]
                .iter()
                .filter_map(|key| row.get(*key)?.as_str())
                .filter(|url| url.starts_with("https://") || url.starts_with("http://"))
                .collect::<Vec<_>>();
            if !urls.is_empty() {
                for key in ["title", "excerpt", "content_excerpt", "snippet", "description", "text"] {
                    let Some(text) = row
                        .get(key)
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|v| !v.is_empty())
                    else {
                        continue;
                    };
                    *observed_sources = true;
                    for citation in citations.iter_mut().filter(|c| urls.contains(&c.url)) {
                        let excerpt: String = crate::visible_text::sanitize_user_visible_text(text)
                            .chars()
                            .take(1000)
                            .collect();
                        let (excerpt, _) =
                            crate::skill_output_artifact::sensitivity_aware_text_model_view(
                                &excerpt,
                            );
                        if citation.excerpts.len() < 4
                            && !excerpt.is_empty()
                            && !citation.excerpts.contains(&excerpt)
                        {
                            citation.excerpts.push(excerpt);
                        }
                    }
                }
            }
            for key in [
                "output",
                "extra",
                "items",
                "results",
                "candidates",
                "pages",
                "entries",
                "source_refs",
                "categories",
                "news",
                "groups",
            ] {
                if let Some(child) = row.get(key) {
                    collect(child, citations, observed_sources, depth + 1);
                }
            }
        }
        _ => {}
    }
}

/// Keep complete, source-bound rows for a wording repair. Bound the additional
/// prompt data independently of the journal trace.
pub(crate) fn repair_sources(answer: &str, journal: &TaskJournal) -> Vec<Value> {
    let mut rows = Vec::new();
    let mut bytes = 0;
    for citation in citations(answer, journal) {
        let row = serde_json::to_value(citation).expect("serializable citation");
        bytes += row.to_string().len();
        if bytes > 16_384 {
            break;
        }
        rows.push(row);
    }
    rows
}

pub(super) fn missing_checks(citations: &[Citation<'_>], checks: &[CitationCheck]) -> Vec<String> {
    citations
        .iter()
        .filter(|citation| {
            let mut matching = checks.iter().filter(|check| check.index == citation.index);
            let Some(check) = matching.next() else {
                return true;
            };
            let quote = check.quote.trim();
            matching.next().is_some()
                || !check.supported
                || quote.is_empty()
                || !citation
                    .excerpts
                    .iter()
                    .any(|source| source.contains(quote))
        })
        .map(|citation| format!("citation[{}]", citation.index))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn journal_rows_bind_only_their_own_excerpts_to_answer_urls() {
        use claw_core::capability_result::CapabilityResultEnvelope;
        let mut journal = TaskJournal::default();
        let answer = "[second](https://example.org/b) [first](https://example.org/a) https://example.org/unknown";
        assert!(citations(answer, &journal).is_empty());
        journal
            .capability_results
            .push(CapabilityResultEnvelope::ok(
                "fixture.read",
                None,
                serde_json::json!({"extra":{"items":[
                    {"link":"https://example.org/a","title":"A couple shares its finances","excerpt":"The couple reported a monthly total of 500."},
                    {"link":"https://example.org/b","title":"Vehicle registration"}
                ]}}),
            ));
        let rows = citations(answer, &journal);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].url, "https://example.org/b");
        assert_eq!(rows[0].excerpts, ["Vehicle registration"]);
        assert_eq!(rows[1].excerpts, ["A couple shares its finances", "The couple reported a monthly total of 500."]);
        assert!(rows[2].excerpts.is_empty());
    }

    #[test]
    fn passing_verdict_requires_each_citations_own_observed_words() {
        let refs = vec![
            Citation {
                index: 0,
                url: "https://example.org/a",
                excerpts: vec!["shares fell".into()],
            },
            Citation {
                index: 1,
                url: "https://example.org/b",
                excerpts: vec!["vehicle registration".into()],
            },
        ];
        let rows = |supported, quote: &str| {
            vec![
                CitationCheck {
                    index: 0,
                    supported,
                    quote: quote.into(),
                },
                CitationCheck {
                    index: 1,
                    supported: true,
                    quote: "registration".into(),
                },
            ]
        };
        assert!(missing_checks(&refs, &rows(true, "shares fell")).is_empty());
        for bad in [
            rows(false, "shares fell"),
            rows(true, "registration"),
            rows(true, ""),
        ] {
            assert_eq!(missing_checks(&refs, &bad), vec!["citation[0]"]);
        }
        assert_eq!(missing_checks(&refs, &[]).len(), 2);
    }
}
