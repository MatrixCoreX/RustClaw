import { DatabaseSync } from "node:sqlite";
import { chmodSync, mkdirSync } from "node:fs";
import path from "node:path";
import { importLegacyState } from "./legacy_migration.mjs";

const SCHEMA_VERSION = 1;

export class NniStore {
  constructor({ databasePath, legacyStatePath, configuredPublicKeys = [] }) {
    this.databasePath = path.resolve(databasePath);
    this.legacyStatePath = legacyStatePath ? path.resolve(legacyStatePath) : null;
    mkdirSync(path.dirname(this.databasePath), { recursive: true });
    this.database = new DatabaseSync(this.databasePath);
    chmodSync(this.databasePath, 0o600);
    this.initializeSchema();
    this.prepareStatements();
    importLegacyState({
      legacyStatePath: this.legacyStatePath,
      statements: this.statements,
      transaction: (operation) => this.transaction(operation),
    });
    this.addPublicKeys(configuredPublicKeys);
  }

  initializeSchema() {
    this.database.exec(`
      PRAGMA journal_mode = WAL;
      PRAGMA synchronous = FULL;
      PRAGMA foreign_keys = ON;
      PRAGMA busy_timeout = 5000;

      CREATE TABLE IF NOT EXISTS metadata (
        key TEXT PRIMARY KEY,
        value TEXT NOT NULL
      );

      CREATE TABLE IF NOT EXISTS public_key_whitelist (
        device_pubkey TEXT PRIMARY KEY,
        created_at_unix INTEGER NOT NULL
      );

      CREATE TABLE IF NOT EXISTS tasks (
        task_id TEXT PRIMARY KEY,
        user_key TEXT NOT NULL,
        device_pubkey TEXT NOT NULL,
        challenge TEXT NOT NULL,
        status TEXT NOT NULL,
        task_kind TEXT NOT NULL,
        created_at_ts INTEGER NOT NULL,
        expires_at_ts INTEGER NOT NULL,
        verified_at_ts INTEGER,
        error_code TEXT
      );

      CREATE INDEX IF NOT EXISTS idx_tasks_join_interval
        ON tasks(task_kind, user_key, device_pubkey, created_at_ts DESC);

      CREATE TABLE IF NOT EXISTS devices (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        user_key TEXT NOT NULL,
        device_pubkey TEXT NOT NULL,
        first_joined_at_ts INTEGER,
        last_compliant_request_ts INTEGER,
        join_count INTEGER NOT NULL DEFAULT 0,
        status TEXT NOT NULL,
        first_heartbeat_ts INTEGER,
        last_heartbeat_ts INTEGER,
        heartbeat_count INTEGER NOT NULL DEFAULT 0,
        UNIQUE(user_key, device_pubkey)
      );

      CREATE INDEX IF NOT EXISTS idx_devices_public_key
        ON devices(device_pubkey);

      CREATE TABLE IF NOT EXISTS request_records (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        request_kind TEXT NOT NULL,
        task_id TEXT,
        user_key TEXT NOT NULL,
        device_pubkey TEXT NOT NULL,
        challenge TEXT,
        signature TEXT,
        compliant INTEGER NOT NULL,
        status TEXT NOT NULL,
        error_code TEXT,
        created_at_ts INTEGER NOT NULL
      );

      CREATE INDEX IF NOT EXISTS idx_request_records_device_time
        ON request_records(device_pubkey, created_at_ts DESC);

      CREATE INDEX IF NOT EXISTS idx_request_records_kind_time
        ON request_records(request_kind, created_at_ts DESC);

      CREATE TABLE IF NOT EXISTS heartbeat_records (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        device_id INTEGER NOT NULL,
        heartbeat_at_unix INTEGER NOT NULL,
        FOREIGN KEY(device_id) REFERENCES devices(id) ON DELETE RESTRICT
      );

      CREATE INDEX IF NOT EXISTS idx_heartbeat_records_device_time
        ON heartbeat_records(device_id, heartbeat_at_unix DESC, id DESC);
    `);
    this.database
      .prepare("INSERT OR REPLACE INTO metadata(key, value) VALUES ('schema_version', ?)")
      .run(String(SCHEMA_VERSION));
  }

