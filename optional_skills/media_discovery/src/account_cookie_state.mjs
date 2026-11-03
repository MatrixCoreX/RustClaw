import fs from "node:fs/promises";
import path from "node:path";
import { createHash } from "node:crypto";

export const ACCOUNT_COOKIES = Object.freeze({
  douyin: ["douyin.com", ["sessionid", "sessionid_ss", "sid_guard"]],
  xiaohongshu: ["xiaohongshu.com", ["web_session"]],
  kuaishou: ["kuaishou.com", ["kuaishou.server.web_st", "kuaishou.server.webday7_st"]],
  toutiao: ["toutiao.com", ["sessionid", "sid_guard"]],
  weibo: ["weibo.com", ["SUB"]],
  tiktok: ["tiktok.com", ["sessionid", "sessionid_ss", "sid_tt"]],
  youtube: ["youtube.com", ["SID", "SAPISID", "__Secure-3PSID"]],
  bilibili: ["bilibili.com", ["SESSDATA"]],
});

// Node 20 remains supported. Without its newer optional SQLite reader, local
// evidence is unknown, never a reason to keep claiming a verified account.
let sqliteReader;
async function databaseReader() {
  sqliteReader ??= import("node:sqlite").then(module => module.DatabaseSync).catch(() => null);
  return sqliteReader;
}

export function cookieEvidenceFromRows(platform, rows, now = Date.now()) {
  const [domain, names] = ACCOUNT_COOKIES[platform];
  const current = BigInt(Math.trunc(now)) * 1000n + 11644473600000000n;
  let expired = false;
  let saved = false;
  const bindings = [];
  for (const row of rows) {
    const host = String(row.host_key).replace(/^\./u, "");
    if (!(host === domain || host.endsWith(`.${domain}`)) || !names.includes(row.name)
      || !(Number(row.plain_length) > 0 || Number(row.encrypted_length) > 0)) continue;
    const expiry = BigInt(row.expires_utc);
    if (expiry !== 0n && expiry <= current) { expired = true; continue; }
    saved = true;
    if (typeof row.value === "string" || row.encrypted_value instanceof Uint8Array) {
      bindings.push(createHash("sha256").update(JSON.stringify([host, row.name, String(expiry), row.value || ""]))
        .update(row.encrypted_value || Buffer.alloc(0)).digest("hex"));
    }
  }
  return { state: saved ? "saved_session" : expired ? "expired" : "unknown",
    binding: saved && bindings.length ? createHash("sha256").update(JSON.stringify(bindings.sort())).digest("hex") : null };
}

export function cookieStateFromRows(platform, rows, now = Date.now()) {
  return cookieEvidenceFromRows(platform, rows, now).state;
}

export async function cookieEvidence(profile, platform) {
  const Database = await databaseReader();
  if (!Database) return { state: "unknown", binding: null };
  let expired = false;
  const bindings = [];
  for (const relative of ["Default/Network/Cookies", "Default/Cookies"]) {
    const file = path.join(profile, relative);
    let database;
    try {
      // Do not follow links out of the skill-owned profile, including parents.
      for (const part of [path.dirname(file), path.join(profile, "Default"), file]) {
        if ((await fs.lstat(part)).isSymbolicLink()) throw new Error("account_storage_invalid");
      }
      if (!(await fs.stat(file)).isFile()) continue;
      database = new Database(file, { readOnly: true });
      database.exec("PRAGMA query_only=ON; PRAGMA busy_timeout=100");
      const names = ACCOUNT_COOKIES[platform][1];
      const rows = database.prepare(`SELECT host_key,name,CAST(expires_utc AS TEXT) AS expires_utc,
        length(value) AS plain_length,length(encrypted_value) AS encrypted_length,
        CASE WHEN length(value)<=16384 THEN value ELSE NULL END AS value,
        CASE WHEN length(encrypted_value)<=16384 THEN encrypted_value ELSE NULL END AS encrypted_value
        FROM cookies WHERE name IN (${names.map(() => "?").join(",")})`).all(...names);
      const evidence = cookieEvidenceFromRows(platform, rows);
      if (evidence.binding) bindings.push([relative, evidence.binding]);
      expired ||= evidence.state === "expired";
    } catch (error) {
      if (error.message === "account_storage_invalid") throw error;
      // Missing, locked, corrupt, or unsupported stores are not login proof.
    } finally {
      database?.close();
    }
  }
  return { state: bindings.length ? "saved_session" : expired ? "expired" : "unknown",
    binding: bindings.length ? createHash("sha256").update(JSON.stringify(bindings.sort())).digest("hex") : null };
}

export async function cookieState(profile, platform) {
  return (await cookieEvidence(profile, platform)).state;
}
