import { readFileSync } from "node:fs";

const LEGACY_IMPORT_KEY = "legacy_json_import_v1";

function integerOrNull(value) {
  return Number.isSafeInteger(value) ? value : null;
}

function nonNegativeInteger(value, fallback = 0) {
  return Number.isSafeInteger(value) && value >= 0 ? value : fallback;
}

function stringOrNull(value) {
  return typeof value === "string" ? value : null;
}

function normalizeLegacyPublicKey(value) {
  const normalized = String(value || "").trim().toLowerCase();
  if (!/^[0-9a-f]{128}$/.test(normalized)) {
    throw new Error("nni_public_key_whitelist_invalid");
  }
  return normalized;
}

function readLegacyState(legacyStatePath) {
  try {
    return JSON.parse(readFileSync(legacyStatePath, "utf8"));
  } catch (error) {
    if (error?.code === "ENOENT") return null;
    throw error;
  }
}

function importTasks(statements, tasks) {
  for (const task of Object.values(tasks)) {
    if (!task || typeof task !== "object" || !stringOrNull(task.task_id)) continue;
    statements.insertTaskLegacy.run(
      task.task_id,
      stringOrNull(task.user_key) || "anonymous",
      stringOrNull(task.device_pubkey) || "",
      stringOrNull(task.challenge) || "",
      stringOrNull(task.status) || "pending",
      stringOrNull(task.task_kind) || "nni_join",
      integerOrNull(task.created_at_ts) ?? 0,
      integerOrNull(task.expires_at_ts) ?? 0,
      integerOrNull(task.verified_at_ts),
      stringOrNull(task.error_code),
    );
  }
}

function importDevices(statements, devices) {
  for (const [key, device] of Object.entries(devices)) {
    if (!device || typeof device !== "object") continue;
    const userKey = stringOrNull(device.user_key) || key.split(":", 1)[0] || "anonymous";
    const devicePubkey = stringOrNull(device.device_pubkey) || "";
    if (!devicePubkey) continue;
    const history = Array.isArray(device.heartbeat_timestamps_unix)
      ? device.heartbeat_timestamps_unix.filter(
          (value) => Number.isSafeInteger(value) && value >= 0,
        )
      : null;
    statements.insertDeviceLegacy.run(
      userKey,
      devicePubkey,
      integerOrNull(device.first_joined_at_ts),
      integerOrNull(device.last_compliant_request_ts),
      nonNegativeInteger(device.join_count),
      stringOrNull(device.status) || "heartbeat",
      integerOrNull(device.first_heartbeat_ts) ?? history?.[0] ?? null,
      integerOrNull(device.last_heartbeat_ts) ?? history?.at(-1) ?? null,
      Math.max(nonNegativeInteger(device.heartbeat_count), history?.length || 0),
    );
  }
}

function importRequests(statements, requests) {
  const heartbeatRequestsByDevice = new Map();
  for (const [index, record] of requests.entries()) {
    if (!record || typeof record !== "object") continue;
    const userKey = stringOrNull(record.user_key) || "anonymous";
    const devicePubkey = stringOrNull(record.device_pubkey) || "";
    const createdAt = integerOrNull(record.created_at_ts) ?? 0;
    statements.insertRequestLegacy.run(
      integerOrNull(record.id) ?? index + 1,
      stringOrNull(record.request_kind) || "nni_join",
      stringOrNull(record.task_id),
      userKey,
      devicePubkey,
      stringOrNull(record.challenge),
      stringOrNull(record.signature),
      record.compliant === true ? 1 : 0,
      stringOrNull(record.status) || "unknown",
      stringOrNull(record.error_code),
      createdAt,
    );
    if (
      record.request_kind === "nni_heartbeat" &&
      record.compliant === true &&
      record.status === "accepted" &&
      Number.isSafeInteger(record.created_at_ts)
    ) {
      const key = `${userKey}:${devicePubkey}`;
      const existing = heartbeatRequestsByDevice.get(key) || [];
      existing.push(record);
      heartbeatRequestsByDevice.set(key, existing);
    }
  }
  return heartbeatRequestsByDevice;
}

function importHeartbeatHistory(statements, devices, heartbeatRequestsByDevice) {
  const importedHistoryKeys = new Set();
  for (const device of Object.values(devices)) {
    if (!device || typeof device !== "object" || !Array.isArray(device.heartbeat_timestamps_unix)) {
      continue;
    }
    const userKey = stringOrNull(device.user_key) || "anonymous";
    const devicePubkey = stringOrNull(device.device_pubkey) || "";
    const key = `${userKey}:${devicePubkey}`;
    const history = device.heartbeat_timestamps_unix.filter(
      (value) => Number.isSafeInteger(value) && value >= 0,
    );
    const deviceId = statements.deviceIdentity.get(userKey, devicePubkey).id;
    for (const timestamp of history) statements.insertHeartbeatLegacy.run(deviceId, timestamp);
    importedHistoryKeys.add(key);
  }

  for (const [key, heartbeatRequests] of heartbeatRequestsByDevice.entries()) {
    if (importedHistoryKeys.has(key)) continue;
    for (const record of heartbeatRequests) {
      const userKey = stringOrNull(record.user_key) || "anonymous";
      const devicePubkey = stringOrNull(record.device_pubkey) || "";
      statements.insertDeviceLegacy.run(
        userKey,
        devicePubkey,
        null,
        record.created_at_ts,
        0,
        "heartbeat",
        record.created_at_ts,
        record.created_at_ts,
        heartbeatRequests.length,
      );
      const deviceId = statements.deviceIdentity.get(userKey, devicePubkey).id;
      statements.insertHeartbeatLegacy.run(deviceId, record.created_at_ts);
    }
  }

  for (const aggregate of statements.heartbeatAggregates.all()) {
    statements.upsertLegacyHeartbeatAggregate.run(
      aggregate.user_key,
      aggregate.device_pubkey,
      aggregate.first_heartbeat_ts,
      aggregate.last_heartbeat_ts,
      aggregate.heartbeat_count,
    );
  }
}

export function importLegacyState({ legacyStatePath, statements, transaction }) {
  if (!legacyStatePath || statements.metadata.get(LEGACY_IMPORT_KEY)) return;
  const parsed = readLegacyState(legacyStatePath);
  if (!parsed) return;

  const tasks = parsed.tasks && typeof parsed.tasks === "object" ? parsed.tasks : {};
  const devices = parsed.devices && typeof parsed.devices === "object" ? parsed.devices : {};
  const requests = Array.isArray(parsed.requests) ? parsed.requests : [];

  transaction(() => {
    const whitelist = parsed.public_key_whitelist ?? parsed.public_key_allowlist ?? [];
    if (!Array.isArray(whitelist)) throw new Error("nni_public_key_whitelist_invalid");
    const importedAt = Math.floor(Date.now() / 1000);
    for (const publicKey of whitelist) {
      statements.addPublicKey.run(normalizeLegacyPublicKey(publicKey), importedAt);
    }

    importTasks(statements, tasks);
    importDevices(statements, devices);
    const heartbeatRequestsByDevice = importRequests(statements, requests);
    importHeartbeatHistory(statements, devices, heartbeatRequestsByDevice);
    statements.setMetadata.run(LEGACY_IMPORT_KEY, String(importedAt));
  });
}
