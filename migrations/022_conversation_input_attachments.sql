CREATE TABLE IF NOT EXISTS conversation_input_attachments (
    attachment_id TEXT PRIMARY KEY,
    input_id TEXT NOT NULL,
    owner_principal_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    workspace_rel_path TEXT NOT NULL,
    mime_type TEXT,
    display_name TEXT,
    size_bytes INTEGER NOT NULL,
    sha256 TEXT NOT NULL,
    created_at_ts INTEGER NOT NULL,
    UNIQUE (input_id, workspace_rel_path)
);

CREATE INDEX IF NOT EXISTS idx_conversation_input_attachments_input
    ON conversation_input_attachments(input_id, attachment_id);
