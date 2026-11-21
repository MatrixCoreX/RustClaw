use super::*;

#[tokio::test]
async fn task_cancellation_wakes_a_stalled_provider_wait_and_releases_its_permit() {
    let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(1));
    let token = CancellationToken::new();
    let cancelled = token.clone();
    let resource = semaphore.clone();
    let running = tokio::spawn(async move {
        let _permit = resource.acquire_owned().await.unwrap();
        let conversation = CancellationToken::new();
        tokio::select! {
            value = wait(Some(cancelled), &conversation) => value,
            _ = std::future::pending::<()>() => unreachable!(),
        }
    });
    tokio::task::yield_now().await;
    token.cancel();
    let reason = tokio::time::timeout(std::time::Duration::from_secs(1), running)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reason, ModelInterrupt::TaskCancelled);
    assert_eq!(reason.cost_status(), "cancelled");
    assert_eq!(semaphore.available_permits(), 1);
}

#[tokio::test]
async fn conversation_input_keeps_its_own_reason_and_task_cancellation_wins_a_tie() {
    let task = CancellationToken::new();
    let conversation = CancellationToken::new();
    conversation.cancel();
    assert_eq!(
        wait(Some(task.clone()), &conversation).await,
        ModelInterrupt::ConversationInput
    );
    task.cancel();
    assert_eq!(
        wait(Some(task), &conversation).await,
        ModelInterrupt::TaskCancelled
    );
}
