import assert from "node:assert/strict";
import test from "node:test";
import { act } from "react-test-renderer";
import { serviceActionsHarness as harness } from "../../test/helpers/service-actions-harness";

const services = ["telegramd", "whatsappd", "whatsapp_webd", "wechatd", "feishud", "larkd"] as const;
const actions = ["start", "stop", "restart"] as const;
for (const service of services) for (const action of actions) test(`${service} ${action} retains its exact request and current feedback`, async context => {
  const h = await harness(context);
  try {
    const request = await h.start(service, action);
    assert.equal(h.pending[request.index].path, `/v1/services/${service}/${action}`);
    assert.equal(h.pending[request.index].init?.method, "POST");
    assert.equal(h.pending[request.index].init?.body, undefined);
    assert.equal(h.current.serviceActionLoading[service], true);
    await h.reply(request.index); await h.advance();
    assert.equal(await request.completion, true);
    assert.equal(h.current.serviceActionMessage?.tone, "success");
    assert.equal(h.current.serviceActionLoading[service], false);
    assert.deepEqual(h.reads, ["device-a:admin:epoch-0:health", "device-a:admin:epoch-0:config"]);
  } finally { await h.dispose(); }
});

for (const scope of ["device-b:admin:epoch-0", "device-a:admin:epoch-1", ""]) {
  for (const failure of [false, true]) test(`obsolete ${failure ? "failure" : "success"} is ignored after scope ${scope || "logout"}`, async context => {
    const h = await harness(context);
    try {
      const request = await h.start();
      await h.update({ requestScope: scope });
      assert.deepEqual(h.current.serviceActionLoading, {});
      if (failure) await h.reject(request.index); else await h.reply(request.index);
      await h.advance();
      assert.equal(await request.completion, false);
      assert.equal(h.current.serviceActionMessage, null);
      assert.deepEqual(h.reads, []);
    } finally { await h.dispose(); }
  });
}

test("role loss suppresses captured callbacks and old results", async context => {
  const h = await harness(context);
  try {
    const captured = h.current, request = await h.start();
    await h.update({ isAdminIdentity: false });
    await h.start("telegramd", "start", captured);
    await h.start("telegramd");
    assert.equal(h.pending.length, 1);
    await h.reply(request.index); await h.advance();
    assert.equal(h.current.serviceActionMessage, null);
    assert.equal(await request.completion, false);
    assert.deepEqual(h.current.serviceActionLoading, {});
  } finally { await h.dispose(); }
});

test("no service mutation starts without authenticated administrative scope", async context => {
  const h = await harness(context);
  try {
    await h.update({ requestScope: "" }); await h.start();
    await h.update({ requestScope: "device-a:user:epoch-0", isAdminIdentity: false }); await h.start();
    assert.equal(h.pending.length, 0);
  } finally { await h.dispose(); }
});

test("A/B/A does not revive a captured action or notice setter", async context => {
  const h = await harness(context);
  try {
    const captured = h.current;
    await h.update({ requestScope: "device-b:admin:epoch-0" });
    await h.update({ requestScope: "device-a:admin:epoch-0" });
    const request = await h.start("wechatd", "start", captured);
    assert.equal(h.pending.length, 0);
    assert.equal(await request.completion, false);
    await act(async () => { captured.setServiceActionMessage({ tone: "success", text: "obsolete-fixture" }); });
    assert.equal(h.pending.length, 0);
    assert.equal(h.current.serviceActionMessage, null);
  } finally { await h.dispose(); }
});

test("captured external notice setters cannot update a new authenticated scope", async context => {
  const h = await harness(context);
  try {
    const captured = h.current.setServiceActionMessage;
    await h.update({ requestScope: "device-b:admin:epoch-0" });
    await act(async () => { captured({ tone: "success", text: "obsolete-fixture" }); });
    assert.equal(h.current.serviceActionMessage, null);
    await act(async () => { h.current.setServiceActionMessage({ tone: "success", text: "current-fixture" }); });
    assert.equal(h.current.serviceActionMessage?.text, "current-fixture");
  } finally { await h.dispose(); }
});

