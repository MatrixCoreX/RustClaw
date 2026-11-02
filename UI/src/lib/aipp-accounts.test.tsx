import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { accountRows, accountsFromTask, invokeAccountControl } from "./aipp-accounts";
import { AippAccountsPanel, AippAccountUse } from "../components/AippAccountsPanel";

const response = (data: unknown) => new Response(JSON.stringify({ ok: true, data }));
const snapshot = { schema_version: 1, platform: "weibo", status: "saved_session", display_name: null, checked_at: 123, session_present: true };
const task = (status: string, accounts: unknown = [snapshot]) => response({ status, result_json: { extra: { accounts } } });
const t = (_zh: string, en: string) => en;

test("account rows expose only typed public metadata, never an inferred identity", () => {
  const rows = accountRows([{ ...snapshot, display_name: "Post author", token: "fixture-private" },
    { ...snapshot, status: "verified", display_name: "Forged\nname" }, { ...snapshot, platform: "../outside" }]);
  assert.equal(rows.length, 2);
  assert.equal(rows[0].display_name, null);
  assert.equal(rows[1].display_name, null);
  assert.ok(!JSON.stringify(rows).includes("fixture-private"));
  assert.deepEqual(accountsFromTask({ text: "logged in as Post author" }), []);
  assert.deepEqual(accountsFromTask({ task_journal: { trace: { capability_results: [{ data: { extra: { accounts: [snapshot] } } }] } } }), [snapshot]);
});

test("enqueued is not signed out; only terminal structured success settles controls", async () => {
  let release: ((value: Response) => void) | undefined;
  let settled = false;
  const controller = new AbortController();
  const promise = invokeAccountControl(async (url, init) => {
    if (url.endsWith("account-control")) {
      assert.equal(JSON.parse(String(init?.body)).confirm, true);
      assert.equal(JSON.parse(String(init?.body)).platform, "weibo");
      return response({ task_id: "fixture-task" });
    }
    return new Promise(resolve => { release = resolve; });
  }, "fixture", "logout", controller.signal, "weibo").then(rows => { settled = true; return rows; });
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(settled, false);
  release!(task("succeeded", [{ ...snapshot, status: "signed_out", session_present: false }]));
  assert.equal((await promise)[0].status, "signed_out");
});

test("busy failure is a machine code and does not get mislabeled successful", async () => {
  await assert.rejects(invokeAccountControl(async url => url.endsWith("account-control")
    ? response({ task_id: "fixture-task" })
    : response({ status: "failed", result_json: { task_journal: { trace: { capability_results: [{
      error: { code: "account_session_busy", details: {} },
    }] } } } }), "fixture", "logout", new AbortController().signal, "weibo"), /account_session_busy/u);
});

test("scope abort stops polling without cancelling server-side media tasks", async () => {
  const controller = new AbortController();
  const paths: string[] = [];
  const promise = invokeAccountControl(async url => {
    paths.push(url);
    if (url.endsWith("account-control")) return response({ task_id: "fixture-task" });
    controller.abort(new Error("scope-ended"));
    return task("queued");
  }, "fixture", "status", controller.signal);
  await assert.rejects(promise, /scope-ended/u);
  assert.equal(paths.length, 2);
  assert.ok(paths.every(url => !url.includes("cancel")));
});

