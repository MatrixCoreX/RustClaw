use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use anyhow::Result;

static INTERRUPTED: AtomicBool = AtomicBool::new(false);
static HANDLER: OnceLock<Result<(), String>> = OnceLock::new();

pub(crate) fn install() -> Result<()> {
    reset();
    let result = HANDLER.get_or_init(|| {
        ctrlc::set_handler(|| {
            INTERRUPTED.store(true, Ordering::SeqCst);
        })
        .map_err(|error| error.to_string())
    });
    match result {
        Ok(()) => Ok(()),
        Err(error) => anyhow::bail!("interrupt_handler_install_failed:{error}"),
    }
}

pub(crate) fn requested() -> bool {
    INTERRUPTED.load(Ordering::SeqCst)
}

pub(crate) fn reset() {
    INTERRUPTED.store(false, Ordering::SeqCst);
}
