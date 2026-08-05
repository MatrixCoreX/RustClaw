CREATE TABLE IF NOT EXISTS runtime_schema_migrations (
    migration_id TEXT PRIMARY KEY,
    schema_digest TEXT NOT NULL,
    applied_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS principals (
    principal_id TEXT PRIMARY KEY,
    role TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'active'
        CHECK (status IN ('active', 'frozen', 'merged', 'deleted')),
    revision INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS credential_bindings (
    binding_id TEXT PRIMARY KEY,
    credential_digest TEXT NOT NULL UNIQUE,
    principal_id TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'active'
        CHECK (status IN ('active', 'revoked')),
    created_at TEXT NOT NULL,
    revoked_at TEXT,
    FOREIGN KEY(principal_id) REFERENCES principals(principal_id)
);

CREATE INDEX IF NOT EXISTS idx_credential_bindings_principal_status
ON credential_bindings(principal_id, status);

CREATE TABLE IF NOT EXISTS memory_runtime_settings (
    setting_key TEXT PRIMARY KEY,
    setting_scope TEXT NOT NULL
        CHECK (setting_scope IN ('admin', 'principal', 'conversation')),
    principal_id TEXT,
    conversation_id TEXT,
    use_mode TEXT NOT NULL DEFAULT 'inherit'
        CHECK (use_mode IN ('inherit', 'enabled', 'disabled')),
    generate_mode TEXT NOT NULL DEFAULT 'inherit'
        CHECK (generate_mode IN ('inherit', 'enabled', 'disabled')),
    external_context_policy TEXT NOT NULL DEFAULT 'inherit'
        CHECK (external_context_policy IN ('inherit', 'exclude', 'evidence_only', 'allow')),
    managed_deny_use INTEGER NOT NULL DEFAULT 0,
    managed_deny_generate INTEGER NOT NULL DEFAULT 0,
    revision INTEGER NOT NULL DEFAULT 1,
    policy_digest TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    updated_by_principal_id TEXT
);

CREATE INDEX IF NOT EXISTS idx_memory_runtime_settings_principal_scope
ON memory_runtime_settings(principal_id, setting_scope, conversation_id);

CREATE TABLE IF NOT EXISTS memory_onboarding_state (
    singleton_id INTEGER PRIMARY KEY CHECK (singleton_id = 1),
    installation_class TEXT NOT NULL
        CHECK (installation_class IN ('new_install', 'upgrade')),
    status TEXT NOT NULL
        CHECK (status IN ('pending_choice', 'upgrade_preserved', 'completed')),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS memory_project_identities (
    project_ref TEXT PRIMARY KEY,
    locator_digest TEXT NOT NULL UNIQUE,
    canonical_locator TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'active'
        CHECK (status IN ('active', 'unlinked')),
    revision INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS memory_project_aliases (
    alias_digest TEXT PRIMARY KEY,
    project_ref TEXT NOT NULL,
    canonical_alias TEXT NOT NULL,
    created_at TEXT NOT NULL,
    FOREIGN KEY(project_ref) REFERENCES memory_project_identities(project_ref)
);
