import assert from "node:assert/strict";
import { EventEmitter } from "node:events";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { accountStatus, logoutAccount, writeAccount } from "../src/account_sessions.mjs";
import { observeAccount, selfAccountName } from "../src/account_observation.mjs";
import { beginRun, commitPageRecords, readRecords } from "../src/storage.mjs";

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
  assert.equal(rows[0].status, "saved_session");
  assert.equal(rows[0].display_name, null);
  assert.ok(!JSON.stringify(rows).includes("fixture-private"));
  assert.ok(!JSON.stringify(rows).includes(root));
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