test("same-service duplicate/conflicting actions are suppressed while different services remain independent", async context => {
  const h = await harness(context);
  try {
    const first = await h.start();
    const duplicate = await h.start("wechatd", "restart");
    assert.equal(h.pending.length, 1);
    assert.equal(await duplicate.completion, false);
    const other = await h.start("telegramd", "stop");
    assert.equal(h.pending.length, 2);
    assert.equal(h.current.serviceActionLoading.wechatd, true);
    assert.equal(h.current.serviceActionLoading.telegramd, true);
    await h.reply(first.index); await h.reply(other.index); await h.advance();
    assert.equal(await first.completion, true); assert.equal(await other.completion, true);
    assert.equal(h.pending.length, 2);
  } finally { await h.dispose(); }
});

test("an old endpoint's finally cannot release a new action's busy state", async context => {
  const h = await harness(context);
  try {
    const old = await h.start();
    await h.update({ requestScope: "device-b:admin:epoch-0" });
    const latest = await h.start();
    await h.reject(old.index);
    assert.equal(h.current.serviceActionLoading.wechatd, true);
    assert.equal(h.current.serviceActionMessage, null);
    await h.reply(latest.index); await h.advance();
    assert.equal(await latest.completion, true);
    assert.equal(h.current.serviceActionLoading.wechatd, false);
    assert.deepEqual(h.reads, ["device-b:admin:epoch-0:health", "device-b:admin:epoch-0:config"]);
  } finally { await h.dispose(); }
});

test("ownership is checked after response-body parsing", async context => {
  const h = await harness(context);
  try {
    const request = await h.start();
    let release!: (body: unknown) => void;
    const response = new Response("{}");
    response.json = () => new Promise(resolve => { release = resolve; });
    await act(async () => { h.pending[request.index].resolve(response); });
    await h.update({ requestScope: "device-a:admin:epoch-1" });
    await act(async () => { release({ ok: true, data: {} }); });
    await h.advance();
    assert.equal(await request.completion, false);
    assert.equal(h.current.serviceActionMessage, null);
    assert.deepEqual(h.reads, []);
  } finally { await h.dispose(); }
});

test("scope changes during recovery delay suppress subsequent callbacks", async context => {
  const h = await harness(context);
  try {
    const request = await h.start(); await h.reply(request.index);
    assert.equal(h.current.serviceActionMessage?.tone, "success");
    await h.update({ requestScope: "device-b:admin:epoch-0" }); await h.advance();
    assert.equal(await request.completion, false);
    assert.deepEqual(h.reads, []);
    assert.equal(h.current.serviceActionMessage, null);
  } finally { await h.dispose(); }
});

for (const failure of ["throw", "reject"] as const) test(`accepted work stays successful if a refresh callback ${failure}s`, async context => {
  const h = await harness(context);
  try {
    await h.update({ onHealthRefresh: failure === "throw"
      ? () => { throw new Error("fixture_refresh_failed"); }
      : async () => { throw new Error("fixture_refresh_failed"); } });
    const request = await h.start(); await h.reply(request.index); await h.advance();
    assert.equal(await request.completion, true);
    assert.equal(h.current.serviceActionMessage?.tone, "success");
    assert.equal(h.current.serviceActionLoading.wechatd, false);
    assert.equal(h.pending.length, 1);
  } finally { await h.dispose(); }
});

test("older parallel responses do not replace the latest dispatched action's notice", async context => {
  const h = await harness(context);
  try {
    const old = await h.start("wechatd");
    const latest = await h.start("telegramd");
    await h.reply(latest.index);
    const notice = h.current.serviceActionMessage;
    await h.reject(old.index);
    assert.deepEqual(h.current.serviceActionMessage, notice);
    await h.advance();
    assert.equal(await latest.completion, true);
  } finally { await h.dispose(); }
});

test("current transport and structured failures retain localized feedback without automatic resubmission", async context => {
  const h = await harness(context);
  try {
    for (const failure of ["transport", "response"] as const) {
      const request = await h.start();
      if (failure === "transport") await h.reject(request.index);
      else await h.reply(request.index, 409, { ok: false, error: null, data: { error_code: "service_start_not_running" } });
      assert.equal(await request.completion, false);
      assert.equal(h.current.serviceActionMessage?.tone, "error");
      assert.equal(h.current.serviceActionLoading.wechatd, false);
    }
    assert.equal(h.pending.length, 2); assert.deepEqual(h.reads, []);
  } finally { await h.dispose(); }
});

test("unmount invalidates an acknowledged action before delayed follow-up", async context => {
  const h = await harness(context);
  const request = await h.start(); await h.reply(request.index);
  await h.dispose(); await h.advance();
  assert.equal(await request.completion, false); assert.deepEqual(h.reads, []);
});
