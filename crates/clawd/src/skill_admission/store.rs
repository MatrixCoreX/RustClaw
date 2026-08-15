use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use claw_core::config::AppConfig;
use claw_core::skill_registry::SkillsRegistry;
use fs2::FileExt;
use serde::de::DeserializeOwned;
use serde::Serialize;
use sha2::{Digest, Sha256};
use skill_sdk::{
    AdmissionReceipt, AdmissionState, HostPolicyGrant, InstallReceiptStore, PackageManifest,
};

use super::model::{
    AdmissionExecutionBinding, AdmissionMutation, ExternalSkillMetadata, GenerationPointer,
    GenerationRecord, OverlaySkillRecord, OverlaySnapshot, SkillAdmissionSource,
    OVERLAY_GENERATION_SCHEMA_VERSION,
};
use super::registry::render_registry_fragment;

const OVERLAY_DIRECTORY: &str = ".runtime-admission";

#[derive(Debug, thiserror::Error)]
#[error("{code}: {detail}")]
pub(crate) struct AdmissionServiceError {
    pub(crate) code: &'static str,
    pub(crate) detail: String,
}

type Result<T> = std::result::Result<T, AdmissionServiceError>;

#[derive(Debug, Clone)]
pub(crate) struct SkillAdmissionService {
    workspace_root: PathBuf,
    root: PathBuf,
    base_registry_path: PathBuf,
    package_root: PathBuf,
}

impl SkillAdmissionService {
    pub(crate) fn from_config(workspace_root: &Path, config: &AppConfig) -> Result<Self> {
        let data_root = resolve_path(workspace_root, &config.database.skill_data_root);
        let root = data_root.join(OVERLAY_DIRECTORY);
        if !root.starts_with(workspace_root) {
            return Err(error(
                "skill_admission_root_outside_workspace",
                format!(
                    "runtime overlay must remain workspace-relative: {}",
                    root.display()
                ),
            ));
        }
        let registry =
            config.skills.registry_path.as_deref().ok_or_else(|| {
                error("skill_registry_missing", "skills.registry_path is required")
            })?;
        Ok(Self {
            workspace_root: workspace_root.to_path_buf(),
            root,
            base_registry_path: resolve_path(workspace_root, registry),
            package_root: workspace_root.join("data/skill-packages"),
        })
    }

    #[cfg(test)]
    pub(crate) fn for_test(workspace_root: &Path, base_registry_path: &Path) -> Self {
        Self {
            workspace_root: workspace_root.to_path_buf(),
            root: workspace_root.join("data/skills").join(OVERLAY_DIRECTORY),
            base_registry_path: base_registry_path.to_path_buf(),
            package_root: workspace_root.join("data/skill-packages"),
        }
    }

    #[cfg(test)]
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn source_manifest_path(&self, skill_name: &str) -> Result<Option<PathBuf>> {
        let Some((_, record, generation_root)) = self.read_current_generation()? else {
            return Ok(None);
        };
        if !record.skills.contains_key(skill_name) {
            return Ok(None);
        }
        let metadata: ExternalSkillMetadata = read_json(
            &generation_root
                .join("metadata")
                .join(format!("{skill_name}.json")),
        )?;
        Ok(Some(PathBuf::from(metadata.package_manifest_path)))
    }

    pub(crate) fn is_bundled_skill(&self, skill_name: &str) -> Result<bool> {
        SkillsRegistry::load_from_path(&self.base_registry_path)
            .map(|registry| registry.get(skill_name).is_some())
            .map_err(|detail| error("skill_registry_invalid", detail))
    }

    pub(crate) fn snapshot(&self) -> Result<OverlaySnapshot> {
        let Some((pointer, record, generation_root)) = self.read_current_generation()? else {
            return Ok(OverlaySnapshot {
                base_registry_digest: Some(digest_file(&self.base_registry_path)?),
                ..OverlaySnapshot::default()
            });
        };
        self.validate_generation(&pointer, &record, &generation_root, true)
    }

    /// Read the signed admission state for control-plane display without
    /// re-hashing every installed package artifact.
    ///
    /// Runtime reload and execution continue to use `snapshot`, which performs
    /// full installed-artifact verification. This projection is only suitable
    /// for catalog/status UI and never grants execution authority.
    pub(crate) fn catalog_snapshot(&self) -> Result<OverlaySnapshot> {
        let Some((pointer, record, generation_root)) = self.read_current_generation()? else {
            return Ok(OverlaySnapshot {
                base_registry_digest: Some(digest_file(&self.base_registry_path)?),
                ..OverlaySnapshot::default()
            });
        };
        self.validate_generation(&pointer, &record, &generation_root, false)
    }

