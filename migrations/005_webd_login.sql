-- Webd UI: username/password mapped to auth_keys.user_key (Argon2 hash in password_hash).
CREATE TABLE IF NOT EXISTS webd_login_accounts (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    username       TEXT NOT NULL COLLATE NOCASE UNIQUE,
    password_hash  TEXT NOT NULL,
    user_key       TEXT NOT NULL,
    enabled        INTEGER NOT NULL DEFAULT 1,
    created_at     TEXT NOT NULL,
    updated_at     TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_webd_login_accounts_user_key ON webd_login_accounts(user_key);