test("account panel defaults collapsed and preserves status/logout feedback when toggled", async () => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  let root: ReactTestRenderer | undefined;
  let signedOut = false;
  const controls = { schema_version: 1, status_capability: "fixture.account_status", logout_capability: "fixture.account_logout" };
  const apiFetch = async (url: string, init?: RequestInit) => {
    if (url.endsWith("account-control")) {
      signedOut = JSON.parse(String(init?.body)).operation === "logout";
      return response({ task_id: "fixture-task" });
    }
    return task("succeeded", [{ ...snapshot, status: signedOut ? "signed_out" : "saved_session" }]);
  };
  try {
    await act(async () => { root = create(<AippAccountsPanel skill="fixture" controls={controls} requestScope="fixture" t={t} apiFetch={apiFetch} />); });
    const toggle = root!.root.findByProps({ "aria-expanded": false });
    assert.ok(toggle.children.includes("Platform accounts"));
    const content = () => root!.root.findByProps({ id: toggle.props["aria-controls"] });
    assert.equal(content().props.hidden, true);
    await act(async () => { toggle.props.onClick(); });
    assert.equal(toggle.props["aria-expanded"], true);
    assert.equal(content().props.hidden, false);
    assert.match(JSON.stringify(root!.toJSON()), /Saved session; account unverified/u);
    const logout = root!.root.findAllByType("button").find(node => node.children.includes("Sign out"));
    assert.ok(logout);
    await act(async () => { logout!.props.onClick(); });
    assert.match(JSON.stringify(root!.toJSON()), /Signed out\. Sign in again/u);
    assert.equal(root!.root.findAllByType("button").filter(node => node.children.includes("Sign out")).length, 0);
    await act(async () => { toggle.props.onClick(); });
    assert.equal(content().props.hidden, true);
    await act(async () => { toggle.props.onClick(); });
    assert.equal(content().props.hidden, false);
    assert.match(JSON.stringify(root!.toJSON()), /Signed out\. Sign in again/u);
  } finally { await act(async () => root?.unmount()); }
});

test("account panels expand independently without restarting or aborting pending status checks", async () => {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  let root: ReactTestRenderer | undefined;
  const requests: Array<{ url: string; signal?: AbortSignal | null }> = [];
  const pending: Array<(value: Response) => void> = [];
  const controls = { schema_version: 1, status_capability: "fixture.account_status", logout_capability: "fixture.account_logout" };
  const apiFetch = async (url: string, init?: RequestInit) => {
    requests.push({ url, signal: init?.signal });
    if (url.endsWith("account-control")) return response({ task_id: `fixture-${requests.length}` });
    return new Promise<Response>(resolve => { pending.push(resolve); });
  };
  try {
    await act(async () => { root = create(<>
      <AippAccountsPanel key="first" skill="first" controls={controls} requestScope="first" t={t} apiFetch={apiFetch} />
      <AippAccountsPanel key="second" skill="second" controls={controls} requestScope="second" t={t} apiFetch={apiFetch} />
    </>); });
    const panels = root!.root.findAllByType(AippAccountsPanel);
    const toggles = panels.map(panel => panel.findByProps({ "aria-expanded": false }));
    assert.notEqual(toggles[0].props["aria-controls"], toggles[1].props["aria-controls"]);
    assert.equal(pending.length, 2);
    const count = requests.length;
    await act(async () => { toggles[0].props.onClick(); });
    assert.equal(toggles[0].props["aria-expanded"], true);
    assert.equal(toggles[1].props["aria-expanded"], false);
    await act(async () => { toggles[0].props.onClick(); });
    assert.equal(toggles[0].props["aria-expanded"], false);
    assert.equal(requests.length, count);
    assert.ok(requests.every(request => request.signal && !request.signal.aborted));
    await act(async () => { for (const resolve of pending) resolve(task("succeeded")); });
    await act(async () => { toggles[1].props.onClick(); });
    assert.ok(panels[1].findAllByType("p").some(node => node.children.includes("Saved session; account unverified")));
    assert.equal(requests.length, count);
  } finally { await act(async () => root?.unmount()); }
});

test("task account use distinguishes actual attached credentials and unknown history", async () => {
  let root: ReactTestRenderer | undefined;
  try {
    await act(async () => { root = create(<AippAccountUse t={t} />); });
    assert.match(JSON.stringify(root!.toJSON()), /not recorded/u);
    await act(async () => { root!.update(<AippAccountUse t={t} uses={[{ schema_version: 1, platform: "weibo", mode: "authenticated_browser",
      account: { ...snapshot, status: "verified", display_name: "Self account" } }]} />); });
    assert.match(JSON.stringify(root!.toJSON()), /Requests included login credentials/u);
    assert.match(JSON.stringify(root!.toJSON()), /Self account/u);
  } finally { await act(async () => root?.unmount()); }
});
