-- A projection of explicit runtime configuration, not a permission grant.
-- No row/default is installed until the startup owner supplies configuration.
CREATE TABLE memory_storage_budget (
    singleton_id INTEGER PRIMARY KEY CHECK(singleton_id = 1),
    configured_limit_bytes TEXT NOT NULL,
    pressure_revision INTEGER NOT NULL CHECK(pressure_revision > 0)
);
