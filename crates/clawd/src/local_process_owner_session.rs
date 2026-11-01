use super::owner_protocol::{
    OwnerAction, OwnerCredential, OwnerFailure, OwnerOutcome, OwnerReply, OwnerRequest,
    OwnerSnapshot,
};

// Credentials are private and deliberately have no Debug/serialization wrapper.
pub(crate) struct OwnerSession {
    credential: OwnerCredential,
    last_reply: Option<(OwnerAction, OwnerReply)>,
}

impl OwnerSession {
    pub(crate) fn new(credential: OwnerCredential) -> Result<Self, OwnerFailure> {
        credential.validate()?;
        Ok(Self {
            credential,
            last_reply: None,
        })
    }

    pub(crate) fn credential(&self) -> OwnerCredential {
        self.credential.clone()
    }

    pub(crate) fn handle(
        &mut self,
        request: &OwnerRequest,
        execute: impl FnOnce(OwnerAction) -> Result<OwnerSnapshot, OwnerFailure>,
    ) -> Result<OwnerReply, OwnerFailure> {
        if request.schema_version != 1 {
            return Err(OwnerFailure::UnsupportedSchema);
        }
        request.credential.validate()?;
        if self.credential != request.credential {
            return Err(OwnerFailure::CredentialMismatch);
        }
        let previous = self
            .last_reply
            .map(|(_, reply)| reply.request_id)
            .unwrap_or(0);
        if let Some((action, reply)) = self.last_reply {
            if request.request_id == previous && action == request.action {
                return Ok(reply);
            }
        }
        if request.request_id != 0 && request.request_id <= previous {
            return Err(OwnerFailure::ReplayConflict);
        }
        let expected = previous
            .checked_add(1)
            .ok_or(OwnerFailure::SequenceExhausted)?;
        if request.request_id != expected {
            return Err(OwnerFailure::SequenceGap);
        }
        let outcome = match execute(request.action).and_then(|snapshot| {
            snapshot.validate()?;
            Ok(snapshot)
        }) {
            Ok(snapshot) => OwnerOutcome::Ok { snapshot },
            Err(error_code) => OwnerOutcome::Error { error_code },
        };
        let reply = OwnerReply {
            schema_version: 1,
            request_id: request.request_id,
            outcome,
        };
        // Cache failures too: an interrupted response must not repeat an action.
        self.last_reply = Some((request.action, reply));
        Ok(reply)
    }
}

#[cfg(test)]
#[path = "local_process_owner_session_tests.rs"]
mod tests;
