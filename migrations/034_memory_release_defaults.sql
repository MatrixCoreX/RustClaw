-- Restore release defaults for settings written by the old first-start initializer.
DELETE FROM memory_runtime_settings
WHERE setting_key = 'admin:default'
  AND setting_scope = 'admin'
  AND principal_id IS NULL
  AND conversation_id IS NULL
  AND use_mode = 'disabled'
  AND generate_mode = 'disabled'
  AND external_context_policy = 'inherit'
  AND managed_deny_use = 0
  AND managed_deny_generate = 0
  AND revision = 1
  AND policy_digest = 'onboarding-pending-v1'
  AND updated_by_principal_id IS NULL
  AND EXISTS (
      SELECT 1 FROM memory_onboarding_state
      WHERE singleton_id = 1 AND installation_class = 'new_install'
  );
