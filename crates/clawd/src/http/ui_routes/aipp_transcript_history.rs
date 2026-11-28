/// SQLite selects candidates. Only the completed task's typed delivery record
/// can classify an Agent transcript. A keyword in a reply is never evidence.
const OWNED_TRANSCRIPT_TASKS: &str = "
WITH owned AS (
    SELECT rowid, task_id, status, created_at, result_json,
           CASE WHEN json_valid(payload_json) THEN
               CASE WHEN json_type(payload_json, '$.attachments[0].name') = 'text'
                    THEN json_extract(payload_json, '$.attachments[0].name') END
           END AS name,
           CASE WHEN channel = 'ui' AND json_valid(payload_json) THEN
               COALESCE(json_extract(payload_json, '$.entrypoint') = ?2
                    AND json_extract(payload_json, '$.aipp_skill') = ?3, 0)
           ELSE 0 END AS from_app
    FROM tasks WHERE principal_id = ?1 AND kind = 'ask'
)
SELECT rowid, task_id, status, created_at, name, from_app, result_json
FROM owned
WHERE (from_app OR (status = 'succeeded' AND (
       instr(result_json, 'transcript-review:') > 0 OR
       instr(result_json, 'transcript-fallback:') > 0 OR
       instr(result_json, 'transcription_delivery') > 0)))";

const TRANSCRIPT_HISTORY_PAGE_SIZE: usize = 20;

#[derive(Debug, Default, Deserialize)]
struct AippTranscriptQuery {
    before: Option<i64>,
}

#[derive(Debug, Serialize)]
struct TranscriptHistoryItem {
    task_id: String,
    status: String,
    created_at: Option<u64>,
    name: Option<String>,
    source: &'static str,
}

enum HistoryTranscript {
    App {
        text: Option<String>,
        error_code: Option<String>,
        retryable: Option<bool>,
    },
    AgentArtifact(crate::task_artifacts::TaskArtifactManifest),
    AgentInline(String),
}

fn history_transcript(
    task_id: &str,
    from_app: bool,
    result_json: Option<String>,
) -> Option<HistoryTranscript> {
    let result = result_json.and_then(|raw| serde_json::from_str::<Value>(&raw).ok());
    if from_app {
        let field = |key| result.as_ref()?.get(key)?.as_str().map(str::to_owned);
        return Some(HistoryTranscript::App {
            text: field("text"),
            error_code: field("error_code"),
            retryable: result.as_ref().and_then(|value| value["retryable"].as_bool()),
        });
    }
    let result = result?;
    if let Some(artifact) = crate::task_artifacts::transcript_delivery_manifest(task_id, &result) {
        return Some(HistoryTranscript::AgentArtifact(artifact));
    }
    // Older Agent results delivered the complete transcript inline and recorded
    // its length. Require both structured delivery and a matching final reply.
    let text = result.get("text")?.as_str()?;
    if text.trim().is_empty() || text.len() > crate::task_artifacts::MAX_HISTORY_TRANSCRIPT_BYTES {
        return None;
    }
    let length = text.chars().count() as u64;
    let deliveries = result
        .pointer("/task_journal/trace/capability_results")?
        .as_array()?;
    deliveries
        .iter()
        .any(|record| {
            record["status"] == "ok"
                && record
                    .pointer("/data/extra/transcription_review/schema_version")
                    .and_then(Value::as_u64)
                    == Some(1)
                && record
                    .pointer("/data/extra/transcription_review/raw_text")
                    .and_then(Value::as_str)
                    .is_some_and(|raw| !raw.trim().is_empty())
                && record
                    .pointer("/data/extra/transcription_delivery")
                    .is_some_and(|delivery| {
                        delivery["mode"] == "inline"
                            && delivery["reviewed_by_model"] == true
                            && delivery["character_count"].as_u64() == Some(length)
                    })
        })
        .then(|| HistoryTranscript::AgentInline(text.to_owned()))
}

fn transcript_history_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<Option<(i64, TranscriptHistoryItem, HistoryTranscript)>> {
    let task_id = row.get::<_, String>(1)?;
    if uuid::Uuid::parse_str(&task_id).is_err() {
        return Ok(None);
    }
    let from_app = row.get::<_, bool>(5)?;
    let Some(transcript) = history_transcript(&task_id, from_app, row.get(6)?) else {
        return Ok(None);
    };
    let name = row
        .get::<_, Option<String>>(4)?
        .or_else(|| match &transcript {
            HistoryTranscript::AgentArtifact(manifest) => Some(manifest.filename.clone()),
            HistoryTranscript::AgentInline(_) => Some("transcript.txt".to_owned()),
            HistoryTranscript::App { .. } => None,
        });
    Ok(Some((
        row.get(0)?,
        TranscriptHistoryItem {
            task_id,
            status: row.get(2)?,
            created_at: row.get::<_, String>(3)?.parse().ok(),
            name,
            source: if from_app { "aipp" } else { "agent" },
        },
        transcript,
    )))
}

