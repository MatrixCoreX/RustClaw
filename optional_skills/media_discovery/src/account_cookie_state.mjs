import fs from "node:fs/promises";
import path from "node:path";

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

export function cookieStateFromRows(platform, rows, now = Date.now()) {
  const [domain, names] = ACCOUNT_COOKIES[platform];
  const current = BigInt(Math.trunc(now)) * 1000n + 11644473600000000n;
  let expired = false;
  for (const row of rows) {
    const host = String(row.host_key).replace(/^\./u, "");
    if (!(host === domain || host.endsWith(`.${domain}`)) || !names.includes(row.name)
      || !(Number(row.plain_length) > 0 || Number(row.encrypted_length) > 0)) continue;
    const expiry = BigInt(row.expires_utc);
    if (expiry === 0n || expiry > current) return "saved_session";
    expired = true;
  }
  return expired ? "expired" : "unknown";
}

export async function cookieState(profile, platform) {
  const Database = await databaseReader();
  if (!Database) return "unknown";
  let expired = false;
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
        length(value) AS plain_length,length(encrypted_value) AS encrypted_length
        FROM cookies WHERE name IN (${names.map(() => "?").join(",")})`).all(...names);
      const state = cookieStateFromRows(platform, rows);
      if (state === "saved_session") return state;
      expired ||= state === "expired";
    } catch (error) {
      if (error.message === "account_storage_invalid") throw error;
      // Missing, locked, corrupt, or unsupported stores are not login proof.
    } finally {
      database?.close();
    }
  }
  return expired ? "expired" : "unknown";
}
