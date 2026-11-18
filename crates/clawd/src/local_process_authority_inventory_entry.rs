use super::inventory::InventoryFrame;
use super::protocol::AuthorityAction;
use super::*;
use sha2::{Digest, Sha256};

pub(super) unsafe fn run(
    privacy: OwnerProcessPrivacy,
    channel: super::super::launch_channel::LaunchChannel,
    keys: File,
    journal: File,
    action: AuthorityAction,
    deadline: Instant,
) -> Result<(), EntryFailure> {
    let AuthorityAction::InventoryResources {
        authority_fingerprint,
        request_id,
    } = &action
    else {
        return Err(EntryFailure::InvalidBootstrap);
    };
    let fingerprint = *authority_fingerprint;
    let request_id = *request_id;
    let begin = InventoryFrame::Begin {
        schema_version: 1,
        fingerprint,
        request_id,
    };
    let bytes = serde_json::to_vec(&begin).map_err(|_| EntryFailure::InvalidBootstrap)?;
    let mut digest = Sha256::new();
    digest.update(&bytes);
    // Begin is not admission or a proof. Only Complete plus successful exit is.
    channel
        .send_control(&bytes, deadline)
        .map_err(EntryFailure::Transport)?;
    let mut index = 0;
    let result = unsafe {
        super::journal::inventory(keys, journal, &action, |reference, budget| {
            let frame = InventoryFrame::Item {
                schema_version: 1,
                fingerprint,
                request_id,
                index,
                credential: reference.credential(),
                binding: reference.record().binding_digest,
                budget: budget.clone(),
            };
            let bytes = serde_json::to_vec(&frame).map_err(|_| {
                super::super::owner_reference::OwnerReferenceFailure::StorageUnavailable
            })?;
            channel.send_control(&bytes, deadline).map_err(|_| {
                super::super::owner_reference::OwnerReferenceFailure::StorageUnavailable
            })?;
            digest.update(&bytes);
            index += 1;
            Ok(())
        })
    };
    privacy.validate().map_err(EntryFailure::Privacy)?;
    let frame = match result {
        Ok(counts) => InventoryFrame::Complete {
            schema_version: 1,
            fingerprint,
            request_id,
            examined: counts.examined,
            pending: counts.pending,
            digest: digest.finalize().into(),
        },
        Err(error) => {
            let bytes = serde_json::to_vec(&InventoryFrame::Error {
                schema_version: 1,
                error,
            })
            .map_err(|_| EntryFailure::InvalidBootstrap)?;
            channel
                .send_control(&bytes, deadline)
                .map_err(EntryFailure::Transport)?;
            return Err(EntryFailure::AuthorityAdmissionDenied);
        }
    };
    let bytes = serde_json::to_vec(&frame).map_err(|_| EntryFailure::InvalidBootstrap)?;
    channel
        .send_control(&bytes, deadline)
        .map_err(EntryFailure::Transport)
}