async fn list_aipp_transcripts(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(skill): AxumPath<String>,
    Query(query): Query<AippTranscriptQuery>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    tokio::task::spawn_blocking(move || {
        let (actor, _) = match authorize_aipp_transcripts(&state, &headers, &skill) {
            Ok(auth) => auth,
            Err(error) => return error,
        };
        let result = (|| -> anyhow::Result<Value> {
            let db = state.core.db.get()?;
            let mut statement = db.prepare(&format!(
                "{OWNED_TRANSCRIPT_TASKS} AND rowid < ?4 ORDER BY rowid DESC"
            ))?;
            let mut rows = statement.query(rusqlite::params![
                actor.principal_id,
                TRANSCRIPT_ENTRYPOINT,
                skill,
                query.before.unwrap_or(i64::MAX)
            ])?;
            let mut items = Vec::with_capacity(TRANSCRIPT_HISTORY_PAGE_SIZE);
            let mut last = None;
            let mut next = None;
            while let Some(row) = rows.next()? {
                let Some((seq, item, _)) = transcript_history_row(row)? else {
                    continue;
                };
                if items.len() == TRANSCRIPT_HISTORY_PAGE_SIZE {
                    next = last;
                    break;
                }
                last = Some(seq);
                items.push(item);
            }
            Ok(json!({ "items": items, "next": next }))
        })();
        match result {
            Ok(page) => crate::api_ok(page),
            Err(error) => {
                tracing::warn!(%error, "aipp_transcript_history_read_failed");
                transcript_history_read_error()
            }
        }
    })
    .await
    .unwrap_or_else(|_| transcript_history_read_error())
}

async fn read_aipp_transcript(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath((skill, task_id)): AxumPath<(String, uuid::Uuid)>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    tokio::task::spawn_blocking(move || {
        let (actor, _) = match authorize_aipp_transcripts(&state, &headers, &skill) {
            Ok(auth) => auth,
            Err(error) => return error,
        };
        let result = (|| -> anyhow::Result<_> {
            let db = state.core.db.get()?;
            Ok(db.query_row(
                &format!("{OWNED_TRANSCRIPT_TASKS} AND task_id = ?4"),
                rusqlite::params![actor.principal_id, TRANSCRIPT_ENTRYPOINT, skill, task_id.to_string()],
                transcript_history_row,
            ).optional()?.flatten())
        })();
        match result {
            Ok(Some((_, item, transcript))) => {
                let (text, error_code, retryable) = match transcript {
                    HistoryTranscript::App { text, error_code, retryable } =>
                        (text.filter(|_| item.status == "succeeded"), error_code, retryable),
                    HistoryTranscript::AgentInline(text) => (Some(text), None, None),
                    HistoryTranscript::AgentArtifact(manifest) => {
                        let workspace = &state.skill_rt.workspace_root;
                        let owner = state.identity_files_root(&actor).ok();
                        let text = crate::task_artifacts::read_history_transcript(
                            workspace, owner.as_deref(), &item.task_id, &manifest,
                        );
                        let Some(text) = text else {
                            return aipp_api_error(StatusCode::GONE, "aipp_transcript_file_unavailable");
                        };
                        (Some(text), None, None)
                    }
                };
                let mut result = json!({ "task_id": item.task_id, "status": item.status,
                    "name": item.name, "source": item.source, "text": text, "error_code": error_code });
                if let Some(retryable) = retryable {
                    result["retryable"] = json!(retryable);
                }
                crate::api_ok(result)
            }
            Ok(None) => aipp_api_error(StatusCode::NOT_FOUND, "aipp_transcript_not_found"),
            Err(error) => {
                tracing::warn!(%error, "aipp_transcript_record_read_failed");
                transcript_history_read_error()
            }
        }
    }).await.unwrap_or_else(|_| transcript_history_read_error())
}

fn transcript_history_read_error() -> (StatusCode, Json<ApiResponse<Value>>) {
    aipp_api_error(
        StatusCode::INTERNAL_SERVER_ERROR,
        "aipp_transcript_read_failed",
    )
}
