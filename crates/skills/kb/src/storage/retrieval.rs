use std::collections::{HashMap, HashSet};
use std::time::Instant;

use anyhow::Result;
use rusqlite::{params, params_from_iter, types::Value as SqlValue, Connection, OptionalExtension};

use super::database::{count_rows, ensure_schema, has_fts, open};
use super::namespace_index::{load_documents_by_paths, load_namespace_header};
use super::vector_index::ann_projection_available;
use super::{nonnegative_usize, Chunk, KbRuntime, SearchCandidateSet, SearchFilters};

const EXACT_VECTOR_LIMIT_STANDARD: usize = 4_096;
const EXACT_VECTOR_LIMIT_CONSTRAINED: usize = 1_024;
const ANN_MAX_CANDIDATES_STANDARD: usize = 4_096;
const ANN_MAX_CANDIDATES_CONSTRAINED: usize = 1_024;

pub(super) struct SemanticCandidateSet {
    pub(super) scores: Vec<(String, f64)>,
    pub(super) profile_id: String,
    pub(super) generation: Option<u64>,
    pub(super) degradation: Option<String>,
    pub(super) backend: &'static str,
    pub(super) rows_scored: usize,
}

pub(crate) fn load_search_candidates(
    runtime: &KbRuntime,
    namespace: &str,
    terms: &[String],
    query_vector: &[f32],
    filters: SearchFilters<'_>,
    limit: usize,
) -> Result<SearchCandidateSet> {
    let mut db = open(runtime)?;
    ensure_schema(&mut db)?;
    let mut index = load_namespace_header(&db, &runtime.scope_user_key, namespace)?;
    let total_chunks = count_rows(
        &db,
        "kb_chunks",
        "owner_user_key = ?1 AND namespace = ?2",
        &runtime.scope_user_key,
        namespace,
    )?;
    let source_prefix = format!("kb:{}:{}:", runtime.scope_user_key.trim(), namespace.trim());
    let lexical_started = Instant::now();
    let lexical_chunk_ids = if has_fts(&db)? && !terms.is_empty() {
        fts_candidate_chunk_ids(
            &db,
            &runtime.scope_user_key,
            namespace,
            &source_prefix,
            terms,
            filters,
            limit,
        )
        .unwrap_or_default()
    } else {
        Vec::new()
    };
    let lexical_latency_ms = elapsed_ms(lexical_started);
    let semantic_started = Instant::now();
    let semantic_candidates = semantic_candidate_scores(
        &db,
        &runtime.scope_user_key,
        namespace,
        query_vector,
        filters,
        limit,
        if runtime.constrained_storage() {
            EXACT_VECTOR_LIMIT_CONSTRAINED
        } else {
            EXACT_VECTOR_LIMIT_STANDARD
        },
        if runtime.constrained_storage() {
            ANN_MAX_CANDIDATES_CONSTRAINED
        } else {
            ANN_MAX_CANDIDATES_STANDARD
        },
    )?;
    let semantic_latency_ms = elapsed_ms(semantic_started);
    let lexical_set = lexical_chunk_ids.iter().cloned().collect::<HashSet<_>>();
    let semantic_scores = semantic_candidates
        .scores
        .iter()
        .cloned()
        .collect::<HashMap<_, _>>();
    let mut chunk_ids = lexical_chunk_ids;
    let mut seen = lexical_set.clone();
    for (chunk_id, _) in semantic_candidates.scores {
        if seen.insert(chunk_id.clone()) {
            chunk_ids.push(chunk_id);
        }
    }
    let retrieval_mode = match (lexical_set.is_empty(), semantic_scores.is_empty()) {
        (false, false) => "hybrid_candidates",
        (false, true) => "fts5_candidates",
        (true, false) => "semantic_candidates",
        (true, true) => "no_match",
    };
    let materialization_started = Instant::now();
    index.chunks = load_chunks_by_ids(&db, &runtime.scope_user_key, namespace, &chunk_ids)?;
    let document_paths = index
        .chunks
        .iter()
        .map(|chunk| chunk.path.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    index.docs = load_documents_by_paths(&db, &runtime.scope_user_key, namespace, &document_paths)?;
    let materialization_latency_ms = elapsed_ms(materialization_started);
    Ok(SearchCandidateSet {
        index,
        total_chunks,
        retrieval_mode,
        lexical_chunk_ids: lexical_set,
        semantic_scores,
        embedding_profile_id: semantic_candidates.profile_id,
        vector_generation: semantic_candidates.generation,
        vector_degradation: semantic_candidates.degradation,
        lexical_latency_ms,
        semantic_latency_ms,
        materialization_latency_ms,
        semantic_backend: semantic_candidates.backend,
        semantic_rows_scored: semantic_candidates.rows_scored,
    })
}

fn elapsed_ms(started: Instant) -> f64 {
    (started.elapsed().as_secs_f64() * 1_000.0 * 1_000.0).round() / 1_000.0
}

fn fts_candidate_chunk_ids(
    db: &Connection,
    owner: &str,
    namespace: &str,
    source_prefix: &str,
    terms: &[String],
    filters: SearchFilters<'_>,
    limit: usize,
) -> Result<Vec<String>> {
    let query = terms
        .iter()
        .filter(|term| !term.trim().is_empty())
        .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" OR ");
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let path_pattern = filters.path_prefix.map(path_descendant_pattern);
    let mut stmt = db.prepare(
        "SELECT c.chunk_id
         FROM memory_retrieval_index_fts
         JOIN memory_retrieval_index i ON i.id = memory_retrieval_index_fts.rowid
         JOIN kb_chunks c
           ON c.owner_user_key = ?2
          AND c.namespace = ?3
          AND c.chunk_id = substr(i.source_ref, length(?4) + 1)
         WHERE memory_retrieval_index_fts MATCH ?1
           AND i.source_kind = 'kb_doc'
           AND i.user_key = ?2
           AND substr(i.source_ref, 1, length(?4)) = ?4
           AND (?5 IS NULL OR c.document_path = ?5 OR c.document_path LIKE ?6 ESCAPE '\\')
           AND (?7 IS NULL OR c.file_type = ?7)
           AND (?8 IS NULL OR c.mtime_epoch >= ?8)
           AND (?9 IS NULL OR c.mtime_epoch <= ?9)
         ORDER BY bm25(memory_retrieval_index_fts), i.id
         LIMIT ?10",
    )?;
    let chunk_ids = stmt
        .query_map(
            params![
                query,
                owner,
                namespace,
                source_prefix,
                filters.path_prefix,
                path_pattern,
                filters.file_type,
                filters.time_from,
                filters.time_to,
                limit.max(1) as i64
            ],
            |row| row.get::<_, String>(0),
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(chunk_ids)
}

pub(super) fn semantic_candidate_scores(
    db: &Connection,
    owner: &str,
    namespace: &str,
    query_vector: &[f32],
    filters: SearchFilters<'_>,
    limit: usize,
    exact_vector_limit: usize,
    ann_max_candidates: usize,
) -> Result<SemanticCandidateSet> {
    let profile = claw_core::retrieval_embedding::local_hash_profile();
    if query_vector.is_empty() {
        return Ok(SemanticCandidateSet {
            scores: Vec::new(),
            profile_id: profile.profile_id,
            generation: None,
            degradation: Some("embedding_query_empty".to_string()),
            backend: "none",
            rows_scored: 0,
        });
    }
    let active = db
        .query_row(
            "SELECT n.active_vector_generation, n.revision,
                    g.profile_id, g.namespace_revision, g.state, g.indexed_rows,
                    p.model_id, p.dimensions, p.normalization, p.profile_version
             FROM kb_namespaces n
             LEFT JOIN kb_vector_generations g
               ON g.owner_user_key = n.owner_user_key
              AND g.namespace = n.namespace
              AND g.generation = n.active_vector_generation
             LEFT JOIN kb_embedding_profiles p ON p.profile_id = g.profile_id
             WHERE n.owner_user_key = ?1 AND n.namespace = ?2",
            params![owner, namespace],
            |row| {
                Ok((
                    row.get::<_, Option<i64>>(0)?,
                    row.get::<_, i64>(1)?.max(0) as u64,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<i64>>(3)?
                        .map(|value| value.max(0) as u64),
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<i64>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<i64>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, Option<String>>(9)?,
                ))
            },
        )
        .optional()?;
    let Some((
        Some(generation),
        namespace_revision,
        profile_id,
        generation_revision,
        state,
        generation_indexed_rows,
        model_id,
        dimensions,
        normalization,
        profile_version,
    )) = active
    else {
        return Ok(SemanticCandidateSet {
            scores: Vec::new(),
            profile_id: profile.profile_id,
            generation: None,
            degradation: Some("embedding_projection_missing".to_string()),
            backend: "none",
            rows_scored: 0,
        });
    };
    let generation_u64 = generation.max(0) as u64;
    let compatible = profile_id.as_deref() == Some(profile.profile_id.as_str())
        && generation_revision == Some(namespace_revision)
        && state.as_deref() == Some("active")
        && model_id.as_deref() == Some(profile.model_id.as_str())
        && dimensions == Some(profile.dimensions as i64)
        && normalization.as_deref() == Some(profile.normalization.as_str())
        && profile_version.as_deref() == Some(profile.version.as_str());
    if !compatible {
        return Ok(SemanticCandidateSet {
            scores: Vec::new(),
            profile_id: profile.profile_id,
            generation: Some(generation_u64),
            degradation: Some("embedding_projection_stale".to_string()),
            backend: "none",
            rows_scored: 0,
        });
    }
    let unfiltered = filters.path_prefix.is_none()
        && filters.file_type.is_none()
        && filters.time_from.is_none()
        && filters.time_to.is_none();
    let generation_rows = nonnegative_usize(generation_indexed_rows.unwrap_or(0));
    let exact_rows = if unfiltered && generation_rows > exact_vector_limit.max(1) {
        None
    } else {
        let rows = load_filtered_vector_rows(
            db,
            owner,
            namespace,
            generation,
            &profile.profile_id,
            profile.dimensions,
            filters,
            (!unfiltered).then_some(exact_vector_limit.max(1) + 1),
        )?;
        (rows.len() <= exact_vector_limit.max(1)).then_some(rows)
    };
    let (backend, rows) = if let Some(rows) = exact_rows {
        ("exact", rows)
    } else {
        if !ann_projection_available(
            db,
            owner,
            namespace,
            generation,
            &profile.profile_id,
            generation_rows.max(1),
        )? {
            return Ok(SemanticCandidateSet {
                scores: Vec::new(),
                profile_id: profile.profile_id,
                generation: Some(generation_u64),
                degradation: Some("ann_projection_stale".to_string()),
                backend: "ann_projection",
                rows_scored: 0,
            });
        }
        let path_pattern = filters.path_prefix.map(path_descendant_pattern);
        let buckets = claw_core::retrieval_embedding::vector_ann_buckets(query_vector);
        let mut stmt = db.prepare(
            "WITH ann_matches(chunk_id) AS (
                 SELECT chunk_id FROM kb_vector_ann_projection INDEXED BY idx_kb_vector_ann_bucket
                  WHERE owner_user_key = ?1 AND namespace = ?2 AND generation = ?3
                    AND profile_id = ?4 AND projection_version = ?6 AND band = 0 AND bucket = ?12
                 UNION ALL
                 SELECT chunk_id FROM kb_vector_ann_projection INDEXED BY idx_kb_vector_ann_bucket
                  WHERE owner_user_key = ?1 AND namespace = ?2 AND generation = ?3
                    AND profile_id = ?4 AND projection_version = ?6 AND band = 1 AND bucket = ?13
                 UNION ALL
                 SELECT chunk_id FROM kb_vector_ann_projection INDEXED BY idx_kb_vector_ann_bucket
                  WHERE owner_user_key = ?1 AND namespace = ?2 AND generation = ?3
                    AND profile_id = ?4 AND projection_version = ?6 AND band = 2 AND bucket = ?14
                 UNION ALL
                 SELECT chunk_id FROM kb_vector_ann_projection INDEXED BY idx_kb_vector_ann_bucket
                  WHERE owner_user_key = ?1 AND namespace = ?2 AND generation = ?3
                    AND profile_id = ?4 AND projection_version = ?6 AND band = 3 AND bucket = ?15
                 UNION ALL
                 SELECT chunk_id FROM kb_vector_ann_projection INDEXED BY idx_kb_vector_ann_bucket
                  WHERE owner_user_key = ?1 AND namespace = ?2 AND generation = ?3
                    AND profile_id = ?4 AND projection_version = ?6 AND band = 4 AND bucket = ?16
                 UNION ALL
                 SELECT chunk_id FROM kb_vector_ann_projection INDEXED BY idx_kb_vector_ann_bucket
                  WHERE owner_user_key = ?1 AND namespace = ?2 AND generation = ?3
                    AND profile_id = ?4 AND projection_version = ?6 AND band = 5 AND bucket = ?17
                 UNION ALL
                 SELECT chunk_id FROM kb_vector_ann_projection INDEXED BY idx_kb_vector_ann_bucket
                  WHERE owner_user_key = ?1 AND namespace = ?2 AND generation = ?3
                    AND profile_id = ?4 AND projection_version = ?6 AND band = 6 AND bucket = ?18
                 UNION ALL
                 SELECT chunk_id FROM kb_vector_ann_projection INDEXED BY idx_kb_vector_ann_bucket
                  WHERE owner_user_key = ?1 AND namespace = ?2 AND generation = ?3
                    AND profile_id = ?4 AND projection_version = ?6 AND band = 7 AND bucket = ?19
             )
             SELECT c.chunk_id, v.vector_json
             FROM ann_matches a
             CROSS JOIN kb_chunks c
               ON c.owner_user_key = ?1
              AND c.namespace = ?2
              AND c.chunk_id = a.chunk_id
             CROSS JOIN kb_vector_projection v
               ON v.owner_user_key = ?1
              AND v.namespace = ?2
              AND v.generation = ?3
              AND v.chunk_id = a.chunk_id
             WHERE v.profile_id = ?4 AND v.dimensions = ?5
               AND (?7 IS NULL OR c.document_path = ?7 OR c.document_path LIKE ?8 ESCAPE '\\')
               AND (?9 IS NULL OR c.file_type = ?9)
               AND (?10 IS NULL OR c.mtime_epoch >= ?10)
               AND (?11 IS NULL OR c.mtime_epoch <= ?11)
             GROUP BY c.chunk_id, v.vector_json
             ORDER BY COUNT(*) DESC, c.chunk_id
             LIMIT ?20",
        )?;
        let candidate_limit = limit.max(256).min(ann_max_candidates.max(1));
        let rows = stmt
            .query_map(
                params![
                    owner,
                    namespace,
                    generation,
                    profile.profile_id,
                    profile.dimensions as i64,
                    claw_core::retrieval_embedding::VECTOR_ANN_PROJECTION_VERSION,
                    filters.path_prefix,
                    path_pattern,
                    filters.file_type,
                    filters.time_from,
                    filters.time_to,
                    buckets[0] as i64,
                    buckets[1] as i64,
                    buckets[2] as i64,
                    buckets[3] as i64,
                    buckets[4] as i64,
                    buckets[5] as i64,
                    buckets[6] as i64,
                    buckets[7] as i64,
                    candidate_limit as i64,
                ],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        ("ann_projection", rows)
    };
    let rows_scored = rows.len();
    let mut scored = rows
        .into_iter()
        .filter_map(|(chunk_id, vector_json)| {
            let vector = serde_json::from_str::<Vec<f32>>(&vector_json).ok()?;
            let score =
                claw_core::retrieval_embedding::cosine_similarity(query_vector, &vector) as f64;
            (score >= 0.35).then_some((chunk_id, score))
        })
        .collect::<Vec<_>>();
    scored.sort_by(|left, right| {
        right
            .1
            .partial_cmp(&left.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.0.cmp(&right.0))
    });
    scored.truncate(limit.max(1));
    Ok(SemanticCandidateSet {
        scores: scored,
        profile_id: profile.profile_id,
        generation: Some(generation_u64),
        degradation: None,
        backend,
        rows_scored,
    })
}

fn path_descendant_pattern(prefix: &str) -> String {
    let escaped = prefix
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    format!("{escaped}/%")
}

fn vector_filter_sql(filters: SearchFilters<'_>) -> (String, Vec<SqlValue>) {
    let mut clauses = String::new();
    let mut values = Vec::new();
    if let Some(prefix) = filters.path_prefix {
        clauses.push_str(
            " AND (c.document_path = ? OR (c.document_path >= ? AND c.document_path < ?))",
        );
        values.push(SqlValue::Text(prefix.to_string()));
        values.push(SqlValue::Text(format!("{prefix}/")));
        values.push(SqlValue::Text(format!("{prefix}0")));
    }
    if let Some(file_type) = filters.file_type {
        clauses.push_str(" AND c.file_type = ?");
        values.push(SqlValue::Text(file_type.to_string()));
    }
    if let Some(time_from) = filters.time_from {
        clauses.push_str(" AND c.mtime_epoch >= ?");
        values.push(SqlValue::Integer(time_from));
    }
    if let Some(time_to) = filters.time_to {
        clauses.push_str(" AND c.mtime_epoch <= ?");
        values.push(SqlValue::Integer(time_to));
    }
    (clauses, values)
}

fn vector_query_params(
    owner: &str,
    namespace: &str,
    generation: i64,
    profile_id: &str,
    dimensions: usize,
    filters: SearchFilters<'_>,
) -> (String, Vec<SqlValue>) {
    let (clauses, filter_values) = vector_filter_sql(filters);
    let mut values = vec![
        SqlValue::Integer(generation),
        SqlValue::Text(owner.to_string()),
        SqlValue::Text(namespace.to_string()),
        SqlValue::Text(profile_id.to_string()),
        SqlValue::Integer(dimensions as i64),
    ];
    values.extend(filter_values);
    (clauses, values)
}

fn load_filtered_vector_rows(
    db: &Connection,
    owner: &str,
    namespace: &str,
    generation: i64,
    profile_id: &str,
    dimensions: usize,
    filters: SearchFilters<'_>,
    max_rows: Option<usize>,
) -> Result<Vec<(String, String)>> {
    let (clauses, mut values) = vector_query_params(
        owner, namespace, generation, profile_id, dimensions, filters,
    );
    let chunk_index = if filters.path_prefix.is_some() {
        "idx_kb_chunks_owner_namespace_document"
    } else {
        "idx_kb_chunks_owner_namespace_filter"
    };
    let limit_clause = if let Some(max_rows) = max_rows {
        values.push(SqlValue::Integer(max_rows as i64));
        " LIMIT ?"
    } else {
        ""
    };
    let sql = format!(
        "SELECT c.chunk_id, v.vector_json
         FROM kb_chunks c INDEXED BY {chunk_index}
         CROSS JOIN kb_vector_projection v
           ON v.owner_user_key = c.owner_user_key
          AND v.namespace = c.namespace
          AND v.generation = ?
          AND v.chunk_id = c.chunk_id
         WHERE c.owner_user_key = ? AND c.namespace = ?
           AND v.profile_id = ? AND v.dimensions = ?{clauses}
         ORDER BY c.chunk_id{limit_clause}"
    );
    let mut stmt = db.prepare(&sql)?;
    let rows = stmt
        .query_map(params_from_iter(values), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn load_chunks_by_ids(
    db: &Connection,
    owner: &str,
    namespace: &str,
    chunk_ids: &[String],
) -> Result<Vec<Chunk>> {
    if chunk_ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = std::iter::repeat_n("?", chunk_ids.len())
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT chunk_id, document_path, file_type, ordinal, text,
                text_sha256, len_tokens, mtime_epoch, content_kind, location_json
         FROM kb_chunks
         WHERE owner_user_key = ? AND namespace = ? AND chunk_id IN ({placeholders})"
    );
    let mut values = Vec::with_capacity(chunk_ids.len() + 2);
    values.push(SqlValue::Text(owner.to_string()));
    values.push(SqlValue::Text(namespace.to_string()));
    values.extend(chunk_ids.iter().cloned().map(SqlValue::Text));
    let mut stmt = db.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(values), |row| {
        Ok(Chunk {
            chunk_id: row.get(0)?,
            path: row.get(1)?,
            file_type: row.get(2)?,
            offset: nonnegative_usize(row.get::<_, i64>(3)?),
            text: row.get(4)?,
            text_sha256: row.get(5)?,
            len_tokens: nonnegative_usize(row.get::<_, i64>(6)?),
            mtime_epoch: row.get(7)?,
            content_kind: row.get(8)?,
            location: serde_json::from_str(&row.get::<_, String>(9)?).unwrap_or_default(),
        })
    })?;
    let mut by_id = rows
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .map(|chunk| (chunk.chunk_id.clone(), chunk))
        .collect::<HashMap<_, _>>();
    Ok(chunk_ids
        .iter()
        .filter_map(|chunk_id| by_id.remove(chunk_id))
        .collect())
}