  prepareStatements() {
    this.statements = {
      metadata: this.database.prepare("SELECT value FROM metadata WHERE key = ?"),
      setMetadata: this.database.prepare("INSERT OR REPLACE INTO metadata(key, value) VALUES (?, ?)"),
      addPublicKey: this.database.prepare(
        "INSERT OR IGNORE INTO public_key_whitelist(device_pubkey, created_at_unix) VALUES (?, ?)",
      ),
      whitelistCount: this.database.prepare("SELECT COUNT(*) AS count FROM public_key_whitelist"),
      publicKeyAllowed: this.database.prepare(
        "SELECT 1 AS allowed FROM public_key_whitelist WHERE device_pubkey = ?",
      ),
      latestJoinTaskTs: this.database.prepare(`
        SELECT MAX(created_at_ts) AS latest_ts
        FROM tasks
        WHERE task_kind = 'nni_join' AND user_key = ? AND device_pubkey = ?
      `),
      insertTask: this.database.prepare(`
        INSERT INTO tasks(
          task_id, user_key, device_pubkey, challenge, status, task_kind,
          created_at_ts, expires_at_ts, verified_at_ts, error_code
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
      `),
      insertTaskLegacy: this.database.prepare(`
        INSERT OR IGNORE INTO tasks(
          task_id, user_key, device_pubkey, challenge, status, task_kind,
          created_at_ts, expires_at_ts, verified_at_ts, error_code
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
      `),
      getTask: this.database.prepare("SELECT * FROM tasks WHERE task_id = ?"),
      updateTask: this.database.prepare(`
        UPDATE tasks SET status = ?, verified_at_ts = ?, error_code = ? WHERE task_id = ?
      `),
      insertRequest: this.database.prepare(`
        INSERT INTO request_records(
          request_kind, task_id, user_key, device_pubkey, challenge, signature,
          compliant, status, error_code, created_at_ts
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
      `),
      insertRequestLegacy: this.database.prepare(`
        INSERT OR IGNORE INTO request_records(
          id, request_kind, task_id, user_key, device_pubkey, challenge, signature,
          compliant, status, error_code, created_at_ts
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
      `),
      upsertJoinDevice: this.database.prepare(`
        INSERT INTO devices(
          user_key, device_pubkey, first_joined_at_ts, last_compliant_request_ts,
          join_count, status, heartbeat_count
        ) VALUES (?, ?, ?, ?, 1, 'joined', 0)
        ON CONFLICT(user_key, device_pubkey) DO UPDATE SET
          first_joined_at_ts = COALESCE(devices.first_joined_at_ts, excluded.first_joined_at_ts),
          last_compliant_request_ts = excluded.last_compliant_request_ts,
          join_count = devices.join_count + 1,
          status = 'joined'
      `),
      upsertHeartbeatDevice: this.database.prepare(`
        INSERT INTO devices(
          user_key, device_pubkey, last_compliant_request_ts, status,
          first_heartbeat_ts, last_heartbeat_ts, heartbeat_count, join_count
        ) VALUES (?, ?, ?, 'heartbeat', ?, ?, 1, 0)
        ON CONFLICT(user_key, device_pubkey) DO UPDATE SET
          last_compliant_request_ts = excluded.last_compliant_request_ts,
          first_heartbeat_ts = COALESCE(devices.first_heartbeat_ts, excluded.first_heartbeat_ts),
          last_heartbeat_ts = excluded.last_heartbeat_ts,
          heartbeat_count = devices.heartbeat_count + 1
      `),
      insertHeartbeat: this.database.prepare(`
        INSERT INTO heartbeat_records(device_id, heartbeat_at_unix) VALUES (?, ?)
      `),
      insertHeartbeatLegacy: this.database.prepare(`
        INSERT INTO heartbeat_records(device_id, heartbeat_at_unix) VALUES (?, ?)
      `),
      deviceIdentity: this.database.prepare(`
        SELECT id, heartbeat_count FROM devices WHERE user_key = ? AND device_pubkey = ?
      `),
      insertDeviceLegacy: this.database.prepare(`
        INSERT OR IGNORE INTO devices(
          user_key, device_pubkey, first_joined_at_ts, last_compliant_request_ts,
          join_count, status, first_heartbeat_ts, last_heartbeat_ts, heartbeat_count
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
      `),
      heartbeatAggregates: this.database.prepare(`
        SELECT
          devices.user_key,
          devices.device_pubkey,
          MIN(heartbeat_at_unix) AS first_heartbeat_ts,
          MAX(heartbeat_at_unix) AS last_heartbeat_ts,
          COUNT(*) AS heartbeat_count
        FROM heartbeat_records
        JOIN devices ON devices.id = heartbeat_records.device_id
        GROUP BY heartbeat_records.device_id
      `),
      upsertLegacyHeartbeatAggregate: this.database.prepare(`
        INSERT INTO devices(
          user_key, device_pubkey, status, first_heartbeat_ts,
          last_heartbeat_ts, heartbeat_count, join_count
        ) VALUES (?, ?, 'heartbeat', ?, ?, ?, 0)
        ON CONFLICT(user_key, device_pubkey) DO UPDATE SET
          first_heartbeat_ts = COALESCE(devices.first_heartbeat_ts, excluded.first_heartbeat_ts),
          last_heartbeat_ts = CASE
            WHEN devices.last_heartbeat_ts IS NULL THEN excluded.last_heartbeat_ts
            WHEN excluded.last_heartbeat_ts > devices.last_heartbeat_ts THEN excluded.last_heartbeat_ts
            ELSE devices.last_heartbeat_ts
          END,
          heartbeat_count = MAX(devices.heartbeat_count, excluded.heartbeat_count)
      `),
    };
  }

