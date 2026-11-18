//! Invocation-local machine error metadata, without prompt, response, or credentials.
use claw_core::provider_failure_policy::ProviderFailureClass;
use std::{cell::RefCell, future::Future};

tokio::task_local! {
    static FAILURE: RefCell<Option<(ProviderFailureClass, bool)>>;
}

pub(crate) fn record(class: ProviderFailureClass, retryable: bool) {
    let _ = FAILURE.try_with(|slot| *slot.borrow_mut() = Some((class, retryable)));
}

pub(crate) fn clear() {
    let _ = FAILURE.try_with(|slot| *slot.borrow_mut() = None);
}

pub(crate) fn current() -> Option<(ProviderFailureClass, bool)> {
    FAILURE.try_with(|slot| *slot.borrow()).ok().flatten()
}

pub(crate) async fn capture<F: Future>(
    future: F,
) -> (F::Output, Option<(ProviderFailureClass, bool)>) {
    FAILURE
        .scope(RefCell::new(None), async move { (future.await, current()) })
        .await
}
