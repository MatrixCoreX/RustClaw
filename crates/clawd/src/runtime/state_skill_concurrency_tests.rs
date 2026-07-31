use std::sync::Arc;
use std::time::Duration;

use super::SkillConcurrencyGates;

#[tokio::test]
async fn failed_or_aborted_item_releases_the_next_skill_queue_slot() {
    let gates = Arc::new(SkillConcurrencyGates::default());
    let semaphore = gates.semaphore("serial_media", 1);
    let first_item = semaphore
        .clone()
        .acquire_owned()
        .await
        .expect("first queue item");

    let waiting_semaphore = semaphore.clone();
    let mut second_item = tokio::spawn(async move { waiting_semaphore.acquire_owned().await });
    assert!(
        tokio::time::timeout(Duration::from_millis(25), &mut second_item)
            .await
            .is_err(),
        "second item must wait while the first item owns the slot"
    );

    // Dropping the permit models every exit path from the dispatcher,
    // including structured failure, timeout, cancellation, and task abort.
    drop(first_item);
    let second_permit = tokio::time::timeout(Duration::from_secs(1), second_item)
        .await
        .expect("second item should continue after the first exits")
        .expect("queue task")
        .expect("second queue permit");
    drop(second_permit);
}

#[tokio::test]
async fn one_skill_queue_does_not_serialize_unrelated_skills() {
    let gates = SkillConcurrencyGates::default();
    let media = gates.semaphore("serial_media", 1);
    let unrelated = gates.semaphore("unrelated_skill", 1);
    let _media_permit = media.acquire_owned().await.expect("media permit");

    let unrelated_permit =
        tokio::time::timeout(Duration::from_millis(100), unrelated.acquire_owned())
            .await
            .expect("unrelated skill must not wait behind media")
            .expect("unrelated permit");
    drop(unrelated_permit);
}