    pub(crate) fn admit_external(&self, mutation: AdmissionMutation) -> Result<OverlaySnapshot> {
        if mutation.metadata.source != SkillAdmissionSource::ExternalOverlay {
            return Err(error(
                "skill_admission_source_invalid",
                "external admission requires external_overlay source",
            ));
        }
        self.commit_mutation(mutation)
    }

    pub(crate) fn admit_bundled(&self, mutation: AdmissionMutation) -> Result<OverlaySnapshot> {
        if mutation.metadata.source != SkillAdmissionSource::BundledBase {
            return Err(error(
                "skill_admission_source_invalid",
                "bundled admission requires bundled_base source",
            ));
        }
        self.commit_mutation(mutation)
    }

    pub(crate) fn set_state(
        &self,
        skill_name: &str,
        state: AdmissionState,
        grant: Option<HostPolicyGrant>,
    ) -> Result<OverlaySnapshot> {
        let (_, record, generation_root) = self
            .read_current_generation()?
            .ok_or_else(|| error("skill_admission_missing", format!("skill={skill_name}")))?;
        if !record.skills.contains_key(skill_name) {
            return Err(error(
                "skill_admission_missing",
                format!("skill={skill_name}"),
            ));
        }
        let metadata = read_json(
            &generation_root
                .join("metadata")
                .join(format!("{skill_name}.json")),
        )?;
        let prompt = fs::read_to_string(
            generation_root
                .join("prompts")
                .join(format!("{skill_name}.md")),
        )
        .map_err(|source| {
            error(
                "skill_admission_prompt_read_failed",
                format!("skill={skill_name} error={source}"),
            )
        })?;
        self.commit_mutation(AdmissionMutation {
            metadata,
            prompt,
            state,
            grant,
        })
    }

    pub(crate) fn current_repair_inputs(&self) -> Result<Vec<AdmissionMutation>> {
        let Some((_, record, generation_root)) = self.read_current_generation()? else {
            return Err(error(
                "skill_admission_generation_missing",
                "skill_admission_generation_missing",
            ));
        };
        record
            .skills
            .iter()
            .map(|(name, skill)| {
                let metadata = read_json(
                    &generation_root
                        .join("metadata")
                        .join(format!("{name}.json")),
                )?;
                let prompt =
                    fs::read_to_string(generation_root.join("prompts").join(format!("{name}.md")))
                        .map_err(|source| {
                            error(
                                "skill_admission_prompt_read_failed",
                                format!("skill={name} error={source}"),
                            )
                        })?;
                let grant = read_optional_json::<HostPolicyGrant>(
                    &generation_root
                        .join("policy.d")
                        .join(format!("{name}.json")),
                )?;
                Ok(AdmissionMutation {
                    metadata,
                    prompt,
                    state: skill.state,
                    grant,
                })
            })
            .collect()
    }

    pub(crate) fn repair_current_generation(
        &self,
        mutations: Vec<AdmissionMutation>,
    ) -> Result<OverlaySnapshot> {
        self.commit_mutations(mutations, true, &BTreeSet::new())
    }

    pub(crate) fn retire_release_owned_bundled(
        &self,
        skill_names: &BTreeSet<String>,
    ) -> Result<OverlaySnapshot> {
        if skill_names.is_empty() {
            return Err(error(
                "skill_admission_retirement_empty",
                "skill_admission_retirement_empty",
            ));
        }
        let mut mutations = self.current_repair_inputs()?;
        mutations.retain(|mutation| !skill_names.contains(&mutation.metadata.name));
        self.commit_mutations(mutations, true, skill_names)
    }

    pub(crate) fn rollback_generation(&self, expected_generation: u64) -> Result<OverlaySnapshot> {
        fs::create_dir_all(&self.root).map_err(io_error("skill_admission_root_create_failed"))?;
        let lock = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(self.root.join("mutation.lock"))
            .map_err(io_error("skill_admission_lock_open_failed"))?;
        FileExt::lock_exclusive(&lock).map_err(io_error("skill_admission_lock_failed"))?;
        let result = (|| {
            let (current, record, _) = self.read_current_generation()?.ok_or_else(|| {
                error(
                    "skill_admission_generation_missing",
                    "current generation is absent",
                )
            })?;
            if current.generation != expected_generation {
                return Err(error(
                    "skill_admission_generation_changed",
                    format!(
                        "expected={expected_generation} actual={}",
                        current.generation
                    ),
                ));
            }
            let Some(previous) = record.previous_generation else {
                remove_optional_file(&self.root.join("current-generation.json"))?;
                sync_directory(&self.root)?;
                return self.snapshot();
            };
            let previous_root = self
                .root
                .join("generations")
                .join(generation_directory_name(previous));
            let previous_record: GenerationRecord =
                read_json(&previous_root.join("generation.json"))?;
            let pointer = GenerationPointer {
                schema_version: OVERLAY_GENERATION_SCHEMA_VERSION,
                generation: previous,
                generation_digest: digest_json(&previous_record)?,
                activated_at_unix: now_unix(),
            };
            self.validate_generation(&pointer, &previous_record, &previous_root, true)?;
            atomic_write_json(&self.root.join("current-generation.json"), &pointer)?;
            self.snapshot()
        })();
        let _ = FileExt::unlock(&lock);
        result
    }

