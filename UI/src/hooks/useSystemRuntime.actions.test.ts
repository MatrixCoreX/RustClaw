import assert from "node:assert/strict";
import test from "node:test";
import { act } from "react-test-renderer";
import { systemRuntimeHarness as harness } from "../../test/helpers/system-runtime-harness";
import type { UseSystemRuntimeParams, useSystemRuntime } from "./useSystemRuntime";

type Runtime = ReturnType<typeof useSystemRuntime>;
type Harness = Awaited<ReturnType<typeof harness>>;
const actions = [
  { name: "dependency", call: (r: Runtime) => r.installHostDependency("node"),
    busy: (r: Runtime) => r.dependencyInstallingId !== null, error: (r: Runtime) => r.hostDependenciesErrorCode },
  { name: "diagnostics", call: (r: Runtime) => r.exportSystemDiagnostics(),
    busy: (r: Runtime) => r.systemDiagnosticsExporting, error: (r: Runtime) => r.systemDiagnosticsExportError },
  { name: "hardware", call: (r: Runtime) => r.restartPiApp(),
    busy: (r: Runtime) => r.piAppRestarting, error: (r: Runtime) => r.piAppRestartMessage },
  { name: "restart", call: (r: Runtime) => r.restartSystem(),
    busy: (r: Runtime) => r.systemRestarting, error: (r: Runtime) => r.systemRestartMessage },
  // The HTTPS wizard already confirms activation before calling this hook.
  { name: "workspace", call: (r: Runtime) => r.startWorkspaceUpdate("local_https_enable"),
    busy: (r: Runtime) => r.workspaceUpdateLoading, error: (r: Runtime) => r.workspaceUpdateMessage },
] as const;
async function start(h: Harness, call: (r: Runtime) => unknown) {
  const index = h.pending.length;
  await act(async () => { void call(h.current); });
  return index;
}
const scopes: { name: string; change: Partial<UseSystemRuntimeParams> }[] = [
  { name: "endpoint", change: { apiBase: "http://localhost:9876" } },
  { name: "logout", change: { uiAuthReady: false } },
  { name: "role", change: { isAdminIdentity: false } },
  { name: "login epoch", change: { requestEpoch: 1 } },
];
for (const action of actions) {
  for (const scope of scopes) test(`${action.name} ignores old failures and releases its UI after ${scope.name} changes`, async () => {
    const h = await harness();
    try {
      await h.update({ isAdminIdentity: true });
      const index = await start(h, action.call);
      assert.equal(h.pending.length, index + 1);
      assert.equal(action.busy(h.current), true);
      await h.update(scope.change);
      assert.equal(action.busy(h.current), false);
      await h.reject(index);
      assert.equal(action.error(h.current), null);
      assert.equal(action.busy(h.current), false);
    } finally { await h.dispose(); }
  });

  test(`${action.name} suppresses duplicate local submissions but retains current errors`, async () => {
    const h = await harness();
    try {
      await h.update({ isAdminIdentity: true });
      const index = await start(h, action.call);
      await start(h, action.call);
      assert.equal(h.pending.length, index + 1);
      await h.reject(index);
      assert.ok(action.error(h.current));
      assert.equal(action.busy(h.current), false);
      const next = h.pending.length;
      await start(h, action.call);
      assert.equal(h.pending.length, next + 1);
      assert.equal(h.pending[next].path, h.pending[index].path);
    } finally { await h.dispose(); }
  });

  test(`${action.name} rejects captured callbacks after endpoint A/B/A`, async () => {
    const h = await harness();
    try {
      await h.update({ isAdminIdentity: true });
      const captured = h.current;
      await h.update({ apiBase: "http://localhost:9876" });
      await h.update({ apiBase: "http://localhost:8788" });
      await act(async () => { void action.call(captured); });
      assert.equal(h.pending.length, 0);
    } finally { await h.dispose(); }
  });

  test(`${action.name} checks ownership after a held response body`, async () => {
    const h = await harness();
    try {
      await h.update({ isAdminIdentity: true });
      const index = await start(h, action.call);
      let release!: (body: unknown) => void;
      const response = new Response("{}");
      response.json = () => new Promise(resolve => { release = resolve; });
      await act(async () => { h.pending[index].resolve(response); });
      await h.update({ requestEpoch: 1 });
      await act(async () => { release({ ok: true, data: {}, error: null }); });
      assert.equal(h.pending.length, index + 1);
      assert.equal(action.error(h.current), null);
      assert.equal(action.busy(h.current), false);
    } finally { await h.dispose(); }
  });

  test(`${action.name} does not dispatch follow-ups after unmount`, async () => {
    const h = await harness();
    await h.update({ isAdminIdentity: true });
    const index = await start(h, action.call);
    await h.dispose();
    await h.respond(index, {});
    assert.equal(h.pending.length, index + 1);
  });
}

