import assert from "node:assert/strict";
import test from "node:test";
import { whatsappLoginHarness, whatsappStatus } from "../../test/helpers/whatsapp-login-harness";

test("current WhatsApp status preserves its response and bridge reachability", async (t) => {
  const h = await whatsappLoginHarness(t); t.after(() => h.dispose());
  const read = await h.read();
  assert.equal(h.pending[read.index].path, "/v1/whatsapp-web/login-status");
  assert.equal(h.pending[read.index].init?.method, undefined);
  await h.reply(read.index, whatsappStatus());
  assert.deepEqual(h.current.waLoginStatus, whatsappStatus());
  assert.equal(h.current.waWebBridgeReachable, true);
  assert.equal(h.current.waLoginLoading, false);
});

for (const change of [{ requestScope: "device-b:admin:epoch-1", apiBase: "https://device-b.example.test" },
  { requestScope: "device-a:user:epoch-1" }, { requestScope: "device-a:admin:epoch-2" }, { uiAuthReady: false }, { apiBase: "https://device-b.example.test" }]) {
  test(`WhatsApp dialog/status clears on ${JSON.stringify(change)}`, async (t) => {
    const h = await whatsappLoginHarness(t); t.after(() => h.dispose());
    await h.dialog(true); await h.reply(0, whatsappStatus("old"));
    await h.update(change);
    assert.equal(h.current.waLoginDialogOpen, false);
    assert.equal(h.current.waLoginStatus, null);
    assert.equal(h.current.waWebBridgeReachable, false);
    assert.equal(h.current.waLoginError, null);
  });
  test(`WhatsApp late status cannot publish after ${JSON.stringify(change)}`, async (t) => {
    const h = await whatsappLoginHarness(t); t.after(() => h.dispose());
    const old = await h.read(); await h.update(change); await h.reply(old.index, whatsappStatus("obsolete"));
    assert.equal(h.current.waLoginStatus, null);
    assert.equal(h.current.waWebBridgeReachable, false);
    assert.equal(h.current.waLoginLoading, false);
  });
}

for (const operation of ["read", "logout"] as const) {
  test(`WhatsApp ${operation} captured callbacks cannot run after A/B/A or unmount`, async (t) => {
    const h = await whatsappLoginHarness(t); t.after(() => h.dispose());
    const old = h.current;
    await h.update({ requestScope: "device-b" }); await h.update({ requestScope: "device-a:admin:epoch-0" });
    const stale = operation === "read" ? await h.read(false, old) : await h.logout(old);
    assert.equal(h.pending.length, 0); await stale.completion;
    const current = h.current; await h.dispose();
    const disposed = operation === "read" ? await h.read(false, current) : await h.logout(current);
    assert.equal(h.pending.length, 0); await disposed.completion;
  });
  test(`WhatsApp ${operation} rechecks its owner after the JSON body`, async (t) => {
    const h = await whatsappLoginHarness(t); t.after(() => h.dispose());
    const request = operation === "read" ? await h.read() : await h.logout();
    const finish = await h.heldBody(request.index);
    await h.update({ requestScope: "device-b" });
    await finish(operation === "read" ? whatsappStatus("obsolete") : {});
    await h.advance(1200);
    assert.equal(h.pending.length, 1);
    assert.equal(h.current.waLoginStatus, null);
    assert.deepEqual(h.notices, []);
  });
  test(`WhatsApp ${operation} does not dispatch without authentication`, async (t) => {
    const h = await whatsappLoginHarness(t); t.after(() => h.dispose());
    await h.update({ uiAuthReady: false });
    const request = operation === "read" ? await h.read() : await h.logout();
    assert.equal(h.pending.length, 0); await request.completion;
  });
  test(`WhatsApp old ${operation} error cannot release a new owner's busy state`, async (t) => {
    const h = await whatsappLoginHarness(t); t.after(() => h.dispose());
    const old = operation === "read" ? await h.read() : await h.logout();
    await h.update({ requestScope: "device-b" });
    const fresh = operation === "read" ? await h.read() : await h.logout();
    assert.equal(h.pending.length, 2);
    await h.reject(old.index);
    assert.equal(h.current.waLoginError, null);
    assert.equal(operation === "read" ? h.current.waLoginLoading : h.current.waLogoutLoading, true);
    await h.reject(fresh.index);
  });
}

