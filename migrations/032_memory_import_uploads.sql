-- Migration id: 032_memory_import_uploads_v1
-- Runtime-owned, principal-private staging. No chunk is canonical memory.
CREATE TABLE memory_import_uploads (
    upload_id TEXT PRIMARY KEY,
    principal_id TEXT NOT NULL REFERENCES principals(principal_id),
    file_digest TEXT NOT NULL,
    total_bytes INTEGER NOT NULL CHECK(total_bytes > 0),
    received_bytes INTEGER NOT NULL DEFAULT 0 CHECK(received_bytes >= 0 AND received_bytes <= total_bytes),
    chunk_bytes INTEGER NOT NULL CHECK(chunk_bytes > 0),
    state TEXT NOT NULL CHECK(state IN ('uploading','parsing','preview','confirmed','cancelled')),
    revision INTEGER NOT NULL CHECK(revision > 0),
    parser_token TEXT,
    parser_lease_until_ts INTEGER,
    preview_import_id TEXT,
    created_at_ts INTEGER NOT NULL,
    updated_at_ts INTEGER NOT NULL,
    CHECK((state = 'parsing' AND parser_token IS NOT NULL AND parser_lease_until_ts IS NOT NULL)
       OR (state != 'parsing' AND parser_token IS NULL AND parser_lease_until_ts IS NULL))
);
CREATE UNIQUE INDEX idx_memory_import_upload_active_file
ON memory_import_uploads(principal_id,file_digest)
WHERE state IN ('uploading','parsing','preview');
CREATE INDEX idx_memory_import_upload_owner ON memory_import_uploads(principal_id,updated_at_ts);
CREATE TABLE memory_import_upload_chunks (
    upload_id TEXT NOT NULL REFERENCES memory_import_uploads(upload_id) ON DELETE CASCADE,
    byte_offset INTEGER NOT NULL CHECK(byte_offset >= 0),
    byte_count INTEGER NOT NULL CHECK(byte_count > 0),
    content_digest TEXT NOT NULL,
    content BLOB,
    PRIMARY KEY(upload_id,byte_offset),
    CHECK(content IS NULL OR LENGTH(content) = byte_count)
);
CREATE TABLE memory_import_upload_items (
    upload_id TEXT NOT NULL REFERENCES memory_import_uploads(upload_id) ON DELETE CASCADE,
    item_index INTEGER NOT NULL CHECK(item_index >= 0),
    item_json TEXT NOT NULL,
    PRIMARY KEY(upload_id,item_index)
);
