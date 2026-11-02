import assert from "node:assert/strict";
import { EventEmitter } from "node:events";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { accountStatus, logoutAccount, writeAccount } from "../src/account_sessions.mjs";
import { observeAccount, selfAccountName } from "../src/account_observation.mjs";
import { beginRun, commitPageRecords, readRecords } from "../src/storage.mjs";
import { cookieStateFromRows } from "../src/account_cookie_state.mjs";

const { DatabaseSync } = await import("node:sqlite").catch(() => ({}));

async function fixture(t) {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "discovery-account-fixture-"));
  t.after(() => fs.rm(root, { recursive: true, force: true }));
  return root;
}

test("unknown and saved sessions do not fabricate a signed-in account", async t => {
  const root = await fixture(t);
  assert.equal((await accountStatus(root, ["weibo"]))[0].status, "unknown");
  await writeAccount(root, { platform: "weibo", status: "saved_session", display_name: "not verified", cookie: "fixture-private" });
  const rows = await accountStatus(root, ["weibo"]);
  assert.equal(rows[0].status, "unknown", "a saved snapshot without a readable profile is not current evidence");
  assert.equal(rows[0].display_name, null);
  assert.ok(!JSON.stringify(rows).includes("fixture-private"));
  assert.ok(!JSON.stringify(rows).includes(root));
});

test("removed profiles invalidate old verified names instead of keeping stale login claims", async t => {
  const root = await fixture(t);
  await writeAccount(root, { platform: "weibo", status: "verified", display_name: "Historical self", checked_at: 1, session_present: true });
  const row = (await accountStatus(root, ["weibo"]))[0];
  assert.equal(row.status, "unknown");
  assert.equal(row.display_name, null);
  assert.equal(row.session_present, null);
});

test("cookie evidence uses exact domains, credential presence and lossless Chrome timestamps", () => {
  const now = Date.now();
  const current = BigInt(now) * 1000n + 11644473600000000n;
  const row = { host_key: ".weibo.com", name: "SUB", expires_utc: String(current + 3600000000n), plain_length: 0, encrypted_length: 32 };
  assert.equal(cookieStateFromRows("weibo", [row], now), "saved_session");
  assert.equal(cookieStateFromRows("weibo", [{ ...row, expires_utc: String(current - 1n) }], now), "expired");
  assert.equal(cookieStateFromRows("weibo", [{ ...row, expires_utc: "0" }], now), "saved_session");
  assert.equal(cookieStateFromRows("weibo", [{ ...row, host_key: "notweibo.com" }], now), "unknown");
  assert.equal(cookieStateFromRows("weibo", [{ ...row, plain_length: 0, encrypted_length: 0 }], now), "unknown");
});

test("read-only SQLite refresh detects expiry, preserves cookie bytes, and supports Mac profile variants", async t => {
  const root = await fixture(t);
  const directory = path.join(root, "browser-profile", "macos-fixture", "weibo", "Default", "Network");
  await fs.mkdir(directory, { recursive: true });
  const file = path.join(directory, "Cookies");
  if (DatabaseSync) {
    const database = new DatabaseSync(file);
    database.exec("CREATE TABLE cookies(host_key TEXT,name TEXT,expires_utc INTEGER,value TEXT,encrypted_value BLOB)");
    database.prepare("INSERT INTO cookies VALUES (?,?,?,?,?)").run(".weibo.com", "SUB", 1, "fixture-private", Buffer.alloc(0));
    database.close();
  } else {
    // A supported older Node cannot inspect SQLite: it must fail conservatively.
    await fs.writeFile(file, "fixture-unreadable-store");
  }
  const before = await fs.readFile(file);
  await writeAccount(root, { platform: "weibo", status: "verified", display_name: "Historical self", session_present: true });
  const row = (await accountStatus(root, ["weibo"]))[0];
  assert.equal(row.status, DatabaseSync ? "expired" : "unknown");
  assert.equal(row.display_name, null);
  assert.deepEqual(await fs.readFile(file), before);
  assert.ok(!JSON.stringify(row).includes("fixture-private"));
});

test("a corrupt optional account snapshot does not hide other platform rows", async t => {
  const root = await fixture(t);
  await fs.mkdir(path.join(root, "accounts"));
  await fs.writeFile(path.join(root, "accounts", "weibo.json"), "not-json");
  const rows = await accountStatus(root, ["weibo", "douyin"]);
  assert.deepEqual(rows.map(row => row.status), ["unknown", "unknown"]);
});

test("partial logout returns stable mutation evidence and never resurrects the old verified identity", async t => {
  const root = await fixture(t);
  const profile = path.join(root, "browser-profile", "weibo");
  await fs.mkdir(path.join(profile, "Default"), { recursive: true });
  await fs.writeFile(path.join(profile, "Default", "Cookies"), "fixture-private");
  await fs.writeFile(path.join(profile, "other-session-state"), "fixture-private");
  await writeAccount(root, { platform: "weibo", status: "verified", display_name: "Historical self", session_present: true });
  const original = fs.rm;
  let removed = 0;
  const mock = t.mock.method(fs, "rm", async (file, options) => {
    if (file.startsWith(profile + path.sep)) {
      if (removed === 1) throw Object.assign(new Error("fixture-permission-failure"), { code: "EACCES" });
      await original(file, options);
      removed += 1;
      return;
    }
    return original(file, options);
  });
  try {
    const { handleRequest } = await import("../src/main.mjs");
    const result = await handleRequest({ request_id: "partial-logout-fixture", user_id: 1, chat_id: 1,
      args: { action: "account_logout", platform: "weibo", confirm: true },
      context: { skill_storage: { storage_kind: "directory", directory_path: root } } });
    assert.equal(removed, 1);
    assert.equal(result.status, "error");
    assert.equal(result.extra.error_code, "account_operation_failed");
    assert.equal(result.extra.side_effect_applied, true);
    assert.equal(result.extra.failure_phase, "post_dispatch");
    assert.equal(result.extra.retryable, false);
    const row = (await accountStatus(root, ["weibo"]))[0];
    assert.equal(row.status, "unknown");
    assert.equal(row.display_name, null);
  } finally { mock.mock.restore(); }
});