    pub(crate) fn rollback_to_generation(&self, target_generation: u64) -> Result<OverlaySnapshot> {
        loop {
            let snapshot = self.snapshot()?;
            if snapshot.generation == target_generation {
                return Ok(snapshot);
            }
            if snapshot.generation < target_generation {
                return Err(error(
                    "skill_admission_rollback_target_invalid",
                    format!("target={target_generation} current={}", snapshot.generation),
                ));
            }
            self.rollback_generation(snapshot.generation)?;
        }
    }

    fn commit_mutation(&self, mutation: AdmissionMutation) -> Result<OverlaySnapshot> {
        self.commit_mutations(vec![mutation], false, &BTreeSet::new())
    }

    fn commit_mutations(
        &self,
        mutations: Vec<AdmissionMutation>,
        require_complete_current_generation: bool,
        retired_release_owned_bundled: &BTreeSet<String>,
    ) -> Result<OverlaySnapshot> {
        fs::create_dir_all(&self.root).map_err(io_error("skill_admission_root_create_failed"))?;
        secure_directory(&self.root)?;
        let lock_path = self.root.join("mutation.lock");
        let lock = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(&lock_path)
            .map_err(io_error("skill_admission_lock_open_failed"))?;
        FileExt::lock_exclusive(&lock).map_err(io_error("skill_admission_lock_failed"))?;
        let result = self.commit_mutations_locked(
            mutations,
            require_complete_current_generation,
            retired_release_owned_bundled,
        );
        let _ = FileExt::unlock(&lock);
        result
    }

