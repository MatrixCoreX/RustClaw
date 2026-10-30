import assert from "node:assert/strict";
import test from "node:test";
import { channelBindHarness, bindSession } from "../../test/helpers/channel-bind-harness";

for (const platform of ["feishu", "lark"] as const) {
  test(`${platform} current begin preserves its exact request and QR result`, async (t) => {
    const h = await channelBindHarness(t, platform); t.after(() => h.dispose());
    const begin = await h.begin();
    assert.equal(h.pending[begin.index].path, `/v1/admin/channel-binds/${platform}/start`);
    assert.equal(h.pending[begin.index].init?.method, "POST");
    assert.deepEqual(JSON.parse(String(h.pending[begin.index].init?.body)), {});
    await h.reply(begin.index, bindSession(platform));
    assert.deepEqual(h.current.bindSession, bindSession(platform));
    assert.equal(h.current.bindLoading, false);
    assert.equal(h.encoders.length, 1);
    await h.qr(0); assert.equal(h.current.bindQrDataUrl, "data:image/png;base64,fixture-only");
  });
  for (const change of [{ requestScope: "device-b:admin:epoch-1" }, { requestScope: "device-a:admin:epoch-2" },
    { isAdminIdentity: false }, { uiAuthReady: false }, { platform: platform === "feishu" ? "lark" as const : "feishu" as const }]) {
    test(`${platform} clears snapshots on ${JSON.stringify(change)}`, async (t) => {
      const h = await channelBindHarness(t, platform); t.after(() => h.dispose());
      await h.load(); await h.qr(0); await h.update(change);
      assert.equal(h.current.bindSession, null); assert.equal(h.current.bindQrDataUrl, null);
      assert.equal(h.current.bindLoading, false); assert.equal(h.current.bindError, null);
      await h.advance(); assert.equal(h.pending.length, 1);
    });
    test(`${platform} old begin cannot publish after ${JSON.stringify(change)}`, async (t) => {
      const h = await channelBindHarness(t, platform); t.after(() => h.dispose());
      const begin = await h.begin(); await h.update(change); await h.reply(begin.index, bindSession(platform));
      assert.equal(h.current.bindSession, null); assert.equal(h.encoders.length, 0);
    });
    test(`${platform} old bound poll cannot publish or refresh after ${JSON.stringify(change)}`, async (t) => {
      const h = await channelBindHarness(t, platform); t.after(() => h.dispose());
      await h.load(); await h.advance(); assert.equal(h.pending.length, 2);
      await h.update(change); await h.reply(1, bindSession(platform, 1, "bound"));
      assert.equal(h.current.bindSession, null); assert.deepEqual(h.reads, []);
    });
  }
  test(`${platform} captured begin cannot dispatch after A/B/A or unmount`, async (t) => {
    const h = await channelBindHarness(t, platform); t.after(() => h.dispose());
    const old = h.current; await h.update({ requestScope: "device-b" }); await h.update({ requestScope: "device-a:admin:epoch-0" });
    const stale = await h.begin(old); assert.equal(h.pending.length, 0); await stale.completion;
    const current = h.current; await h.dispose();
    const disposed = await h.begin(current); assert.equal(h.pending.length, 0); await disposed.completion;
  });
  test(`${platform} ordinary users and unauthenticated callbacks cannot begin binds`, async (t) => {
    const h = await channelBindHarness(t, platform); t.after(() => h.dispose());
    await h.update({ isAdminIdentity: false }); const ordinary = await h.begin(); assert.equal(h.pending.length, 0); await ordinary.completion;
    await h.update({ isAdminIdentity: true, uiAuthReady: false }); const absent = await h.begin(); assert.equal(h.pending.length, 0); await absent.completion;
  });
  test(`${platform} duplicate begins share a single POST`, async (t) => {
    const h = await channelBindHarness(t, platform); t.after(() => h.dispose());
    const first = await h.begin(), duplicate = await h.begin(); assert.equal(h.pending.length, 1);
    await h.reply(first.index, bindSession(platform)); await first.completion; await duplicate.completion;
    assert.equal(h.encoders.length, 1);
  });
  test(`${platform} stale begin error/finally cannot release a new owner's request`, async (t) => {
    const h = await channelBindHarness(t, platform); t.after(() => h.dispose());
    const old = await h.begin(); await h.update({ requestScope: "device-b" }); const fresh = await h.begin();
    await h.reject(old.index); assert.equal(h.current.bindLoading, true); assert.equal(h.current.bindError, null);
    await h.reply(fresh.index, bindSession(platform, 2)); assert.equal(h.current.bindSession?.session_id, 2);
  });
  test(`${platform} rechecks the begin owner after JSON body resolution`, async (t) => {
    const h = await channelBindHarness(t, platform); t.after(() => h.dispose());
    const begin = await h.begin(), finish = await h.heldBody(begin.index);
    await h.update({ requestScope: "device-b" }); await finish(bindSession(platform));
    assert.equal(h.current.bindSession, null); assert.equal(h.encoders.length, 0);
  });
  test(`${platform} new begin fences an older session poll`, async (t) => {
    const h = await channelBindHarness(t, platform); t.after(() => h.dispose());
    await h.load(); await h.advance(); const replacement = await h.begin();
    await h.reply(replacement.index, bindSession(platform, 2)); await h.reply(1, bindSession(platform, 1, "bound"));
    assert.equal(h.current.bindSession?.session_id, 2); assert.deepEqual(h.reads, []);
  });
  test(`${platform} delayed QR encoders cannot cross owners even for the same entry URL`, async (t) => {
    const h = await channelBindHarness(t, platform); t.after(() => h.dispose());
    await h.load(); await h.update({ requestScope: "device-b" }); await h.load();
    assert.equal(h.encoders.length, 2);
    await h.qr(0, "obsolete-qr"); assert.equal(h.current.bindQrDataUrl, null);
    await h.qr(1, "current-qr"); assert.equal(h.current.bindQrDataUrl, "current-qr");
  });
  test(`${platform} the same session's new status object does not lose an in-flight QR encoder`, async (t) => {
    const h = await channelBindHarness(t, platform); t.after(() => h.dispose());
    await h.load(); await h.advance(); await h.reply(1, bindSession(platform, 1, "detected"));
    await h.qr(0, "current-qr"); assert.equal(h.current.bindQrDataUrl, "current-qr");
  });
  test(`${platform} accepted binding attempts both refreshes even when config refresh fails`, async (t) => {
    const h = await channelBindHarness(t, platform); t.after(() => h.dispose());
    h.failRefresh(); await h.load(); await h.advance(); await h.reply(1, bindSession(platform, 1, "bound"));
    assert.equal(h.current.bindSession?.status, "bound");
    assert.deepEqual(h.reads.map(row => row.kind), ["config", "health"]); assert.equal(h.current.bindError, null);
    await h.advance(10000); assert.equal(h.pending.length, 2);
  });
  test(`${platform} scope changes during config refresh prevent the old health continuation`, async (t) => {
    const h = await channelBindHarness(t, platform); t.after(() => h.dispose());
    const release = h.pauseConfig(); await h.load(); await h.advance(); await h.reply(1, bindSession(platform, 1, "bound"));
    assert.equal(h.reads.length, 1); await h.update({ requestScope: "device-b" }); await release();
    assert.deepEqual(h.reads.map(row => row.kind), ["config"]);
  });
  test(`${platform} wrong-channel and wrong-session poll responses cannot replace the current session`, async (t) => {
    const h = await channelBindHarness(t, platform); t.after(() => h.dispose());
    await h.load(); await h.advance(); await h.reply(1, bindSession(platform === "feishu" ? "lark" : "feishu", 1, "bound"));
    assert.equal(h.current.bindSession?.channel, platform); assert.deepEqual(h.reads, []);
    const current = await h.begin(); await h.reply(current.index, bindSession(platform)); await h.advance();
    await h.reply(h.pending.length - 1, bindSession(platform, 99, "bound"));
    assert.equal(h.current.bindSession?.session_id, 1); assert.deepEqual(h.reads, []);
  });
  test(`${platform} a failed explicit begin preserves the last session and its polling`, async (t) => {
    const h = await channelBindHarness(t, platform); t.after(() => h.dispose());
    await h.load(); const failed = await h.begin(); await h.reject(failed.index);
    assert.equal(h.current.bindSession?.session_id, 1); assert.ok(h.current.bindError); assert.equal(h.current.bindLoading, false);
    await h.advance(); assert.equal(h.pending[h.pending.length - 1].path, `/v1/admin/channel-binds/${platform}/1`);
  });
  for (const terminal of ["bound", "failed", "expired"]) test(`${platform} terminal ${terminal} stops polling`, async (t) => {
    const h = await channelBindHarness(t, platform); t.after(() => h.dispose());
    await h.load(1, terminal); await h.advance(10000); assert.equal(h.pending.length, 1);
  });
  test(`${platform} pending polls do not overlap and preserve their normal cadence`, async (t) => {
    const h = await channelBindHarness(t, platform); t.after(() => h.dispose());
    await h.load(); await h.advance(); await h.advance(10000); assert.equal(h.pending.length, 2);
    await h.reply(1, bindSession(platform)); await h.advance(999); assert.equal(h.pending.length, 2);
    await h.advance(1); assert.equal(h.pending.length, 3);
  });
  test(`${platform} delayed poll JSON cannot cross an owner boundary`, async (t) => {
    const h = await channelBindHarness(t, platform); t.after(() => h.dispose());
    await h.load(); await h.advance(); const finish = await h.heldBody(1);
    await h.update({ requestScope: "device-b" }); await finish(bindSession(platform, 1, "bound"));
    assert.equal(h.current.bindSession, null); assert.deepEqual(h.reads, []);
  });
  test(`${platform} stale poll failures cannot clear a new pending begin`, async (t) => {
    const h = await channelBindHarness(t, platform); t.after(() => h.dispose());
    await h.load(); await h.advance(); const begin = await h.begin(); await h.reject(1);
    assert.equal(h.current.bindLoading, true); assert.equal(h.current.bindError, null);
    await h.reply(begin.index, bindSession(platform, 2)); assert.equal(h.current.bindSession?.session_id, 2);
  });
  test(`${platform} an old encoder error cannot clear a newer session's QR`, async (t) => {
    const h = await channelBindHarness(t, platform); t.after(() => h.dispose());
    await h.load(); await h.load(2); await h.qr(1, "current-qr"); await h.qrError(0);
    assert.equal(h.current.bindQrDataUrl, "current-qr"); assert.equal(h.current.bindError, null);
  });
  test(`${platform} invalid begin identity retains the previous session`, async (t) => {
    const h = await channelBindHarness(t, platform); t.after(() => h.dispose());
    await h.load();
    for (const id of [0, -1, 1.5, Number.MAX_SAFE_INTEGER + 1]) {
      const begin = await h.begin(); await h.reply(begin.index, bindSession(platform, id));
      assert.equal(h.current.bindSession?.session_id, 1); assert.ok(h.current.bindError);
    }
  });
  test(`${platform} accepted binding survives both secondary refresh failures`, async (t) => {
    const h = await channelBindHarness(t, platform); t.after(() => h.dispose());
    h.failRefresh(true, true); await h.load(); await h.advance(); await h.reply(1, bindSession(platform, 1, "bound"));
    assert.equal(h.current.bindSession?.status, "bound"); assert.equal(h.current.bindError, null);
    assert.deepEqual(h.reads.map(row => row.kind), ["config", "health"]);
  });
  test(`${platform} a secondary refresh cannot continue after an explicit replacement begins`, async (t) => {
    const h = await channelBindHarness(t, platform); t.after(() => h.dispose());
    const release = h.pauseConfig(); await h.load(); await h.advance(); await h.reply(1, bindSession(platform, 1, "bound"));
    const begin = await h.begin(); await release(); assert.deepEqual(h.reads.map(row => row.kind), ["config"]);
    await h.reply(begin.index, bindSession(platform, 2)); assert.equal(h.current.bindSession?.session_id, 2);
  });
}