test("logout clears only selected platform profiles and preserves media/history/environment", async t => {
  const root = await fixture(t);
  const profiles = ["browser-profile/weibo", "browser-profile/macos-fixture/weibo"];
  for (const name of [...profiles, "browser-profile/douyin"]) {
    await fs.mkdir(path.join(root, name, "Default"), { recursive: true });
    await fs.writeFile(path.join(root, name, "Default", "Cookies"), "fixture-private");
    await fs.writeFile(path.join(root, name, "browser-environment.json"), "keep");
  }
  await commitPageRecords(root, [{ kind: "image", platform: "weibo", dedup_key: "fixture:1", account_use: [{
    schema_version: 1, platform: "weibo", mode: "authenticated_browser", account: { status: "verified", display_name: "Self account" },
  }] }]);
  assert.equal((await logoutAccount(root, "weibo")).status, "signed_out");
  assert.equal((await logoutAccount(root, "weibo")).status, "signed_out");
  for (const name of profiles) {
    await assert.rejects(fs.stat(path.join(root, name, "Default")), { code: "ENOENT" });
    assert.equal(await fs.readFile(path.join(root, name, "browser-environment.json"), "utf8"), "keep");
  }
  assert.equal(await fs.readFile(path.join(root, "browser-profile/douyin/Default/Cookies"), "utf8"), "fixture-private");
  assert.equal((await readRecords(root))[0].account_use[0].account.display_name, "Self account");
});

test("an active run blocks logout without cancelling it", async t => {
  const root = await fixture(t);
  const { run } = await beginRun(root, ["weibo"], { mode: "one_shot", config_explicit: true, platform_configs: { weibo: {} } });
  assert.ok(run);
  await assert.rejects(logoutAccount(root, "weibo"), /account_session_busy/u);
  assert.equal((await accountStatus(root, ["weibo"]))[0].status, "unknown");
});

test("open Chrome and symlinked profiles are rejected before deleting anything", async t => {
  const root = await fixture(t);
  const profile = path.join(root, "browser-profile/weibo");
  await fs.mkdir(profile, { recursive: true });
  await fs.writeFile(path.join(profile, "keep.txt"), "keep");
  await fs.symlink("fixture-host-123", path.join(profile, "SingletonLock"));
  await assert.rejects(logoutAccount(root, "weibo"), /account_session_busy/u);
  assert.equal(await fs.readFile(path.join(profile, "keep.txt"), "utf8"), "keep");
  await fs.symlink(profile, path.join(root, "browser-profile/douyin"), "dir");
  await assert.rejects(logoutAccount(root, "douyin"), /account_storage_invalid/u);
});

test("account identity is restricted to first-party self endpoints, never content authors", () => {
  assert.equal(selfAccountName("weibo", "https://weibo.com/ajax/statuses/show", { user: { screen_name: "Post author" } }), null);
  assert.equal(selfAccountName("weibo", "https://foreign.test/ajax/config", { data: { user: { screen_name: "Forged" } } }), null);
  assert.equal(selfAccountName("weibo", "not a url", {}), null);
  assert.equal(selfAccountName("weibo", "https://weibo.com/ajax/config", { data: { user: { screen_name: "Self account" } } }), "Self account");
});

test("task use requires actual request cookies; saved cookies alone are insufficient", async t => {
  const root = await fixture(t);
  const context = new EventEmitter();
  context.cookies = async () => [{ domain: ".weibo.com", name: "SUB", value: "fixture-private", expires: -1 }];
  const observer = observeAccount(root, "weibo", context);
  const before = await observer.capture();
  assert.equal(before.mode, "unknown");
  assert.equal(before.account.status, "saved_session");
  context.emit("request", { url: () => "https://weibo.com/ajax/config", allHeaders: async () => ({ cookie: "SUB=fixture-private" }) });
  context.emit("response", { url: () => "https://weibo.com/ajax/config", text: async () => JSON.stringify({ data: { user: { screen_name: "Self account" } } }) });
  const after = await observer.capture();
  assert.equal(after.mode, "authenticated_browser");
  assert.equal(after.account.display_name, "Self account");
  assert.equal(after.account.status, "verified");
  assert.ok(!JSON.stringify(after).includes("fixture-private"));
  observer.close();
  assert.equal(context.listenerCount("request"), 0);
  assert.equal(context.listenerCount("response"), 0);
});

test("a stalled optional account probe cannot hold a media result open", async t => {
  const root = await fixture(t);
  const context = new EventEmitter();
  context.cookies = () => new Promise(() => {});
  const observer = observeAccount(root, "weibo", context);
  const result = await observer.capture();
  assert.equal(result.mode, "unknown");
  assert.equal(result.account.status, "unknown");
  observer.close();
});

test("a login timestamp alone is not proof of attached credentials", async t => {
  const root = await fixture(t);
  const context = new EventEmitter();
  context.cookies = async () => [];
  const observer = observeAccount(root, "weibo", context);
  context.emit("request", { url: () => "https://weibo.com/", allHeaders: async () => ({ cookie: "SSOLoginState=fixture-timestamp" }) });
  assert.equal((await observer.capture()).mode, "unknown");
  context.emit("request", { url: () => "https://weibo.com/", allHeaders: async () => ({ cookie: "SUB=" }) });
  assert.equal((await observer.capture()).mode, "unknown");
  observer.close();
});
