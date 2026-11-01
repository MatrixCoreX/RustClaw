use crate::local_process_owner::instance::observe_process;
use crate::local_process_owner::launch_channel::LaunchChannel;
use crate::local_process_owner::owner_handshake::{NoCommitEvidence, OwnerPreparedStart};
use crate::local_process_owner::owner_no_commit::OwnerNoCommitRecord;
use crate::local_process_owner::owner_reference::OwnerAuthorityKey;
use crate::local_process_owner::owner_start::OwnerStart;
use crate::local_process_owner::owner_store::{DurableOwnerReference, OwnerReferenceStore};
use crate::local_process_owner::process_launch::{PayloadIo, ProcessLaunch};
use crate::process_invocation::ProcessInvocation;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::net::UnixStream;
use std::time::Duration;
use uuid::Uuid;

pub(super) fn key() -> OwnerAuthorityKey {
    OwnerAuthorityKey::new([7; 32]).unwrap()
}

pub(super) struct Fixture {
    pub(super) root: tempfile::TempDir,
    pub(super) store: OwnerReferenceStore,
    pub(super) reference: DurableOwnerReference,
    pub(super) evidence: NoCommitEvidence,
}

impl Fixture {
    pub(super) fn new() -> Self {
        let root = tempfile::Builder::new()
            .prefix("nc-")
            .permissions(fs::Permissions::from_mode(0o700))
            .tempdir()
            .unwrap();
        let store =
            OwnerReferenceStore::from_admitted_directory(File::open(root.path()).unwrap(), key())
                .unwrap();
        let credential = store
            .credential_for(
                Uuid::new_v4(),
                observe_process(std::process::id()).unwrap().birth,
                [9; 32],
            )
            .unwrap();
        let reference = store.publish(&credential, [9; 32]).unwrap();
        let launch = ProcessLaunch::new(
            ProcessInvocation::new("/bin/true"),
            root.path().to_path_buf(),
            vec![],
            PayloadIo {
                stdin: None,
                stdout: None,
                stderr: None,
            },
        )
        .unwrap();
        let start = OwnerStart::prepare(
            launch,
            credential.job_id,
            credential.nonce,
            [9; 32],
            Duration::from_millis(1),
        )
        .unwrap();
        let (channel, peer) = UnixStream::pair().unwrap();
        let prepared =
            OwnerPreparedStart::new(start, LaunchChannel::new(channel).unwrap()).unwrap();
        let evidence = prepared.abandon();
        drop(peer);
        assert!(evidence.observed() == (&credential, &[9; 32]));
        Self {
            root,
            store,
            reference,
            evidence,
        }
    }

    pub(super) fn record(&self) -> OwnerNoCommitRecord {
        OwnerNoCommitRecord::from_evidence(self.reference.record().clone(), &self.evidence).unwrap()
    }

    pub(super) fn outcome_path(&self) -> std::path::PathBuf {
        self.root.path().join(format!(
            "{}.outcome",
            self.reference.credential().job_id.simple()
        ))
    }

    pub(super) fn write_outcome(&self, bytes: &[u8]) {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(self.outcome_path())
            .unwrap()
            .write_all(bytes)
            .unwrap();
    }
}
