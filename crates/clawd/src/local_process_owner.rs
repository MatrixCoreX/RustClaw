#[path = "local_process_authority_entry.rs"]
mod authority_entry;
#[path = "local_process_owner_entry.rs"]
mod entry;
#[path = "local_process_exit.rs"]
mod exit;
#[path = "local_process_instance.rs"]
pub(crate) mod instance;
#[path = "local_process_launch_channel.rs"]
mod launch_channel;
#[cfg(test)]
#[path = "local_process_members.rs"]
mod members;
#[path = "local_process_owner_admission.rs"]
mod owner_admission;
#[path = "local_process_owner_authority.rs"]
mod owner_authority;
#[cfg(test)]
#[path = "local_process_owner_group.rs"]
mod owner_group;
#[path = "local_process_owner_handshake.rs"]
mod owner_handshake;
#[path = "local_process_owner_host_handshake.rs"]
mod owner_host_handshake;
#[path = "local_process_owner_no_commit.rs"]
mod owner_no_commit;
#[path = "local_process_owner_outcome.rs"]
mod owner_outcome;
#[path = "local_process_owner_privacy.rs"]
mod owner_privacy;
#[path = "local_process_owner_protocol.rs"]
pub(crate) mod owner_protocol;
#[path = "local_process_owner_reference.rs"]
mod owner_reference;
#[path = "local_process_owner_resources.rs"]
mod owner_resources;
#[path = "local_process_owner_runtime.rs"]
mod owner_runtime;
#[path = "local_process_owner_scope.rs"]
mod owner_scope;
#[path = "local_process_owner_session.rs"]
mod owner_session;
#[path = "local_process_owner_start.rs"]
mod owner_start;
#[path = "local_process_owner_store.rs"]
pub(crate) mod owner_store;
#[path = "local_process_owner_terminal.rs"]
mod owner_terminal;
#[path = "local_process_owner_transport.rs"]
mod owner_transport;
#[path = "process_launch.rs"]
pub(crate) mod process_launch;
#[path = "local_process_startup_protocol.rs"]
mod startup_protocol;

pub(crate) use authority_entry::host::DurableOwnerInventory;
pub(crate) use authority_entry::host::HostResourceLease;
pub(crate) use owner_outcome::DurableOwnerOutcome;
pub(crate) use owner_resources::{DurableOwnerResources, OwnerResourceBudget};

pub(crate) fn run_if_requested() -> Option<Result<(), entry::EntryFailure>> {
    authority_entry::run_if_requested().or_else(entry::run_if_requested)
}

#[cfg(test)]
#[path = "local_process_owner_package_test_support.rs"]
pub(crate) mod package_test_support;
