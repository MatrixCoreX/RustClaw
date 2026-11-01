use super::*;

#[test]
fn caller_loss_before_submission_refuses_the_only_permit() {
    let (caller, permit) = HostSubmissionGuard::pair();
    assert!(permit.is_open());
    drop(caller);
    assert!(!permit.is_open());
    assert!(!permit.enter());
}

#[test]
fn submission_before_caller_loss_cannot_be_revoked() {
    let (caller, permit) = HostSubmissionGuard::pair();
    assert!(permit.enter());
    assert_eq!(caller.0.load(Ordering::Acquire), SUBMITTING);
    let observation = Arc::clone(&caller.0);
    drop(caller);
    assert_eq!(observation.load(Ordering::Acquire), SUBMITTING);
}

#[test]
fn independent_submissions_do_not_cancel_one_another() {
    let (first, first_permit) = HostSubmissionGuard::pair();
    let (second, second_permit) = HostSubmissionGuard::pair();
    drop(first);
    assert!(!first_permit.enter());
    assert!(second_permit.enter());
    drop(second);
}

#[test]
fn cancellation_and_submission_have_one_linear_winner() {
    for _ in 0..64 {
        let (caller, permit) = HostSubmissionGuard::pair();
        let state = Arc::clone(&caller.0);
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let other = Arc::clone(&barrier);
        let submit = std::thread::spawn(move || {
            other.wait();
            permit.enter()
        });
        barrier.wait();
        drop(caller);
        let submitted = submit.join().unwrap();
        assert_eq!(
            state.load(Ordering::Acquire),
            if submitted { SUBMITTING } else { ABANDONED }
        );
    }
}
