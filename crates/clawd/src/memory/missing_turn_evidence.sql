-- Record only the ownership scope and time window needed to invalidate a
-- failed historical pairing. Message content and credentials stay in their source.
CREATE TABLE IF NOT EXISTS memory_turn_evidence_deferrals (
    task_id TEXT PRIMARY KEY,
    principal_id TEXT NOT NULL,
    chat_id INTEGER NOT NULL,
    finished_at INTEGER NOT NULL,
    reason_code TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_memory_turn_deferred_scope
ON memory_turn_evidence_deferrals(principal_id,chat_id,finished_at);
CREATE TRIGGER IF NOT EXISTS deferred_turn_task_deleted AFTER DELETE ON tasks BEGIN
    DELETE FROM memory_turn_evidence_deferrals WHERE task_id=OLD.task_id;
END;
CREATE TRIGGER IF NOT EXISTS deferred_turn_task_changed
AFTER UPDATE OF user_key, principal_id, chat_id, status, updated_at, payload_json, result_json ON tasks
WHEN EXISTS(SELECT 1 FROM memory_turn_evidence_deferrals WHERE task_id=NEW.task_id)
BEGIN
    DELETE FROM memory_turn_evidence_deferrals WHERE task_id=NEW.task_id;
END;
CREATE TRIGGER IF NOT EXISTS deferred_turn_memory_added
AFTER INSERT ON memories BEGIN
    DELETE FROM memory_turn_evidence_deferrals
    WHERE principal_id=NEW.principal_id AND chat_id=NEW.chat_id
      AND finished_at BETWEEN NEW.created_at_ts-5 AND NEW.created_at_ts+5;
END;
CREATE TRIGGER IF NOT EXISTS deferred_turn_memory_removed
AFTER DELETE ON memories BEGIN
    DELETE FROM memory_turn_evidence_deferrals
    WHERE principal_id=OLD.principal_id AND chat_id=OLD.chat_id
      AND finished_at BETWEEN OLD.created_at_ts-5 AND OLD.created_at_ts+5;
END;
CREATE TRIGGER IF NOT EXISTS deferred_turn_memory_changed
AFTER UPDATE OF principal_id,chat_id,user_key,role,content,created_at_ts ON memories BEGIN
    DELETE FROM memory_turn_evidence_deferrals
    WHERE principal_id=OLD.principal_id AND chat_id=OLD.chat_id
      AND finished_at BETWEEN OLD.created_at_ts-5 AND OLD.created_at_ts+5;
    DELETE FROM memory_turn_evidence_deferrals
    WHERE principal_id=NEW.principal_id AND chat_id=NEW.chat_id
      AND finished_at BETWEEN NEW.created_at_ts-5 AND NEW.created_at_ts+5;
END;
