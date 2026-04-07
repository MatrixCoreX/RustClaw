CREATE TABLE IF NOT EXISTS pending_channel_bind_sessions (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    channel           TEXT NOT NULL,
    user_key          TEXT NOT NULL,
    bind_token        TEXT NOT NULL,
    status            TEXT NOT NULL CHECK (status IN ('pending', 'detected', 'bound', 'failed', 'expired')),
    external_user_id  TEXT,
    external_chat_id  TEXT,
    error_text        TEXT,
    install_device_code TEXT,
    install_verification_url TEXT,
    install_poll_interval_seconds INTEGER,
    created_at        TEXT NOT NULL,
    updated_at        TEXT NOT NULL,
    expires_at        TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_pending_channel_bind_sessions_channel_status
ON pending_channel_bind_sessions(channel, status);

CREATE INDEX IF NOT EXISTS idx_pending_channel_bind_sessions_bind_token
ON pending_channel_bind_sessions(bind_token);
