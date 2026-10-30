import assert from "node:assert/strict";
import test from "node:test";
import { wechatLoginHarness, loginStatus, qrStart } from "../../test/helpers/wechat-login-harness";

test("current authenticated status reads preserve the actual response", async (t) => {
  const h = await wechatLoginHarness(t); t.after(() => h.dispose());
  const read = await h.read();
  assert.equal(h.current.wechatLoginLoading, true);
  await h.reply(read.index, loginStatus());
  assert.deepEqual(h.current.wechatLoginStatus, loginStatus());
  assert.equal(h.current.wechatLoginLoading, false);
});

for (const change of [
  { requestScope: "device-b:admin:epoch-1", apiBase: "https://device-b.example.test" },
  { requestScope: "device-a:user:epoch-1" },
  { requestScope: "device-a:admin:epoch-2" },
  { requestScope: "", uiAuthReady: false },
  { apiBase: "https://device-b.example.test" },
]) {
  test(`WeChat clears obsolete login/QR snapshots on ${JSON.stringify(change)}`, async (t) => {
    const h = await wechatLoginHarness(t); t.after(() => h.dispose());
    const start = await h.start(); await h.reply(start.index, qrStart("old"));
    await h.update(change);
    assert.equal(h.current.wechatLoginStatus, null);
    assert.equal(h.current.wechatQrPreviewRequested, false);
    assert.equal(h.current.wechatQrStarting, false);
    assert.equal(h.current.wechatLoginError, null);
  });
  test(`WeChat old status cannot publish after ${JSON.stringify(change)}`, async (t) => {
    const h = await wechatLoginHarness(t); t.after(() => h.dispose());
    const old = await h.read();
    await h.update(change);
    await h.reply(old.index, loginStatus("obsolete"));
    assert.equal(h.current.wechatLoginStatus, null);
    assert.equal(h.current.wechatLoginLoading, false);
  });
  test(`WeChat old QR acknowledgment cannot publish after ${JSON.stringify(change)}`, async (t) => {
    const h = await wechatLoginHarness(t); t.after(() => h.dispose());
    const old = await h.start();
    await h.update(change);
    await h.reply(old.index, qrStart("obsolete"));
    assert.equal(h.current.wechatLoginStatus, null);
    assert.equal(h.current.wechatQrStarting, false);
  });
}

for (const operation of ["read", "start"] as const) {
  test(`WeChat ${operation} rechecks the owner after JSON body resolution`, async (t) => {
    const h = await wechatLoginHarness(t); t.after(() => h.dispose());
    const old = await h[operation](), finish = await h.heldBody(old.index);
    await h.update({ requestScope: "device-b" });
    await finish(operation === "read" ? loginStatus("obsolete") : qrStart("obsolete"));
    assert.equal(h.current.wechatLoginStatus, null);
  });
  test(`WeChat ${operation} captured callbacks cannot dispatch after A/B/A or unmount`, async (t) => {
    const h = await wechatLoginHarness(t); t.after(() => h.dispose());
    const old = h.current;
    await h.update({ requestScope: "device-b" });
    await h.update({ requestScope: "device-a:admin:epoch-0" });
    const stale = await h[operation](true, old);
    assert.equal(h.pending.length, 0);
    await stale.completion;
    const current = h.current;
    await h.dispose();
    const disposed = await h[operation](true, current);
    assert.equal(h.pending.length, 0);
    await disposed.completion;
  });
  test(`WeChat ${operation} rejects unauthenticated callbacks`, async (t) => {
    const h = await wechatLoginHarness(t); t.after(() => h.dispose());
    await h.update({ uiAuthReady: false });
    const operationResult = await h[operation]();
    assert.equal(h.pending.length, 0);
    await operationResult.completion;
  });
  test(`WeChat ${operation} stale error/finally cannot release a new owner's request`, async (t) => {
    const h = await wechatLoginHarness(t); t.after(() => h.dispose());
    const old = await h[operation]();
    await h.update({ requestScope: "device-b" });
    const fresh = await h[operation]();
    assert.equal(h.pending.length, 2);
    await h.reject(old.index);
    assert.equal(h.current.wechatLoginError, null);
    assert.equal(operation === "read" ? h.current.wechatLoginLoading : h.current.wechatQrStarting, true);
    await h.reply(fresh.index, operation === "read" ? loginStatus("current") : qrStart("current"));
    assert.equal(operation === "read" ? h.current.wechatLoginStatus?.account_label : h.current.wechatLoginStatus?.qrcode_url,
      operation === "read" ? "current" : "https://qr.example.test/current");
  });
}

