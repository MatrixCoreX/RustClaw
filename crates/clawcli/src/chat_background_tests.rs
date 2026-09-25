use super::*;

#[test]
fn duplicate_task_followers_are_coalesced() {
    let (output_tx, _output_rx) = output_channel();
    let mut followers = ChatBackgroundFollowers::new(output_tx);
    followers.active_task_ids.insert("task-1".to_string());
    followers.start("http://127.0.0.1:1", "key", "task-1", 0);
    assert_eq!(followers.active_task_ids.len(), 1);
}

#[test]
fn completed_followers_are_reaped_without_touching_other_tasks() {
    let (output_tx, _output_rx) = output_channel();
    let mut followers = ChatBackgroundFollowers::new(output_tx);
    followers.active_task_ids.insert("task-1".to_string());
    followers.active_task_ids.insert("task-2".to_string());
    followers
        .completion_tx
        .send(("task-1".to_string(), 9))
        .expect("completion");
    assert_eq!(followers.reap(), vec![("task-1".to_string(), 9)]);
    assert!(!followers.active_task_ids.contains("task-1"));
    assert!(followers.active_task_ids.contains("task-2"));
}