test("an old dependency acknowledgment does not refresh either endpoint", async () => {
  const h = await harness();
  try {
    await h.update({ isAdminIdentity: true });
    const index = await start(h, r => r.installHostDependency("node"));
    await h.update({ apiBase: "http://localhost:9876" });
    await h.respond(index, { status: "queued" });
    assert.equal(h.pending.length, index + 1);
    assert.equal(h.current.hostDependencies, null);
  } finally { await h.dispose(); }
});

test("different dependencies retain independent dispatch and busy ownership", async () => {
  const h = await harness();
  try {
    await h.update({ isAdminIdentity: true });
    const node = await start(h, r => r.installHostDependency("node"));
    const ffmpeg = await start(h, r => r.installHostDependency("ffmpeg"));
    assert.equal(ffmpeg, node + 1);
    await h.reject(node);
    assert.equal(h.current.dependencyInstallingId, "ffmpeg");
    await h.reject(ffmpeg);
    assert.equal(h.current.dependencyInstallingId, null);
  } finally { await h.dispose(); }
});

test("an ordinary authenticated user may still export diagnostics but cannot dispatch administrative writes", async () => {
  const h = await harness();
  try {
    for (const action of actions.filter(action => action.name !== "diagnostics")) await start(h, action.call);
    assert.equal(h.pending.length, 0);
    await start(h, r => r.exportSystemDiagnostics());
    assert.equal(h.pending[0].path, "/v1/system/diagnostics/export");
    await h.reject(0);
    assert.ok(h.current.systemDiagnosticsExportError);
  } finally { await h.dispose(); }
});

test("same-role reauthentication clears old snapshots and rejects pending reads", async () => {
  const h = await harness();
  try {
    await h.respond(await h.start("summary"), { architecture: "old-principal" });
    const old = await h.start("dependencies");
    await h.update({ requestEpoch: 1 });
    assert.equal(h.current.hostSystemSummary, null);
    await h.respond(old, { platform: "old-principal", operations: [] });
    assert.equal(h.current.hostDependencies, null);
  } finally { await h.dispose(); }
});

test("a workspace acknowledgment supersedes an earlier status read", async () => {
  const h = await harness();
  try {
    await h.update({ isAdminIdentity: true });
    const read = await h.start("workspace");
    const write = await start(h, r => r.startWorkspaceUpdate("local_https_enable"));
    await h.respond(write, { status: "running", mode: "local_https_enable", current_version: "accepted" });
    await h.respond(read, { status: "succeeded", mode: "build_ui", current_version: "obsolete" });
    assert.equal(h.current.workspaceUpdateStatus?.current_version, "accepted");
    assert.equal(h.reloadTimers, 0);
  } finally { await h.dispose(); }
});

test("a new action remains busy when an old endpoint's finally completes", async () => {
  const h = await harness();
  try {
    await h.update({ isAdminIdentity: true });
    const old = await start(h, r => r.restartPiApp());
    await h.update({ apiBase: "http://localhost:9876" });
    const latest = await start(h, r => r.restartPiApp());
    await h.reject(old);
    assert.equal(h.current.piAppRestarting, true);
    assert.equal(h.current.piAppRestartMessage, null);
    assert.equal(h.pending.length, latest + 1);
    await h.respond(latest, {});
    assert.equal(h.current.piAppRestarting, false);
    assert.ok(h.current.piAppRestartMessage);
    assert.equal(h.pending.at(-1)?.path, "/v1/pi-app/status");
  } finally { await h.dispose(); }
});

test("workspace reads and polls wait for a pending write without inventing failures", async () => {
  const h = await harness();
  try {
    await h.update({ isAdminIdentity: true });
    await h.respond(await h.start("workspace"), { status: "restarting", mode: "build_ui" });
    const write = await start(h, r => r.startWorkspaceUpdate("local_https_enable"));
    await h.start("workspace"); await h.poll(); await h.poll(); await h.poll();
    assert.equal(h.pending.length, write + 1);
    assert.equal(h.current.workspaceUpdateMessage, null);
    assert.equal(h.current.workspaceUpdateLoading, true);
    await h.respond(write, { status: "running", mode: "local_https_enable" });
    await h.poll();
    assert.equal(h.pending.length, write + 2);
    assert.match(h.pending.at(-1)!.path, /^\/v1\/admin\/workspace-update/);
  } finally { await h.dispose(); }
});

test("dependency acknowledgment rechecks state and invalidates an older manual refresh", async () => {
  const h = await harness();
  try {
    await h.update({ isAdminIdentity: true });
    const read = await h.start("dependencies");
    const write = await start(h, r => r.installHostDependency("node"));
    assert.deepEqual(JSON.parse(h.pending[write].init?.body as string), { dependency_id: "node" });
    await h.respond(write, { status: "queued" });
    const refresh = h.pending.length - 1;
    assert.equal(h.pending[refresh].path, "/v1/system/dependencies");
    await h.respond(refresh, { platform: "fresh", operations: [] });
    await h.respond(read, { platform: "obsolete", operations: [] });
    assert.equal(h.current.hostDependencies?.platform, "fresh");
    assert.equal(h.current.dependencyInstallingId, null);
    assert.equal(h.current.hostDependenciesLoading, false);
  } finally { await h.dispose(); }
});
