CREATE TABLE IF NOT EXISTS auth_keys (
    user_key     TEXT PRIMARY KEY,
    role         TEXT NOT NULL CHECK (role IN ('admin', 'user')),
    enabled      INTEGER NOT NULL DEFAULT 1,
    created_at   TEXT NOT NULL,
    last_used_at TEXT
);

CREATE TABLE IF NOT EXISTS channel_bindings (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    channel           TEXT NOT NULL,
    external_user_id  TEXT,
    external_chat_id  TEXT,
    user_key          TEXT NOT NULL,
    bound_at          TEXT NOT NULL,
    updated_at        TEXT NOT NULL,
    UNIQUE(channel, external_user_id, external_chat_id)
);

CREATE INDEX IF NOT EXISTS idx_channel_bindings_lookup
ON channel_bindings(channel, external_user_id, external_chat_id);

CREATE TABLE IF NOT EXISTS exchange_api_credentials (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    user_key    TEXT NOT NULL,
    exchange    TEXT NOT NULL,
    api_key     TEXT NOT NULL,
    api_secret  TEXT NOT NULL,
    passphrase  TEXT,
    enabled     INTEGER NOT NULL DEFAULT 1,
    updated_at  TEXT NOT NULL,
    UNIQUE(user_key, exchange)
);

CREATE INDEX IF NOT EXISTS idx_exchange_api_credentials_user_exchange
ON exchange_api_credentials(user_key, exchange);
