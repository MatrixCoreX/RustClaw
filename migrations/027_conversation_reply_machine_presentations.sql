-- Machine-addressed presentation metadata for durable, non-terminal replies.
-- The base reply row remains usable for model-authored text. Runtime-authored
-- lifecycle status uses a message key plus bounded public parameters instead
-- of embedding a locale-specific response in production code.

CREATE TABLE IF NOT EXISTS conversation_reply_item_presentations (
    reply_id      TEXT PRIMARY KEY,
    message_key   TEXT NOT NULL,
    params_json   TEXT NOT NULL DEFAULT '{}',
    FOREIGN KEY(reply_id) REFERENCES conversation_reply_items(reply_id) ON DELETE CASCADE
);