test("WhatsApp new-owner status reads are not blocked by an old pending request", async (t) => {
  const h = await whatsappLoginHarness(t); t.after(() => h.dispose());
  const old = await h.read(); await h.update({ requestScope: "device-b" });
  const fresh = await h.read(); assert.equal(h.pending.length, 2);
  await h.reply(fresh.index, whatsappStatus("current")); await h.reply(old.index, whatsappStatus("obsolete"));
  assert.equal(h.current.waLoginStatus?.qr_data_url, "current");
});

test("WhatsApp captured dialog setters cannot reopen or close a new owner's dialog", async (t) => {
  const h = await whatsappLoginHarness(t); t.after(() => h.dispose());
  const old = h.current;
  await h.update({ requestScope: "device-b" });
  await h.dialog(true, old); assert.equal(h.current.waLoginDialogOpen, false);
  await h.dialog(true); await h.reply(0, whatsappStatus());
  await h.dialog(false, old); assert.equal(h.current.waLoginDialogOpen, true);
  await h.dialog(value => !value); assert.equal(h.current.waLoginDialogOpen, false);
});

test("WhatsApp logout is single-flight, keeps its exact POST and fences old/concurrent reads", async (t) => {
  const h = await whatsappLoginHarness(t); t.after(() => h.dispose());
  const old = await h.read(), first = await h.logout(), duplicate = await h.logout(), during = await h.read();
  assert.equal(h.pending.length, 2);
  assert.equal(h.pending[first.index].path, "/v1/whatsapp-web/logout");
  assert.equal(h.pending[first.index].init?.method, "POST");
  assert.equal(h.pending[first.index].init?.body, undefined);
  await during.completion;
  await h.reply(old.index, whatsappStatus("obsolete"));
  assert.equal(h.current.waLoginStatus, null);
  await h.reply(first.index, {}); await h.advance(1200);
  assert.equal(h.pending.length, 3);
  await h.reply(2, { connected: false, qr_ready: false, phase: "logged_out" });
  await first.completion; await duplicate.completion;
  assert.equal(h.notices.length, 1);
  assert.equal(h.current.waLogoutLoading, false);
});

test("WhatsApp owner changes during the logout settle delay prevent old follow-up reads/notices", async (t) => {
  const h = await whatsappLoginHarness(t); t.after(() => h.dispose());
  const logout = await h.logout(); await h.reply(logout.index, {});
  await h.update({ requestScope: "device-b" }); await h.advance(1200);
  assert.equal(h.pending.length, 1);
  await logout.completion;
  assert.deepEqual(h.notices, []);
  assert.equal(h.current.waLogoutLoading, false);
});

test("WhatsApp accepted logout remains successful when its follow-up status read fails", async (t) => {
  const h = await whatsappLoginHarness(t); t.after(() => h.dispose());
  const logout = await h.logout(); await h.reply(logout.index, {}); await h.advance(1200);
  assert.equal(h.pending.length, 2);
  await h.reply(1, null, "whatsapp_web.login_status_unavailable"); await logout.completion;
  assert.equal(h.notices[0]?.notice?.tone, "success");
  assert.ok(h.current.waLoginError);
  assert.equal(h.current.waLogoutLoading, false);
  assert.equal(h.pending.filter(row => row.init?.method === "POST").length, 1);
});

test("WhatsApp current logout errors stay visible and an explicit retry is allowed", async (t) => {
  const h = await whatsappLoginHarness(t); t.after(() => h.dispose());
  const failed = await h.logout(); await h.reply(failed.index, null, "whatsapp_web.logout_failed");
  assert.ok(h.current.waLoginError); assert.equal(h.current.waLogoutLoading, false);
  assert.deepEqual(h.notices, []);
  const retry = await h.logout(); await h.reply(retry.index, {}); await h.advance(1200);
  await h.reply(2, whatsappStatus()); await retry.completion;
  assert.equal(h.current.waLoginError, null);
  assert.equal(h.notices[0]?.notice?.tone, "success");
});

test("WhatsApp a foreground refresh promotes a silent pending status read", async (t) => {
  const h = await whatsappLoginHarness(t); t.after(() => h.dispose());
  const silent = await h.read(true), foreground = await h.read();
  assert.equal(h.pending.length, 1); assert.equal(h.current.waLoginLoading, true);
  await h.reject(silent.index); await foreground.completion;
  assert.ok(h.current.waLoginError); assert.equal(h.current.waLoginLoading, false);
});