  transaction(operation) {
    this.database.exec("BEGIN IMMEDIATE");
    try {
      const result = operation();
      this.database.exec("COMMIT");
      return result;
    } catch (error) {
      this.database.exec("ROLLBACK");
      throw error;
    }
  }

  addPublicKeys(publicKeys) {
    if (!publicKeys.length) return;
    const ts = Math.floor(Date.now() / 1000);
    this.transaction(() => {
      for (const publicKey of publicKeys) this.statements.addPublicKey.run(publicKey, ts);
    });
  }

  publicKeyWhitelistCount() {
    return this.statements.whitelistCount.get().count;
  }

  isPublicKeyAllowed(devicePubkey) {
    return Boolean(this.statements.publicKeyAllowed.get(devicePubkey));
  }

  latestJoinTaskTs(userKey, devicePubkey) {
    return this.statements.latestJoinTaskTs.get(userKey, devicePubkey).latest_ts ?? null;
  }

  createTask(task) {
    this.statements.insertTask.run(
      task.task_id,
      task.user_key,
      task.device_pubkey,
      task.challenge,
      task.status,
      task.task_kind,
      task.created_at_ts,
      task.expires_at_ts,
      task.verified_at_ts,
      task.error_code,
    );
  }

  getTask(taskId) {
    return this.statements.getTask.get(taskId) || null;
  }

  recordRequest(record) {
    this.statements.insertRequest.run(
      record.request_kind,
      record.task_id,
      record.user_key,
      record.device_pubkey,
      record.challenge,
      record.signature,
      record.compliant ? 1 : 0,
      record.status,
      record.error_code,
      record.created_at_ts,
    );
  }

  finishTaskWithRequest(task, signature, ts, taskStatus, compliant, status, errorCode) {
    this.transaction(() => {
      this.statements.updateTask.run(taskStatus, null, errorCode, task.task_id);
      this.recordRequest({
        request_kind: task.task_kind || "nni_join",
        task_id: task.task_id,
        user_key: task.user_key,
        device_pubkey: task.device_pubkey,
        challenge: task.challenge,
        signature,
        compliant,
        status,
        error_code: errorCode,
        created_at_ts: ts,
      });
    });
  }

  acceptJoin(task, signature, ts) {
    this.transaction(() => {
      this.statements.updateTask.run("verified", ts, null, task.task_id);
      this.statements.upsertJoinDevice.run(task.user_key, task.device_pubkey, ts, ts);
      this.recordRequest({
        request_kind: "nni_join",
        task_id: task.task_id,
        user_key: task.user_key,
        device_pubkey: task.device_pubkey,
        challenge: task.challenge,
        signature,
        compliant: true,
        status: "accepted",
        error_code: null,
        created_at_ts: ts,
      });
    });
  }

  acceptHeartbeat(task, signature, ts) {
    return this.transaction(() => {
      this.statements.updateTask.run("verified", ts, null, task.task_id);
      this.statements.upsertHeartbeatDevice.run(
        task.user_key,
        task.device_pubkey,
        ts,
        ts,
        ts,
      );
      const device = this.statements.deviceIdentity.get(task.user_key, task.device_pubkey);
      this.statements.insertHeartbeat.run(device.id, ts);
      this.recordRequest({
        request_kind: "nni_heartbeat",
        task_id: task.task_id,
        user_key: task.user_key,
        device_pubkey: task.device_pubkey,
        challenge: task.challenge,
        signature,
        compliant: true,
        status: "accepted",
        error_code: null,
        created_at_ts: ts,
      });
      return device.heartbeat_count;
    });
  }

  close() {
    this.database.close();
  }
}