test("WeChat new owner can read immediately while the old request is pending", async (t) => {
  const h = await wechatLoginHarness(t); t.after(() => h.dispose());
  const old = await h.read();
  await h.update({ requestScope: "device-b" });
  const current = await h.read();
  assert.equal(h.pending.length, 2);
  await h.reply(current.index, loginStatus("device-b"));
  await h.reply(old.index, loginStatus("device-a"));
  assert.equal(h.current.wechatLoginStatus?.account_label, "device-b");
});

test("WeChat start is single-flight and preserves the first requested force flag", async (t) => {
  const h = await wechatLoginHarness(t); t.after(() => h.dispose());
  const first = await h.start(false), duplicate = await h.start(true);
  assert.equal(h.pending.length, 1);
  assert.deepEqual(JSON.parse(String(h.pending[0].init?.body)), { force: false });
  await h.reply(first.index, qrStart());
  await first.completion; await duplicate.completion;
  assert.equal(h.current.wechatLoginStatus?.qr_status, "wait");
});

test("WeChat old status cannot overwrite a newer QR session", async (t) => {
  const h = await wechatLoginHarness(t); t.after(() => h.dispose());
  const old = await h.read(), start = await h.start();
  await h.reply(start.index, qrStart("new-session"));
  await h.reply(old.index, { ...loginStatus("old"), qr_ready: true, session_key: "old-session", qrcode_url: "https://qr.example.test/old" });
  assert.equal(h.current.wechatLoginStatus?.qrcode_url, "https://qr.example.test/new-session");
});

test("WeChat ordinary users retain QR sign-in and can explicitly retry a current failure", async (t) => {
  const h = await wechatLoginHarness(t); t.after(() => h.dispose());
  await h.update({ requestScope: "device-a:user:epoch-1" });
  const failed = await h.start();
  await h.reply(failed.index, null, "wechat.login_session_in_use");
  assert.ok(h.current.wechatLoginError?.includes("Another user"));
  assert.equal(h.current.wechatQrStarting, false);
  const retry = await h.start(); await h.reply(retry.index, qrStart("ordinary"));
  assert.equal(h.current.wechatLoginError, null);
  assert.equal(h.current.wechatLoginStatus?.qrcode_url, "https://qr.example.test/ordinary");
});

test("WeChat foreground refresh shares and promotes an existing silent read", async (t) => {
  const h = await wechatLoginHarness(t); t.after(() => h.dispose());
  const silent = await h.read(true), foreground = await h.read(false);
  assert.equal(h.pending.length, 1);
  assert.equal(h.current.wechatLoginLoading, true);
  await h.reject(silent.index);
  await foreground.completion;
  assert.ok(h.current.wechatLoginError);
  assert.equal(h.current.wechatLoginLoading, false);
});

test("WeChat hidden-page polling remains idle and visible polling stays single-flight", async (t) => {
  const h = await wechatLoginHarness(t); t.after(() => h.dispose());
  await h.visibility("hidden");
  await h.update({ statusPollingEnabled: true });
  await h.advance(15000); assert.equal(h.pending.length, 0);
  await h.visibility("visible"); assert.equal(h.pending.length, 1);
  await h.advance(15000); assert.equal(h.pending.length, 1);
  await h.reply(0, loginStatus());
  await h.advance(5000); assert.equal(h.pending.length, 2);
  await h.reply(1, loginStatus());
});

