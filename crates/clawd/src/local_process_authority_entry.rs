use std::fs::File;
use std::time::{Duration, Instant};

use super::entry::{helper_selected_for_flag, take_inherited_channel, EntryFailure};
use super::owner_privacy::OwnerProcessPrivacy;

#[path = "local_process_authority_host.rs"]
pub(crate) mod host;
#[path = "local_process_authority_journal.rs"]
mod journal;
#[path = "local_process_authority_protocol.rs"]
mod protocol;

#[path = "local_process_authority_inventory.rs"]
mod inventory;

#[path = "local_process_authority_inventory_entry.rs"]
mod inventory_entry;

const HELPER_FLAG: &str = "--process-authority";
const BOOTSTRAP_READY: &[u8] = b"authority_bootstrap_ready_v1";

pub(super) fn run_if_requested() -> Option<Result<(), EntryFailure>> {
    match helper_selected_for_flag(std::env::args_os().skip(1), HELPER_FLAG) {
        Ok(false) => None,
        Err(error) => Some(Err(error)),
        Ok(true) => Some(unsafe { run_exclusive_helper() }),
    }
}

unsafe fn run_exclusive_helper() -> Result<(), EntryFailure> {
    let privacy = unsafe { OwnerProcessPrivacy::protect() }.map_err(EntryFailure::Privacy)?;
    let deadline = Instant::now() + Duration::from_secs(120);
    let channel = unsafe { take_inherited_channel()? };
    channel
        .send_control(BOOTSTRAP_READY, deadline)
        .map_err(EntryFailure::Transport)?;
    let (bytes, [keys, journal], channel) = channel
        .receive_bootstrap(deadline)
        .map_err(EntryFailure::Transport)?;
    privacy.validate().map_err(EntryFailure::Privacy)?;
    let request = match protocol::AuthorityBootstrap::decode(&bytes) {
        Ok(request) => request,
        Err(error) => {
            privacy.validate().map_err(EntryFailure::Privacy)?;
            let response = protocol::AuthorityResponse::from_result(Err(error));
            let bytes =
                serde_json::to_vec(&response).map_err(|_| EntryFailure::InvalidBootstrap)?;
            channel
                .send_control(&bytes, deadline)
                .map_err(EntryFailure::Transport)?;
            return Err(EntryFailure::AuthorityAdmissionDenied);
        }
    };
    if matches!(
        request.action,
        protocol::AuthorityAction::InventoryResources { .. }
    ) {
        return unsafe {
            inventory_entry::run(
                privacy,
                channel,
                File::from(keys),
                File::from(journal),
                request.action,
                deadline,
            )
        };
    }
    let result = unsafe { journal::admit(File::from(keys), File::from(journal), &request.action) };
    privacy.validate().map_err(EntryFailure::Privacy)?;
    let response =
        result.unwrap_or_else(|error| protocol::AuthorityResponse::from_result(Err(error)));
    let bytes = serde_json::to_vec(&response).map_err(|_| EntryFailure::InvalidBootstrap)?;
    channel
        .send_control(&bytes, deadline)
        .map_err(EntryFailure::Transport)?;
    if response.is_success() {
        Ok(())
    } else {
        Err(EntryFailure::AuthorityAdmissionDenied)
    }
}

#[cfg(test)]
#[path = "local_process_authority_entry_tests.rs"]
mod tests;