    fn commit_mutations_locked(
        &self,
        mutations: Vec<AdmissionMutation>,
        require_complete_current_generation: bool,
        retired_release_owned_bundled: &BTreeSet<String>,
    ) -> Result<OverlaySnapshot> {
        if mutations.is_empty() && retired_release_owned_bundled.is_empty() {
            return Err(error(
                "skill_admission_repair_empty",
                "skill_admission_repair_empty",
            ));
        }
        let current = self.read_current_generation()?;
        let base = SkillsRegistry::load_from_path(&self.base_registry_path)
            .map_err(|detail| error("skill_registry_invalid", detail))?;
        if require_complete_current_generation {
            let (_, current_record, _) = current.as_ref().ok_or_else(|| {
                error(
                    "skill_admission_generation_missing",
                    "skill_admission_generation_missing",
                )
            })?;
            let requested = mutations
                .iter()
                .map(|mutation| (mutation.metadata.name.clone(), mutation.metadata.source))
                .collect::<BTreeMap<_, _>>();
            let expected = current_record
                .skills
                .iter()
                .map(|(name, skill)| (name.clone(), skill.source))
                .collect::<BTreeMap<_, _>>();
            let requested_scope_is_valid = requested.len() == mutations.len()
                && requested
                    .iter()
                    .all(|(name, source)| expected.get(name) == Some(source));
            let retired_scope_is_valid = retired_release_owned_bundled.iter().all(|name| {
                expected.get(name) == Some(&SkillAdmissionSource::BundledBase)
                    && base
                        .get(name)
                        .is_some_and(|entry| entry.install_mode.as_deref() != Some("on_demand"))
            });
            let scopes_are_disjoint = requested
                .keys()
                .all(|name| !retired_release_owned_bundled.contains(name));
            let complete_scope = expected.keys().all(|name| {
                requested.contains_key(name) || retired_release_owned_bundled.contains(name)
            }) && expected.len()
                == requested.len() + retired_release_owned_bundled.len();
            if !requested_scope_is_valid
                || !retired_scope_is_valid
                || !scopes_are_disjoint
                || !complete_scope
            {
                return Err(error(
                    "skill_admission_repair_scope_mismatch",
                    format!(
                        "expected={} requested={} retired={}",
                        expected.len(),
                        mutations.len(),
                        retired_release_owned_bundled.len()
                    ),
                ));
            }
        }
        let mut names = BTreeSet::new();
        let mut verified_mutations = Vec::with_capacity(mutations.len());
        for mutation in mutations {
            let name = mutation.metadata.name.trim().to_ascii_lowercase();
            if name.is_empty() || name != mutation.metadata.name || !names.insert(name.clone()) {
                return Err(error(
                    "skill_admission_name_invalid",
                    format!("name={:?}", mutation.metadata.name),
                ));
            }
            validate_relative_path(
                &mutation.metadata.package_manifest_path,
                "skill_admission_source_manifest_invalid",
            )?;
            match (mutation.metadata.source, base.get(&name)) {
                (SkillAdmissionSource::ExternalOverlay, Some(_)) => {
                    return Err(error(
                        "skill_admission_builtin_override_forbidden",
                        format!("skill={name}"),
                    ));
                }
                (SkillAdmissionSource::BundledBase, None) => {
                    return Err(error(
                        "skill_admission_base_entry_missing",
                        format!("skill={name}"),
                    ));
                }
                _ => {}
            }
            let verified = InstallReceiptStore::new(&self.package_root)
                .verified_current_install(&name)
                .map_err(|source| {
                    error(
                        "skill_install_receipt_invalid",
                        format!("skill={name} error={source}"),
                    )
                })?;
            verified
                .manifest
                .validate_for_platform(&skill_sdk::HostPlatform::current())
                .map_err(|source| error("skill_platform_incompatible", source.to_string()))?;
            if verified.manifest.package.name != name {
                return Err(error(
                    "skill_admission_identity_mismatch",
                    format!(
                        "metadata={} manifest={}",
                        name, verified.manifest.package.name
                    ),
                ));
            }
            verified_mutations.push((mutation, verified));
        }

        let generation = self.next_generation(current.as_ref().map(|(pointer, _, _)| pointer))?;
        let staging_root = self.root.join(format!(".staging-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&staging_root)
            .map_err(io_error("skill_admission_staging_create_failed"))?;
        secure_directory(&staging_root)?;
        let mut staging = StagingGuard::new(staging_root);
        let mut record = if let Some((pointer, previous, previous_root)) = &current {
            copy_directory(previous_root, staging.path())?;
            GenerationRecord {
                schema_version: OVERLAY_GENERATION_SCHEMA_VERSION,
                generation,
                previous_generation: Some(pointer.generation),
                created_at_unix: now_unix(),
                skills: previous.skills.clone(),
            }
        } else {
            GenerationRecord {
                schema_version: OVERLAY_GENERATION_SCHEMA_VERSION,
                generation,
                previous_generation: None,
                created_at_unix: now_unix(),
                skills: BTreeMap::new(),
            }
        };
        for name in retired_release_owned_bundled {
            remove_retired_skill_files(staging.path(), name)?;
            record.skills.remove(name);
        }
        for (mutation, verified) in &verified_mutations {
            self.write_mutated_skill(
                staging.path(),
                generation,
                mutation,
                &verified.manifest,
                &verified.receipt,
                &mut record,
            )?;
        }
        self.refresh_admission_generations(staging.path(), &mut record)?;
        let generation_digest = digest_json(&record)?;
        atomic_write_json(&staging.path().join("generation.json"), &record)?;
        let pointer = GenerationPointer {
            schema_version: OVERLAY_GENERATION_SCHEMA_VERSION,
            generation,
            generation_digest,
            activated_at_unix: now_unix(),
        };
        self.validate_generation(&pointer, &record, staging.path(), true)?;

        let generations_root = self.root.join("generations");
        fs::create_dir_all(&generations_root)
            .map_err(io_error("skill_admission_generations_create_failed"))?;
        let destination = generations_root.join(generation_directory_name(generation));
        fs::rename(staging.path(), &destination)
            .map_err(io_error("skill_admission_generation_commit_failed"))?;
        staging.disarm();
        atomic_write_json(&self.root.join("current-generation.json"), &pointer)?;
        self.snapshot()
    }

    fn write_mutated_skill(
        &self,
        generation_root: &Path,
        generation: u64,
        mutation: &AdmissionMutation,
        manifest: &PackageManifest,
        receipt: &skill_sdk::InstallReceipt,
        generation_record: &mut GenerationRecord,
    ) -> Result<()> {
        let name = &mutation.metadata.name;
        let metadata_path = generation_root
            .join("metadata")
            .join(format!("{name}.json"));
        let prompt_path = generation_root.join("prompts").join(format!("{name}.md"));
        let manifest_path = generation_root
            .join("manifests")
            .join(name)
            .join("skill.toml");
        atomic_write_json(&metadata_path, &mutation.metadata)?;
        atomic_write(&prompt_path, mutation.prompt.as_bytes())?;
        atomic_write(
            &manifest_path,
            manifest
                .to_toml_string()
                .map_err(|source| error("skill_manifest_encode_failed", source.to_string()))?
                .as_bytes(),
        )?;

        let policy_path = generation_root
            .join("policy.d")
            .join(format!("{name}.json"));
        let existing_grant = read_optional_json::<HostPolicyGrant>(&policy_path)?;
        let effective_grant = mutation.grant.as_ref().or(existing_grant.as_ref());
        match mutation.state {
            AdmissionState::Enabled | AdmissionState::InstalledDisabled => {
                let grant = effective_grant
                    .ok_or_else(|| error("skill_policy_grant_missing", format!("skill={name}")))?;
                grant
                    .validate_against(manifest)
                    .map_err(|source| error("skill_policy_grant_invalid", source.to_string()))?;
                atomic_write_json(&policy_path, grant)?;
            }
            AdmissionState::AwaitingPolicyApproval => {
                remove_optional_file(&policy_path)?;
            }
            AdmissionState::Tombstoned => {
                if let Some(grant) = effective_grant {
                    grant.validate_against(manifest).map_err(|source| {
                        error("skill_policy_grant_invalid", source.to_string())
                    })?;
                    atomic_write_json(&policy_path, grant)?;
                }
            }
        }
        let persisted_grant = read_optional_json::<HostPolicyGrant>(&policy_path)?;

        let registry_path = generation_root
            .join("registry.d")
            .join(format!("{name}.toml"));
        let registry_fragment_digest = match (mutation.metadata.source, mutation.state) {
            (
                SkillAdmissionSource::ExternalOverlay,
                AdmissionState::Enabled
                | AdmissionState::InstalledDisabled
                | AdmissionState::Tombstoned,
            ) => {
                let grant = persisted_grant
                    .as_ref()
                    .ok_or_else(|| error("skill_policy_grant_missing", format!("skill={name}")))?;
                let relative_generation = self.generation_relative_path(generation)?;
                let registry = render_registry_fragment(
                    manifest,
                    &mutation.metadata,
                    grant,
                    mutation.state == AdmissionState::Enabled,
                    mutation.state != AdmissionState::Tombstoned,
                    relative_generation
                        .join("manifests")
                        .join(name)
                        .join("skill.toml")
                        .to_string_lossy()
                        .to_string(),
                    relative_generation
                        .join("prompts")
                        .join(format!("{name}.md"))
                        .to_string_lossy()
                        .to_string(),
                )
                .map_err(|detail| error("skill_registry_fragment_invalid", detail))?;
                atomic_write(&registry_path, registry.as_bytes())?;
                Some(digest_bytes(registry.as_bytes()))
            }
            (SkillAdmissionSource::ExternalOverlay, AdmissionState::AwaitingPolicyApproval)
            | (SkillAdmissionSource::BundledBase, _) => {
                remove_optional_file(&registry_path)?;
                None
            }
        };

        let admission = AdmissionReceipt::from_install(
            receipt,
            manifest,
            generation,
            mutation.state,
            persisted_grant.as_ref(),
            now_unix(),
        )
        .map_err(|source| error("skill_admission_receipt_invalid", source.to_string()))?;
        let admission_path = generation_root
            .join("admissions")
            .join(format!("{name}.json"));
        atomic_write_json(&admission_path, &admission)?;
        generation_record.skills.insert(
            name.clone(),
            OverlaySkillRecord {
                source: mutation.metadata.source,
                state: mutation.state,
                manifest_digest: manifest
                    .digest()
                    .map_err(|source| error("skill_manifest_digest_failed", source.to_string()))?,
                metadata_digest: digest_file(&metadata_path)?,
                prompt_digest: digest_file(&prompt_path)?,
                registry_fragment_digest,
                policy_digest: persisted_grant
                    .as_ref()
                    .map(|grant| grant.digest(manifest))
                    .transpose()
                    .map_err(|source| error("skill_policy_digest_failed", source.to_string()))?,
                admission_receipt_digest: admission
                    .digest()
                    .map_err(|source| error("skill_admission_digest_failed", source.to_string()))?,
            },
        );
        Ok(())
    }

    fn refresh_admission_generations(
        &self,
        generation_root: &Path,
        generation_record: &mut GenerationRecord,
    ) -> Result<()> {
        for (name, skill) in &mut generation_record.skills {
            let path = generation_root
                .join("admissions")
                .join(format!("{name}.json"));
            let mut admission: AdmissionReceipt = read_json(&path)?;
            admission.registry_generation = generation_record.generation;
            admission.state = skill.state;
            admission.admitted_at_unix = generation_record.created_at_unix;
            admission
                .validate()
                .map_err(|source| error("skill_admission_receipt_invalid", source.to_string()))?;
            atomic_write_json(&path, &admission)?;
            skill.admission_receipt_digest = admission
                .digest()
                .map_err(|source| error("skill_admission_digest_failed", source.to_string()))?;
        }
        Ok(())
    }

    fn read_current_generation(
        &self,
    ) -> Result<Option<(GenerationPointer, GenerationRecord, PathBuf)>> {
        let pointer_path = self.root.join("current-generation.json");
        if !pointer_path.is_file() {
            return Ok(None);
        }
        let pointer: GenerationPointer = read_json(&pointer_path)?;
        if pointer.schema_version != OVERLAY_GENERATION_SCHEMA_VERSION
            || pointer.generation == 0
            || pointer.activated_at_unix == 0
        {
            return Err(error(
                "skill_admission_pointer_invalid",
                format!("path={}", pointer_path.display()),
            ));
        }
        let root = self
            .root
            .join("generations")
            .join(generation_directory_name(pointer.generation));
        let record: GenerationRecord = read_json(&root.join("generation.json"))?;
        Ok(Some((pointer, record, root)))
    }

    fn validate_generation(
        &self,
        pointer: &GenerationPointer,
        record: &GenerationRecord,
        generation_root: &Path,
        verify_installed_artifacts: bool,
    ) -> Result<OverlaySnapshot> {
        if record.schema_version != OVERLAY_GENERATION_SCHEMA_VERSION
            || record.generation != pointer.generation
            || record.created_at_unix == 0
            || digest_json(record)? != pointer.generation_digest
        {
            return Err(error(
                "skill_admission_generation_invalid",
                format!("generation={}", pointer.generation),
            ));
        }
        let mut snapshot = OverlaySnapshot {
            generation: pointer.generation,
            generation_digest: Some(pointer.generation_digest.clone()),
            base_registry_digest: Some(digest_file(&self.base_registry_path)?),
            registry_dir: Some(generation_root.join("registry.d")),
            ..OverlaySnapshot::default()
        };
        for (name, skill) in &record.skills {
            let manifest_path = generation_root
                .join("manifests")
                .join(name)
                .join("skill.toml");
            let manifest = PackageManifest::load(&manifest_path)
                .map_err(|source| error("skill_manifest_invalid", source.to_string()))?;
            if manifest.package.name != *name
                || manifest
                    .digest()
                    .map_err(|source| error("skill_manifest_digest_failed", source.to_string()))?
                    != skill.manifest_digest
            {
                return Err(error(
                    "skill_admission_manifest_mismatch",
                    format!("skill={name}"),
                ));
            }
            let metadata_path = generation_root
                .join("metadata")
                .join(format!("{name}.json"));
            let metadata: ExternalSkillMetadata = read_json(&metadata_path)?;
            let prompt_path = generation_root.join("prompts").join(format!("{name}.md"));
            if metadata.name != *name
                || metadata.source != skill.source
                || digest_file(&metadata_path)? != skill.metadata_digest
                || digest_file(&prompt_path)? != skill.prompt_digest
            {
                return Err(error(
                    "skill_admission_metadata_mismatch",
                    format!("skill={name}"),
                ));
            }
            validate_relative_path(
                &metadata.package_manifest_path,
                "skill_admission_source_manifest_invalid",
            )?;
            let admission_path = generation_root
                .join("admissions")
                .join(format!("{name}.json"));
            let admission: AdmissionReceipt = read_json(&admission_path)?;
            admission
                .validate()
                .map_err(|source| error("skill_admission_receipt_invalid", source.to_string()))?;
            if admission.skill_name != *name
                || admission.state != skill.state
                || admission.registry_generation != record.generation
                || admission.manifest_digest != skill.manifest_digest
                || admission
                    .digest()
                    .map_err(|source| error("skill_admission_digest_failed", source.to_string()))?
                    != skill.admission_receipt_digest
            {
                return Err(error(
                    "skill_admission_receipt_mismatch",
                    format!("skill={name}"),
                ));
            }
            let policy_path = generation_root
                .join("policy.d")
                .join(format!("{name}.json"));
            let grant = read_optional_json::<HostPolicyGrant>(&policy_path)?;
            match (grant.as_ref(), skill.policy_digest.as_deref()) {
                (Some(grant), Some(expected)) => {
                    let actual = grant.digest(&manifest).map_err(|source| {
                        error("skill_policy_grant_invalid", source.to_string())
                    })?;
                    if actual != expected
                        || admission.granted_policy_digest.as_deref() != Some(actual.as_str())
                    {
                        return Err(error(
                            "skill_admission_policy_mismatch",
                            format!("skill={name}"),
                        ));
                    }
                }
                (None, None) if admission.granted_policy_digest.is_none() => {}
                _ => {
                    return Err(error(
                        "skill_admission_policy_mismatch",
                        format!("skill={name}"),
                    ));
                }
            }
            let registry_path = generation_root
                .join("registry.d")
                .join(format!("{name}.toml"));
            match (skill.source, &skill.registry_fragment_digest, skill.state) {
                (
                    SkillAdmissionSource::ExternalOverlay,
                    Some(expected),
                    AdmissionState::Enabled
                    | AdmissionState::InstalledDisabled
                    | AdmissionState::Tombstoned,
                ) if digest_file(&registry_path)? == *expected => {}
                (
                    SkillAdmissionSource::ExternalOverlay,
                    None,
                    AdmissionState::AwaitingPolicyApproval,
                )
                | (SkillAdmissionSource::BundledBase, None, _)
                    if !registry_path.exists() => {}
                _ => {
                    return Err(error(
                        "skill_admission_registry_mismatch",
                        format!("skill={name}"),
                    ));
                }
            }
            if verify_installed_artifacts && skill.state != AdmissionState::Tombstoned {
                admission
                    .verify_installed(&self.package_root)
                    .map_err(|source| {
                        error(
                            "skill_install_receipt_invalid",
                            format!("skill={name} error={source}"),
                        )
                    })?;
            }
            match skill.state {
                AdmissionState::Enabled => {
                    snapshot.enabled.insert(name.clone());
                    snapshot.execution_bindings.insert(
                        name.clone(),
                        AdmissionExecutionBinding {
                            version: admission.version.clone(),
                            manifest_digest: admission.manifest_digest.clone(),
                            install_receipt_digest: admission.install_receipt_digest.clone(),
                            policy_digest: admission.granted_policy_digest.clone(),
                            admission_receipt_digest: skill.admission_receipt_digest.clone(),
                        },
                    );
                }
                AdmissionState::InstalledDisabled => {
                    snapshot.disabled.insert(name.clone());
                }
                AdmissionState::AwaitingPolicyApproval => {
                    snapshot.awaiting_policy.insert(name.clone());
                }
                AdmissionState::Tombstoned => {
                    snapshot.tombstoned.insert(name.clone());
                }
            }
            snapshot.sources.insert(name.clone(), skill.source);
        }
        SkillsRegistry::load_from_base_and_overlay(
            &self.base_registry_path,
            snapshot.registry_dir.as_deref(),
        )
        .map_err(|detail| error("skill_registry_overlay_invalid", detail))?;
        Ok(snapshot)
    }

    fn generation_relative_path(&self, generation: u64) -> Result<PathBuf> {
        self.root
            .join("generations")
            .join(generation_directory_name(generation))
            .strip_prefix(&self.workspace_root)
            .map(Path::to_path_buf)
            .map_err(|_| {
                error(
                    "skill_admission_root_outside_workspace",
                    format!("root={}", self.root.display()),
                )
            })
    }

    fn next_generation(&self, current: Option<&GenerationPointer>) -> Result<u64> {
        let mut next = current.map_or(1, |pointer| pointer.generation.saturating_add(1));
        let generations = self.root.join("generations");
        if generations.is_dir() {
            for entry in fs::read_dir(&generations)
                .map_err(io_error("skill_admission_generations_read_failed"))?
            {
                let entry = entry.map_err(io_error("skill_admission_generation_read_failed"))?;
                if let Some(value) = entry
                    .file_name()
                    .to_str()
                    .and_then(|value| value.parse::<u64>().ok())
                {
                    next = next.max(value.saturating_add(1));
                }
            }
        }
        if next == 0 {
            return Err(error(
                "skill_admission_generation_exhausted",
                "generation counter overflowed",
            ));
        }
        Ok(next)
    }
}

fn validate_relative_path(value: &str, code: &'static str) -> Result<()> {
    let path = Path::new(value.trim());
    if value.trim().is_empty() || path.is_absolute() {
        return Err(error(code, format!("path={value:?}")));
    }
    for component in path.components() {
        if !matches!(component, std::path::Component::Normal(_)) {
            return Err(error(code, format!("path={value:?}")));
        }
    }
    Ok(())
}

fn resolve_path(workspace_root: &Path, value: &str) -> PathBuf {
    let path = Path::new(value);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace_root.join(path)
    }
}

fn generation_directory_name(generation: u64) -> String {
    format!("{generation:020}")
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .max(1)
}

fn digest_json(value: &impl Serialize) -> Result<String> {
    let bytes = serde_json::to_vec(value)
        .map_err(|source| error("skill_admission_json_encode_failed", source.to_string()))?;
    Ok(digest_bytes(&bytes))
}

fn digest_file(path: &Path) -> Result<String> {
    let bytes = fs::read(path).map_err(|source| {
        error(
            "skill_admission_file_read_failed",
            format!("path={} error={source}", path.display()),
        )
    })?;
    Ok(digest_bytes(&bytes))
}

fn digest_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let bytes = fs::read(path).map_err(|source| {
        error(
            "skill_admission_json_read_failed",
            format!("path={} error={source}", path.display()),
        )
    })?;
    serde_json::from_slice(&bytes).map_err(|source| {
        error(
            "skill_admission_json_invalid",
            format!("path={} error={source}", path.display()),
        )
    })
}

