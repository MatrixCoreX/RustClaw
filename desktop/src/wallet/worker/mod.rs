mod client;
mod platform;
mod protocol;
mod sandbox;
#[cfg(all(test, target_os = "linux"))]
mod tests;
pub(super) mod transport;
#[cfg(windows)]
mod windows_job;
#[cfg(windows)]
pub(super) mod windows_pipe;
use super::Vault;
use crate::Result;
pub use client::VaultClient;
use protocol::{Input, Output, Request};
use std::time::{Duration, SystemTime};

/// Internal process mode, entered before Tauri or any WebView is initialized.
pub fn run() -> Result<()> {
    {
        let protection = sandbox::enter();
        let (mut input, mut output) = platform::stdio()?;
        transport::nonblocking(transport::raw(&input))?;
        transport::nonblocking(transport::raw(&output))?;
        let first: Input =
            serde_json::from_slice(&transport::read(&mut input, Duration::from_secs(30))?)
                .map_err(|_| "wallet_worker_protocol_invalid")?;
        let mut vault = None;
        let initial = (|| {
            if first.version != 1 || first.id != 1 {
                return Err("wallet_worker_protocol_invalid".into());
            }
            let Request::Open { directory } = &first.request else {
                return Err("wallet_worker_protocol_invalid".into());
            };
            protection?;
            // Probe mandatory locked memory before reading any vault credentials.
            let _probe = super::secure_memory::LockedKey::zeroed()?;
            vault = Some(Vault::new(directory.clone())?);
            Ok(serde_json::Value::Null)
        })();
        let failed = initial.is_err();
        send(&mut output, first.id, initial)?;
        if failed {
            return Ok(());
        }
        let mut vault = vault.unwrap();
        let mut last_id = first.id;
        let mut wall = SystemTime::now();
        loop {
            if wall.elapsed().unwrap_or(Duration::MAX) > Duration::from_secs(5) {
                vault.lock();
            }
            wall = SystemTime::now();
            vault.expire();
            if !transport::ready(transport::raw(&input), false, Duration::from_millis(500))? {
                continue;
            }
            let bytes = transport::read(&mut input, Duration::from_secs(30))?;
            let request: Input =
                serde_json::from_slice(&bytes).map_err(|_| "wallet_worker_protocol_invalid")?;
            if request.version != 1
                || request.id
                    != last_id
                        .checked_add(1)
                        .ok_or("wallet_worker_protocol_invalid")?
            {
                return Err("wallet_worker_protocol_invalid".into());
            }
            last_id = request.id;
            let result = dispatch(&mut vault, &request.request);
            send(&mut output, request.id, result)?;
            wall = SystemTime::now();
        }
    }
}
fn send(output: &mut std::fs::File, id: u64, result: Result<serde_json::Value>) -> Result<()> {
    let bytes = serde_json::to_vec(&Output {
        version: 1,
        id,
        result,
    })
    .map_err(|_| "wallet_worker_protocol_invalid")?;
    transport::write(output, &bytes, Duration::from_secs(30))
}
fn dispatch(vault: &mut Vault, request: &Request) -> Result<serde_json::Value> {
    fn json(value: impl serde::Serialize) -> Result<serde_json::Value> {
        serde_json::to_value(value).map_err(|_| "wallet_worker_protocol_invalid".into())
    }
    match request {
        Request::Open { .. } => Err("wallet_worker_protocol_invalid".into()),
        Request::Status {} => json(vault.status()),
        Request::Lock {} => {
            vault.lock();
            json(())
        }
        Request::Account { id } => json(vault.account(*id)?),
        Request::Initialize { password } => {
            vault.initialize(password)?;
            json(())
        }
        Request::Unlock { password } => {
            vault.unlock(password)?;
            json(())
        }
        Request::Create { name } => json(vault.create(name)?),
        Request::Backup {
            id,
            vault_password,
            password,
            path,
        } => {
            vault.backup(*id, vault_password, password, path)?;
            json(())
        }
        Request::Restore {
            password,
            path,
            name,
        } => json(vault.restore(password, path, name)?),
        Request::Sign {
            id,
            password,
            payload,
            cap,
            intent,
        } => {
            use crate::asset_operations::protocol::{validate_challenge, Payload};
            if !intent.write() {
                return Err("wallet_intent_invalid".into());
            }
            let account = vault.account(*id)?;
            cap.validate(cap.service, intent.action())?;
            intent.validate(cap.service, &account.public_key)?;
            let parsed: Payload =
                serde_json::from_str(payload).map_err(|_| "wallet_challenge_invalid")?;
            let now = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map_err(|_| "wallet_challenge_invalid")?
                .as_secs() as i64;
            validate_challenge(
                payload,
                cap,
                &account.public_key,
                parsed.operation_id,
                intent,
                now,
            )?;
            json(vault.sign_with_password(*id, payload.as_bytes(), password)?)
        }
    }
}
