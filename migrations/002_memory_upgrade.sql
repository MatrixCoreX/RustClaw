CREATE TABLE IF NOT EXISTS user_preferences (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id      INTEGER NOT NULL,
    chat_id      INTEGER NOT NULL,
    pref_key     TEXT NOT NULL,
    pref_value   TEXT NOT NULL,
    confidence   REAL NOT NULL DEFAULT 0.8,
    source       TEXT NOT NULL DEFAULT 'memory_extract',
    updated_at   TEXT NOT NULL,
    UNIQUE(user_id, chat_id, pref_key)
);

CREATE INDEX IF NOT EXISTS idx_user_preferences_user_chat_updated
ON user_preferences(user_id, chat_id, updated_at);
