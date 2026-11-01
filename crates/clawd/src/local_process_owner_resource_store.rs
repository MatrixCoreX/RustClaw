use super::super::owner_resources::{OwnerResourceBudget, OwnerResourceRecord};
use super::*;

fn resource_name(job: Uuid) -> Result<CString, OwnerReferenceFailure> {
    if job.is_nil() {
        return Err(OwnerReferenceFailure::InvalidReference);
    }
    CString::new(format!("{job}.resources.json"))
        .map_err(|_| OwnerReferenceFailure::InvalidReference)
}

impl OwnerReferenceStore {
    pub(crate) fn retain_resources(
        &self,
        reference: &DurableOwnerReference,
        budget: OwnerResourceBudget,
    ) -> Result<(), OwnerReferenceFailure> {
        self.require_reference(reference)?;
        budget.validate()?;
        if let Some(existing) = self.load_resources(reference)? {
            return if existing == budget {
                Ok(())
            } else {
                Err(OwnerReferenceFailure::Conflict)
            };
        }
        if self.load_outcome(reference)?.is_some() {
            return Err(OwnerReferenceFailure::Conflict);
        }
        let record = OwnerResourceRecord {
            schema_version: 1,
            reference: reference.record().clone(),
            budget: budget.clone(),
        };
        let bytes = self.key.sign_resources(record)?;
        self.publish_record(&resource_name(reference.credential.job_id)?, &bytes)?;
        match self.load_resources(reference)? {
            Some(actual) if actual == budget => Ok(()),
            Some(_) => Err(OwnerReferenceFailure::Conflict),
            None => Err(OwnerReferenceFailure::StorageUnavailable),
        }
    }

    pub(crate) fn load_resources(
        &self,
        reference: &DurableOwnerReference,
    ) -> Result<Option<OwnerResourceBudget>, OwnerReferenceFailure> {
        self.require_reference(reference)?;
        let Some(bytes) = self.read_record(&resource_name(reference.credential.job_id)?)? else {
            return Ok(None);
        };
        let record = self.key.verify_resources(&bytes)?;
        if record.reference != *reference.record() {
            return Err(OwnerReferenceFailure::Conflict);
        }
        Ok(Some(record.budget))
    }
}
