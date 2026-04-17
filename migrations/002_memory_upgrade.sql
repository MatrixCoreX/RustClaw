CREATE TABLE IF NOT EXISTS user_preferences (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id      INTEGER NOT NULL,
    chat_id      INTEGER NOT NULL,
    user_key     TEXT,
    pref_key     TEXT NOT NULL,
    pref_value   TEXT NOT NULL,
    confidence   REAL NOT NULL DEFAULT 0.8,
    source       TEXT NOT NULL DEFAULT 'memory_extract',
    updated_at   TEXT NOT NULL,
    updated_at_ts INTEGER NOT NULL DEFAULT 0,
    UNIQUE(user_id, chat_id, user_key, pref_key)
);

CREATE INDEX IF NOT EXISTS idx_user_preferences_user_chat_updated
ON user_preferences(user_id, chat_id, updated_at);
CREATE INDEX IF NOT EXISTS idx_user_preferences_user_chat_updated_ts
ON user_preferences(user_id, chat_id, updated_at_ts);
CREATE INDEX IF NOT EXISTS idx_user_preferences_user_key_chat
ON user_preferences(user_key, chat_id, updated_at_ts);