test("WhatsApp ordinary authenticated users retain status and logout access", async (t) => {
  const h = await whatsappLoginHarness(t); t.after(() => h.dispose());
  await h.update({ requestScope: "device-a:user:epoch-2" });
  const read = await h.read(); await h.reply(read.index, whatsappStatus());
  const logout = await h.logout(); await h.reply(logout.index, {}); await h.advance(1200);
  await h.reply(2, whatsappStatus()); await logout.completion;
  assert.equal(h.notices[0]?.notice?.tone, "success");
});

test("WhatsApp visible polling coalesces while hidden pages remain idle", async (t) => {
  const h = await whatsappLoginHarness(t); t.after(() => h.dispose());
  await h.advance(15000); assert.equal(h.pending.length, 0);
  await h.visibility("visible"); assert.equal(h.pending.length, 1);
  await h.advance(15000); assert.equal(h.pending.length, 1);
  await h.reply(0, whatsappStatus()); await h.visibility("hidden");
  await h.advance(10000); assert.equal(h.pending.length, 1);
});

test("WhatsApp a status read predating logout cannot overwrite its fresh post-acknowledgment snapshot", async (t) => {
  const h = await whatsappLoginHarness(t); t.after(() => h.dispose());
  const old = await h.read(), logout = await h.logout();
  await h.reply(logout.index, {}); await h.advance(1200);
  await h.reply(2, { connected: false, qr_ready: false, phase: "logged_out" });
  await h.reply(old.index, { ...whatsappStatus("obsolete"), connected: true });
  await logout.completion;
  assert.equal(h.current.waLoginStatus?.phase, "logged_out");
  assert.equal(h.current.waLoginStatus?.connected, false);
});

test("WhatsApp status failure does not erase an existing snapshot and can be retried", async (t) => {
  const h = await whatsappLoginHarness(t); t.after(() => h.dispose());
  const initial = await h.read(); await h.reply(initial.index, whatsappStatus());
  const failed = await h.read(); await h.reply(failed.index, null, "whatsapp_web.login_status_invalid");
  assert.equal(h.current.waLoginStatus?.qr_data_url, "current");
  assert.equal(h.current.waWebBridgeReachable, false);
  assert.ok(h.current.waLoginError);
  const retry = await h.read(); await h.reply(retry.index, whatsappStatus("retry"));
  assert.equal(h.current.waLoginError, null);
  assert.equal(h.current.waWebBridgeReachable, true);
});

test("WhatsApp silent status failures preserve foreground feedback", async (t) => {
  const h = await whatsappLoginHarness(t); t.after(() => h.dispose());
  const failed = await h.read(); await h.reply(failed.index, null, "whatsapp_web.login_status_invalid");
  const message = h.current.waLoginError;
  const silent = await h.read(true); await h.reject(silent.index);
  assert.equal(h.current.waLoginError, message);
});

test("WhatsApp an obsolete functional panel setter never evaluates its updater", async (t) => {
  const h = await whatsappLoginHarness(t); t.after(() => h.dispose());
  const old = h.current; let calls = 0;
  await h.update({ requestScope: "device-b" });
  await h.dialog(value => { calls++; return !value; }, old);
  assert.equal(calls, 0); assert.equal(h.current.waLoginDialogOpen, false);
});

test("WhatsApp late post-logout refresh cannot overwrite a new owner's status or create another notice", async (t) => {
  const h = await whatsappLoginHarness(t); t.after(() => h.dispose());
  const logout = await h.logout(); await h.reply(logout.index, {}); await h.advance(1200);
  assert.equal(h.notices.length, 1);
  await h.update({ requestScope: "device-b" });
  const read = await h.read(); await h.reply(read.index, whatsappStatus("current-b"));
  await h.reply(1, whatsappStatus("obsolete-a")); await logout.completion;
  assert.equal(h.current.waLoginStatus?.qr_data_url, "current-b");
  assert.equal(h.current.waLogoutLoading, false);
  assert.equal(h.notices.length, 1);
});

test("WhatsApp A/B/A never reactivates an old in-flight status response", async (t) => {
  const h = await whatsappLoginHarness(t); t.after(() => h.dispose());
  const old = await h.read();
  await h.update({ requestScope: "device-b" }); await h.update({ requestScope: "device-a:admin:epoch-0" });
  const current = await h.read(); assert.equal(h.pending.length, 2);
  await h.reply(current.index, whatsappStatus("current-a")); await h.reply(old.index, whatsappStatus("obsolete-a"));
  assert.equal(h.current.waLoginStatus?.qr_data_url, "current-a");
});
