use super::{
    protocol::{Input, Output, Request},
    transport,
};
use crate::{
    asset_operations::protocol::{Capabilities, Intent},
    wallet::{Account, Status},
    Result,
};
use serde::de::DeserializeOwned;
use std::{
    fs::File,
    path::{Path, PathBuf},
    process::Child,
    time::Duration,
};
use uuid::Uuid;
use zeroize::Zeroizing;

pub struct VaultClient {
    worker: Option<Child>,
    input: Option<File>,
    output: Option<File>,
    #[cfg(windows)]
    guard: Option<super::windows_job::Job>,
    next: u64,
    failure: Option<String>,
    was_unlocked: bool,
    revoked: bool,
}
impl VaultClient {
    pub fn new(directory: PathBuf) -> Result<Self> {
        let mut client = Self {
            worker: None,
            input: None,
            output: None,
            #[cfg(windows)]
            guard: None,
            next: 0,
            failure: None,
            was_unlocked: false,
            revoked: false,
        };
        if let Err(error) = client.start(directory) {
            client.stop();
            client.failure = Some(error);
        }
        // Other desktop/device functionality stays available when the OS cannot
        // provide this protection. Wallet requests return the precise failure.
        Ok(client)
    }
    fn start(&mut self, directory: PathBuf) -> Result<()> {
        let exe = std::env::current_exe().map_err(|_| "wallet_worker_unavailable")?;
        self.start_executable(directory, &exe)
    }
    fn start_executable(&mut self, directory: PathBuf, exe: &Path) -> Result<()> {
        let worker = super::platform::spawn(exe)?;
        self.input = Some(worker.input);
        self.output = Some(worker.output);
        self.worker = Some(worker.child);
        #[cfg(windows)]
        {
            self.guard = Some(worker.guard);
        }
        self.call::<()>(Request::Open { directory })
    }
    fn stop(&mut self) {
        self.input = None;
        self.output = None;
        if let Some(mut child) = self.worker.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        #[cfg(windows)]
        {
            self.guard = None;
        }
        self.was_unlocked = false;
        self.revoked = true;
    }
    fn call<T: DeserializeOwned>(&mut self, request: Request) -> Result<T> {
        if let Some(error) = &self.failure {
            return Err(error.clone());
        }
        let response = self.exchange(request);
        let response = match response {
            Ok(r) => r,
            Err(error) => {
                self.stop();
                self.failure = Some(error.clone());
                return Err(error);
            }
        };
        match serde_json::from_value(response.result?) {
            Ok(value) => Ok(value),
            Err(_) => {
                self.stop();
                self.failure = Some("wallet_worker_protocol_invalid".into());
                Err("wallet_worker_protocol_invalid".into())
            }
        }
    }
    fn exchange(&mut self, request: Request) -> Result<Output> {
        self.next = self
            .next
            .checked_add(1)
            .ok_or("wallet_worker_protocol_invalid")?;
        let bytes = Zeroizing::new(
            serde_json::to_vec(&Input {
                version: 1,
                id: self.next,
                request,
            })
            .map_err(|_| "wallet_worker_protocol_invalid")?,
        );
        transport::write(
            self.input.as_mut().ok_or("wallet_worker_unavailable")?,
            &bytes,
            Duration::from_secs(30),
        )?;
        let response: Output = serde_json::from_slice(&transport::read(
            self.output.as_mut().ok_or("wallet_worker_unavailable")?,
            Duration::from_secs(60),
        )?)
        .map_err(|_| "wallet_worker_protocol_invalid")?;
        if response.version != 1 || response.id != self.next {
            return Err("wallet_worker_protocol_invalid".into());
        }
        Ok(response)
    }
    pub fn status(&mut self) -> Result<Status> {
        let status: Status = self.call(Request::Status {})?;
        self.observe_lock(status.unlocked);
        Ok(status)
    }
    fn observe_lock(&mut self, unlocked: bool) {
        // UI status polling must not consume the expiration event before the
        // lifecycle task can revoke pending confirmations.
        self.revoked |= self.was_unlocked && !unlocked;
        self.was_unlocked = unlocked;
    }
    pub fn expire(&mut self) -> bool {
        match self.status() {
            Ok(_) => std::mem::take(&mut self.revoked),
            Err(_) => {
                self.stop();
                true
            }
        }
    }
    pub fn lock(&mut self) {
        if self.call::<()>(Request::Lock {}).is_err() {
            self.stop();
        }
        self.was_unlocked = false;
        self.revoked = false;
    }
    pub fn account(&mut self, id: Uuid) -> Result<Account> {
        self.call(Request::Account { id })
    }
    pub fn initialize(&mut self, password: &str) -> Result<()> {
        let result = self.call(Request::Initialize {
            password: password.into(),
        });
        self.was_unlocked = result.is_ok();
        result
    }
    pub fn unlock(&mut self, password: &str) -> Result<()> {
        let result = self.call(Request::Unlock {
            password: password.into(),
        });
        self.was_unlocked = result.is_ok();
        result
    }
    pub fn create(&mut self, name: &str) -> Result<Account> {
        self.call(Request::Create { name: name.into() })
    }
    pub fn backup(
        &mut self,
        id: Uuid,
        vault_password: &str,
        password: &str,
        path: &Path,
    ) -> Result<()> {
        let result = self.call(Request::Backup {
            id,
            vault_password: vault_password.into(),
            password: password.into(),
            path: path.into(),
        });
        self.was_unlocked = false;
        result
    }
    pub fn restore(&mut self, password: &str, path: &Path, name: &str) -> Result<Account> {
        self.call(Request::Restore {
            password: password.into(),
            path: path.into(),
            name: name.into(),
        })
    }
    pub fn sign_with_password(
        &mut self,
        id: Uuid,
        bytes: &[u8],
        password: &str,
        cap: Capabilities,
        intent: Intent,
    ) -> Result<String> {
        let payload = std::str::from_utf8(bytes).map_err(|_| "wallet_challenge_invalid")?;
        if payload.len() > 8192 {
            return Err("wallet_challenge_invalid".into());
        }
        let result = self.call(Request::Sign {
            id,
            password: password.into(),
            payload: payload.into(),
            cap,
            intent,
        });
        // Signing deliberately locks on both success and password failure. It
        // must not be mistaken for idle expiration and cancel a valid retry.
        self.was_unlocked = false;
        result
    }
}
#[cfg(test)]
#[path = "native_tests.rs"]
mod native_tests;
#[cfg(test)]
#[path = "client_tests.rs"]
mod tests;
impl Drop for VaultClient {
    fn drop(&mut self) {
        self.stop();
    }
}
