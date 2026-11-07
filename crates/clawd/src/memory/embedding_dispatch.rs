use std::{collections::HashMap, future::Future, pin::Pin};

use super::{commit, EmbeddingJob};
use crate::memory::{
    embedding::{EmbeddingProviderError, EmbeddingRequestItem, MemoryEmbeddingProvider},
    vector_store::MemoryEmbeddingProfile,
};

pub(super) enum Failure {
    BeforeDispatch(EmbeddingProviderError),
    Provider(EmbeddingProviderError, Vec<EmbeddingJob>),
    AfterDispatch(anyhow::Error),
}

impl From<EmbeddingProviderError> for Failure {
    fn from(error: EmbeddingProviderError) -> Self {
        Self::BeforeDispatch(error)
    }
}

/// Persist successful split responses before issuing another sub-request. A
/// later network failure cannot cause successful earlier pieces to be replayed.
pub(super) fn execute<'a>(
    state: &'a crate::AppState,
    worker: &'a str,
    profile: &'a MemoryEmbeddingProfile,
    provider: &'a dyn MemoryEmbeddingProvider,
    jobs: &'a [EmbeddingJob],
    requests: &'a [EmbeddingRequestItem],
) -> Pin<Box<dyn Future<Output = Result<usize, Failure>> + Send + 'a>> {
    Box::pin(async move {
        let bytes = requests
            .iter()
            .map(|item| item.request_item_id.len().saturating_add(item.text.len()))
            .fold(0usize, usize::saturating_add);
        if bytes > state.policy.memory.embedding_max_request_bytes.max(1) && jobs.len() > 1 {
            return split(state, worker, profile, provider, jobs, requests).await;
        }
        let mut selected_jobs = Vec::new();
        let mut selected_requests = Vec::new();
        for (job, request) in jobs.iter().zip(requests) {
            if commit::prepare_dispatch(state, worker, profile, job)
                .map_err(Failure::AfterDispatch)?
            {
                selected_jobs.push(job.clone());
                selected_requests.push(request.clone());
            }
        }
        if selected_jobs.is_empty() {
            return Ok(0);
        }
        let responses = match provider.embed_batch(&selected_requests).await {
            Err(error) if error.status_code == Some(413) && selected_jobs.len() > 1 => {
                return split(
                    state,
                    worker,
                    profile,
                    provider,
                    &selected_jobs,
                    &selected_requests,
                )
                .await;
            }
            Err(error) => return Err(Failure::Provider(error, selected_jobs)),
            Ok(responses) => responses,
        };
        let count = responses.len();
        let responses: HashMap<_, _> = responses
            .into_iter()
            .map(|item| (item.request_item_id.clone(), item))
            .collect();
        if count != selected_jobs.len()
            || responses.len() != count
            || selected_jobs
                .iter()
                .any(|job| !responses.contains_key(&job.request_item_id))
        {
            return Err(Failure::AfterDispatch(anyhow::anyhow!(
                "memory_embedding_response_item_mismatch"
            )));
        }
        let mut completed = 0;
        let mut first_error = None;
        for job in &selected_jobs {
            let result = commit::receive(
                state,
                worker,
                profile,
                job,
                &responses[&job.request_item_id].vector,
            )
            .and_then(|()| commit::publish(state, worker, job));
            match result {
                Ok(count) => completed += count,
                Err(error) if first_error.is_none() => first_error = Some(error),
                Err(_) => {}
            }
        }
        match first_error {
            Some(error) => Err(Failure::AfterDispatch(error)),
            None => Ok(completed),
        }
    })
}

async fn split(
    state: &crate::AppState,
    worker: &str,
    profile: &MemoryEmbeddingProfile,
    provider: &dyn MemoryEmbeddingProvider,
    jobs: &[EmbeddingJob],
    requests: &[EmbeddingRequestItem],
) -> Result<usize, Failure> {
    let middle = jobs.len() / 2;
    let left = execute(
        state,
        worker,
        profile,
        provider,
        &jobs[..middle],
        &requests[..middle],
    )
    .await;
    if matches!(
        left,
        Err(Failure::Provider(..) | Failure::BeforeDispatch(_))
    ) {
        return left;
    }
    let right = execute(
        state,
        worker,
        profile,
        provider,
        &jobs[middle..],
        &requests[middle..],
    )
    .await;
    match (left, right) {
        (Ok(left), Ok(right)) => Ok(left + right),
        (Err(Failure::AfterDispatch(local)), Err(Failure::Provider(error, failed))) => {
            super::fail_jobs(state, worker, &failed, &error, true)
                .map_err(Failure::AfterDispatch)?;
            Err(Failure::AfterDispatch(local))
        }
        (Err(error), _) | (_, Err(error)) => Err(error),
    }
}