test("WeChat old connected QR poll cannot continue status reads on a new owner", async (t) => {
  const h = await wechatLoginHarness(t); t.after(() => h.dispose());
  const start = await h.start(); await h.reply(start.index, qrStart("old-session"));
  await h.visibility("visible");
  const wait = h.pending.findIndex(row => row.path.endsWith("/wait"));
  const status = h.pending.findIndex(row => row.path.endsWith("/login-status"));
  assert.ok(wait >= 0 && status >= 0);
  assert.deepEqual(JSON.parse(String(h.pending[wait].init?.body)), { session_key: "old-session", timeout_ms: 1500 });
  await h.reply(status, { ...loginStatus(), qr_ready: true, session_key: "old-session" });
  await h.update({ requestScope: "device-b", statusPollingEnabled: false });
  const count = h.pending.length;
  await h.reply(wait, { connected: true });
  assert.equal(h.pending.length, count);
  assert.equal(h.current.wechatLoginStatus, null);
});

test("WeChat expired old QR poll cannot expire a replacement session", async (t) => {
  const h = await wechatLoginHarness(t); t.after(() => h.dispose());
  const first = await h.start(); await h.reply(first.index, qrStart("old-session"));
  await h.visibility("visible");
  const wait = h.pending.findIndex(row => row.path.endsWith("/wait"));
  await h.visibility("hidden");
  const replacement = await h.start(); await h.reply(replacement.index, qrStart("replacement"));
  await h.reply(wait, { qr_status: "expired" });
  assert.equal(h.current.wechatLoginStatus?.qrcode_url, "https://qr.example.test/replacement");
  assert.equal(h.current.wechatLoginStatus?.qr_status, "wait");
});

test("WeChat stale status stays fenced while QR generation is still pending", async (t) => {
  const h = await wechatLoginHarness(t); t.after(() => h.dispose());
  const old = await h.read(), start = await h.start(), during = await h.read();
  assert.equal(h.pending.length, 2); await during.completion;
  await h.reply(old.index, loginStatus("obsolete"));
  assert.equal(h.current.wechatLoginStatus?.qr_status, "generating");
  assert.equal(h.current.wechatQrStarting, true);
  await h.reply(start.index, qrStart("current"));
});

test("WeChat silent read failure does not replace visible foreground feedback", async (t) => {
  const h = await wechatLoginHarness(t); t.after(() => h.dispose());
  const first = await h.read(); await h.reject(first.index);
  const message = h.current.wechatLoginError;
  assert.ok(message);
  const silent = await h.read(true); await h.reply(silent.index, null, "fixture_error");
  assert.equal(h.current.wechatLoginError, message);
  const retry = await h.read(); await h.reply(retry.index, loginStatus());
  assert.equal(h.current.wechatLoginError, null);
});

test("WeChat current QR waiting progress remains visible and polls are single-flight", async (t) => {
  const h = await wechatLoginHarness(t); t.after(() => h.dispose());
  const start = await h.start(); await h.reply(start.index, qrStart());
  await h.visibility("visible");
  const wait = h.pending.findIndex(row => row.path.endsWith("/wait"));
  await h.advance(10000);
  assert.equal(h.pending.filter(row => row.path.endsWith("/wait")).length, 1);
  await h.reply(wait, { qr_status: "scanned", message: "fixture-scanned" });
  assert.equal(h.current.wechatLoginStatus?.qr_status, "scanned");
  assert.equal(h.current.wechatLoginStatus?.qrcode_url, "https://qr.example.test/current");
});

