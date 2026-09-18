use crate::{wallet::worker::platform::Worker, Result};
use std::{
    fs::File,
    os::fd::{AsRawFd, FromRawFd},
};
pub struct Child;
impl Child {
    pub fn kill(&mut self) -> Result<()> {
        super::bridge::stop_worker()
    }
    pub fn wait(&mut self) -> Result<()> {
        Ok(())
    }
}
pub fn spawn() -> Result<Worker> {
    let fd = super::bridge::start_worker().map_err(|_| "wallet_worker_unavailable")?;
    if fd < 0 {
        return Err("wallet_worker_unavailable".into());
    }
    // SAFETY: a fresh connected socket is detached by the private Binder adapter.
    let input = unsafe { File::from_raw_fd(fd) };
    crate::wallet::worker::transport::nonblocking(input.as_raw_fd())?;
    let output = input.try_clone().map_err(|_| "wallet_worker_unavailable")?;
    Ok(Worker {
        child: Child,
        input,
        output,
    })
}