fn read_optional_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map(Some).map_err(|source| {
            error(
                "skill_admission_json_invalid",
                format!("path={} error={source}", path.display()),
            )
        }),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(error(
            "skill_admission_json_read_failed",
            format!("path={} error={source}", path.display()),
        )),
    }
}

fn atomic_write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|source| error("skill_admission_json_encode_failed", source.to_string()))?;
    atomic_write(path, &bytes)
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().ok_or_else(|| {
        error(
            "skill_admission_parent_missing",
            format!("path={}", path.display()),
        )
    })?;
    fs::create_dir_all(parent).map_err(io_error("skill_admission_parent_create_failed"))?;
    secure_directory(parent)?;
    let temporary = parent.join(format!(".tmp-{}", uuid::Uuid::new_v4()));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .map_err(io_error("skill_admission_temp_create_failed"))?;
    file.write_all(bytes)
        .map_err(io_error("skill_admission_temp_write_failed"))?;
    file.sync_all()
        .map_err(io_error("skill_admission_temp_sync_failed"))?;
    fs::rename(&temporary, path).map_err(|source| {
        let _ = fs::remove_file(&temporary);
        error(
            "skill_admission_atomic_commit_failed",
            format!("path={} error={source}", path.display()),
        )
    })?;
    sync_directory(parent)
}