test("WeChat connected poll requests one fresh status read instead of reusing a pre-confirmation read", async (t) => {
  const h = await wechatLoginHarness(t); t.after(() => h.dispose());
  const start = await h.start(); await h.reply(start.index, qrStart());
  await h.visibility("visible");
  const wait = h.pending.findIndex(row => row.path.endsWith("/wait"));
  const oldStatus = h.pending.findIndex(row => row.path.endsWith("/login-status"));
  await h.reply(wait, { connected: true, current_user_bound: true, provider_connected: true, binding_status: "bound" });
  const statuses = h.pending.map((row, index) => ({ row, index })).filter(({ row }) => row.path.endsWith("/login-status"));
  assert.equal(statuses.length, 2);
  await h.reply(statuses[1].index, { connected: true, current_user_bound: true, qr_ready: false, account_label: "bound-current" });
  await h.reply(oldStatus, loginStatus("obsolete"));
  assert.equal(h.current.wechatLoginStatus?.connected, true);
  assert.equal(h.current.wechatLoginStatus?.account_label, "bound-current");
  assert.equal(h.pending.filter(row => row.path.endsWith("/start")).length, 1);
});

test("WeChat confirmed binding remains confirmed if its follow-up status read fails", async (t) => {
  const h = await wechatLoginHarness(t); t.after(() => h.dispose());
  const start = await h.start(); await h.reply(start.index, qrStart());
  await h.visibility("visible");
  const wait = h.pending.findIndex(row => row.path.endsWith("/wait"));
  const oldStatus = h.pending.findIndex(row => row.path.endsWith("/login-status"));
  await h.reply(wait, { connected: true, current_user_bound: true, provider_connected: true, binding_status: "bound" });
  const fresh = h.pending.reduce((found, row, index) => row.path.endsWith("/login-status") ? index : found, -1);
  assert.notEqual(fresh, oldStatus);
  await h.reject(fresh);
  assert.equal(h.current.wechatLoginStatus?.connected, true);
  assert.equal(h.current.wechatLoginStatus?.current_user_bound, true);
  assert.equal(h.current.wechatLoginStatus?.qr_ready, false);
  assert.equal(h.current.wechatLoginStatus?.qrcode_url, null);
  assert.equal(h.current.wechatLoginError, null);
  await h.reply(oldStatus, loginStatus("obsolete"));
  assert.equal(h.current.wechatLoginStatus?.connected, true);
});

for (const code of ["wechat.login_session_expired", "wechat.login_session_not_ready", "wechat.login_session_owner_mismatch", "wechat.qr_wait_unavailable"]) {
  test(`WeChat current poll handles structured error ${code} without replaying QR start`, async (t) => {
    const h = await wechatLoginHarness(t); t.after(() => h.dispose());
    const start = await h.start(); await h.reply(start.index, qrStart());
    await h.visibility("visible");
    const wait = h.pending.findIndex(row => row.path.endsWith("/wait"));
    await h.reply(wait, null, code);
    assert.ok(h.current.wechatLoginError);
    assert.equal(h.pending.filter(row => row.path.endsWith("/start")).length, 1);
  });
}

test("WeChat QR poll rechecks scope after response-body resolution", async (t) => {
  const h = await wechatLoginHarness(t); t.after(() => h.dispose());
  const start = await h.start(); await h.reply(start.index, qrStart());
  await h.visibility("visible");
  const wait = h.pending.findIndex(row => row.path.endsWith("/wait"));
  const finish = await h.heldBody(wait);
  await h.visibility("hidden");
  await h.update({ requestScope: "device-b" });
  const before = h.pending.length;
  await finish({ connected: true });
  assert.equal(h.pending.length, before);
  assert.equal(h.current.wechatLoginStatus, null);
});

test("WeChat status read pending across A/B/A cannot replace a new current snapshot", async (t) => {
  const h = await wechatLoginHarness(t); t.after(() => h.dispose());
  const old = await h.read();
  await h.update({ requestScope: "device-b" }); await h.update({ requestScope: "device-a:admin:epoch-0" });
  const fresh = await h.read(); assert.equal(h.pending.length, 2);
  await h.reply(fresh.index, loginStatus("fresh-a")); await h.reply(old.index, loginStatus("obsolete-a"));
  assert.equal(h.current.wechatLoginStatus?.account_label, "fresh-a");
});