fn remove_optional_file(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(error(
            "skill_admission_file_remove_failed",
            format!("path={} error={source}", path.display()),
        )),
    }
}

fn remove_retired_skill_files(generation_root: &Path, skill_name: &str) -> Result<()> {
    for path in [
        generation_root
            .join("admissions")
            .join(format!("{skill_name}.json")),
        generation_root
            .join("metadata")
            .join(format!("{skill_name}.json")),
        generation_root
            .join("policy.d")
            .join(format!("{skill_name}.json")),
        generation_root
            .join("prompts")
            .join(format!("{skill_name}.md")),
        generation_root
            .join("registry.d")
            .join(format!("{skill_name}.toml")),
    ] {
        remove_optional_file(&path)?;
    }
    let manifest_root = generation_root.join("manifests").join(skill_name);
    match fs::remove_dir_all(&manifest_root) {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(error(
            "skill_admission_directory_remove_failed",
            format!("path={} error={source}", manifest_root.display()),
        )),
    }
}

fn copy_directory(source: &Path, destination: &Path) -> Result<()> {
    for entry in fs::read_dir(source).map_err(io_error("skill_admission_copy_read_failed"))? {
        let entry = entry.map_err(io_error("skill_admission_copy_entry_failed"))?;
        let file_type = entry
            .file_type()
            .map_err(io_error("skill_admission_copy_type_failed"))?;
        if file_type.is_symlink() {
            return Err(error(
                "skill_admission_symlink_forbidden",
                format!("path={}", entry.path().display()),
            ));
        }
        let target = destination.join(entry.file_name());
        if file_type.is_dir() {
            fs::create_dir_all(&target).map_err(io_error("skill_admission_copy_create_failed"))?;
            copy_directory(&entry.path(), &target)?;
        } else if file_type.is_file() {
            fs::copy(entry.path(), &target)
                .map_err(io_error("skill_admission_copy_file_failed"))?;
        } else {
            return Err(error(
                "skill_admission_entry_type_forbidden",
                format!("path={}", entry.path().display()),
            ));
        }
    }
    Ok(())
}

fn secure_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(io_error("skill_admission_permissions_failed"))?;
    }
    Ok(())
}

fn sync_directory(path: &Path) -> Result<()> {
    File::open(path)
        .and_then(|file| file.sync_all())
        .map_err(io_error("skill_admission_directory_sync_failed"))
}

fn error(code: &'static str, detail: impl Into<String>) -> AdmissionServiceError {
    AdmissionServiceError {
        code,
        detail: detail.into(),
    }
}

fn io_error(code: &'static str) -> impl FnOnce(std::io::Error) -> AdmissionServiceError + Copy {
    move |source| error(code, source.to_string())
}

struct StagingGuard {
    path: PathBuf,
    armed: bool,
}

impl StagingGuard {
    fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for StagingGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
