import assert from "node:assert/strict";
import test from "node:test";
import { systemRuntimeHarness as harness, type SystemRead } from "../../test/helpers/system-runtime-harness";
import type { useSystemRuntime } from "./useSystemRuntime";

type Runtime = ReturnType<typeof useSystemRuntime>;
const reads = [
  { kind: "workspace", value: (r: Runtime) => r.workspaceUpdateStatus?.current_version,
    loading: (r: Runtime) => r.workspaceUpdateLoading, error: (r: Runtime) => r.workspaceUpdateMessage },
  { kind: "nginx", value: (r: Runtime) => r.nginxStatus?.platform,
    loading: (r: Runtime) => r.nginxStatusLoading, error: (r: Runtime) => r.nginxStatusError },
  { kind: "webd", value: (r: Runtime) => r.webdExposureStatus?.platform,
    loading: (r: Runtime) => r.webdExposureLoading, error: (r: Runtime) => r.webdExposureError },
  { kind: "hardware", value: (r: Runtime) => r.piAppStatus?.model },
] as const;
function payload(kind: SystemRead, marker: string) {
  switch (kind) {
    case "workspace": return { status: "idle", step: "fixture", current_version: marker, stdout_tail: "", stderr_tail: "" };
    case "nginx": return { supported: true, platform: marker, installed: true, running: true, configured: true,
      ui_deployed: true, clawd_exposure: "loopback_only", local_https_supported: true,
      local_https_prepared: false, local_https_enabled: false };
    case "webd": return { supported: true, platform: marker, enabled: true, running: true, listen: "127.0.0.1:8788",
      port: 8788, externally_accessible: false, nginx_compatible: true, restart_scheduled: false };
    case "hardware": return { available: true, is_raspberry_pi: false, model: marker };
    default: throw new Error("fixture_read_kind_invalid");
  }
}
function workspace(status: string) {
  return { status, mode: "build_ui", step: "fixture", stdout_tail: "", stderr_tail: "" };
}

for (const read of reads) {
  test(`${read.kind} accepts only the latest refresh`, async () => {
    const h = await harness();
    try {
      await h.update({ isAdminIdentity: true });
      const old = await h.start(read.kind), latest = await h.start(read.kind);
      await h.respond(latest, payload(read.kind, "current"));
      await h.respond(old, payload(read.kind, "obsolete"));
      assert.equal(read.value(h.current), "current");
    } finally { await h.dispose(); }
  });

  test(`${read.kind} ignores obsolete transport failures`, async () => {
    const h = await harness();
    try {
      await h.update({ isAdminIdentity: true });
      const old = await h.start(read.kind), latest = await h.start(read.kind);
      await h.respond(latest, payload(read.kind, "current"));
      await h.reject(old);
      assert.equal(read.value(h.current), "current");
      if ("error" in read) assert.equal(read.error(h.current), null);
    } finally { await h.dispose(); }
  });

  test(`${read.kind} clears data and rejects old results on endpoint change`, async () => {
    const h = await harness();
    try {
      await h.update({ isAdminIdentity: true });
      await h.respond(await h.start(read.kind), payload(read.kind, "old-host"));
      const old = await h.start(read.kind);
      await h.update({ apiBase: "http://localhost:9876" });
      assert.equal(read.value(h.current), undefined);
      if ("loading" in read) assert.equal(read.loading(h.current), false);
      await h.respond(old, payload(read.kind, "obsolete"));
      assert.equal(read.value(h.current), undefined);
      await h.respond(await h.start(read.kind), payload(read.kind, "current"));
      assert.equal(read.value(h.current), "current");
    } finally { await h.dispose(); }
  });

  test(`${read.kind} does not send reads without current administrative access`, async () => {
    const h = await harness();
    try {
      await h.start(read.kind);
      assert.equal(h.pending.filter(request => read.kind !== "hardware" || request.path === "/v1/pi-app/status").length, 0);
      await h.update({ isAdminIdentity: true, uiAuthReady: false });
      await h.start(read.kind);
      assert.equal(h.pending.filter(request => read.kind !== "hardware" || request.path === "/v1/pi-app/status").length, 0);
    } finally { await h.dispose(); }
  });

  for (const scope of ["auth", "role"] as const) {
    test(`${read.kind} discards cached and pending results after ${scope} loss`, async () => {
      const h = await harness();
      try {
        await h.update({ isAdminIdentity: true });
        await h.respond(await h.start(read.kind), payload(read.kind, "old-access"));
        const old = await h.start(read.kind);
        await h.update(scope === "auth" ? { uiAuthReady: false } : { isAdminIdentity: false });
        assert.equal(read.value(h.current), undefined);
        await h.respond(old, payload(read.kind, "obsolete"));
        assert.equal(read.value(h.current), undefined);
      } finally { await h.dispose(); }
    });
  }
}

for (const read of [reads[0], reads[1], reads[2]]) {
  test(`${read.kind} silent polling cannot overtake manual loading`, async () => {
    const h = await harness();
    try {
      await h.update({ isAdminIdentity: true });
      const manual = await h.start(read.kind);
      await h.start(read.kind, true);
      assert.equal(h.pending.length, 1);
      assert.equal(read.loading(h.current), true);
      await h.respond(manual, payload(read.kind, "current"));
      assert.equal(read.loading(h.current), false);
    } finally { await h.dispose(); }
  });
}

test("changing endpoint discards old build history before reading the new device's idle state", async () => {
  const h = await harness();
  try {
    await h.update({ isAdminIdentity: true });
    await h.respond(await h.start("workspace"), workspace("running"));
    await h.update({ apiBase: "http://localhost:9876" });
    await h.respond(await h.start("workspace"), workspace("idle"));
    assert.equal(h.reloadTimers, 0);
    await h.flushTimeouts();
    assert.equal(h.reloads, 0);
    assert.equal(h.pollTimers, 0);
  } finally { await h.dispose(); }
});

test("switching device cancels an already scheduled old-device reload", async () => {
  const h = await harness();
  try {
    await h.update({ isAdminIdentity: true });
    await h.respond(await h.start("workspace"), workspace("running"));
    await h.respond(await h.start("workspace"), workspace("succeeded"));
    assert.equal(h.reloadTimers, 1);
    await h.update({ apiBase: "http://localhost:9876" });
    await h.flushTimeouts();
    assert.equal(h.reloads, 0);
  } finally { await h.dispose(); }
});

test("current-device build completion retains one automatic reload", async () => {
  const h = await harness();
  try {
    await h.update({ isAdminIdentity: true });
    await h.respond(await h.start("workspace"), workspace("running"));
    await h.respond(await h.start("workspace"), workspace("succeeded"));
    assert.equal(h.reloadTimers, 1);
    await h.flushTimeouts();
    assert.equal(h.reloads, 1);
  } finally { await h.dispose(); }
});

test("old restart poll failures cannot set feedback on the new device", async () => {
  const h = await harness();
  try {
    await h.update({ isAdminIdentity: true });
    await h.respond(await h.start("workspace"), workspace("restarting"));
    for (let i = 0; i < 3; i++) await h.poll();
    assert.equal(h.pending.length, 4);
    await h.update({ apiBase: "http://localhost:9876" });
    for (let i = 1; i <= 3; i++) await h.respond(i, null, 503);
    assert.equal(h.current.workspaceUpdateMessage, null);
  } finally { await h.dispose(); }
});

test("current restart polling still reports three genuine failures", async () => {
  const h = await harness();
  try {
    await h.update({ isAdminIdentity: true });
    await h.respond(await h.start("workspace"), workspace("restarting"));
    for (let i = 0; i < 3; i++) { await h.poll(); await h.respond(i + 1, null, 503); }
    assert.notEqual(h.current.workspaceUpdateMessage, null);
  } finally { await h.dispose(); }
});

test("poll suppression during manual refresh is not counted as restart failure", async () => {
  const h = await harness();
  try {
    await h.update({ isAdminIdentity: true });
    await h.respond(await h.start("workspace"), workspace("restarting"));
    const manual = await h.start("workspace");
    for (let i = 0; i < 3; i++) await h.poll();
    assert.equal(h.pending.length, 2);
    assert.equal(h.current.workspaceUpdateMessage, null);
    assert.equal(h.current.workspaceUpdateLoading, true);
    await h.respond(manual, workspace("restarting"));
    assert.equal(h.current.workspaceUpdateLoading, false);
  } finally { await h.dispose(); }
});

test("superseded restart polls are not failures after a newer manual read has completed", async () => {
  const h = await harness();
  try {
    await h.update({ isAdminIdentity: true });
    await h.respond(await h.start("workspace"), workspace("restarting"));
    for (let i = 0; i < 3; i++) await h.poll();
    const manual = await h.start("workspace");
    await h.respond(manual, workspace("restarting"));
    assert.equal(h.current.workspaceUpdateLoading, false);
    for (let i = 1; i <= 3; i++) await h.respond(i, workspace("restarting"));
    assert.equal(h.current.workspaceUpdateMessage, null);
  } finally { await h.dispose(); }
});

for (const language of ["zh", "en"] as const) {
  test(`current malformed nginx responses retain the ${language} UI diagnostic`, async () => {
    const h = await harness();
    try {
      await h.update({ isAdminIdentity: true, t: (zh, en) => language === "zh" ? zh : en });
      const index = await h.start("nginx");
      await h.respondRaw(index, "<!doctype html><html><body>fixture</body></html>", "text/html");
      assert.ok(h.current.nginxStatusError?.includes(language === "zh"
        ? "状态接口返回了网页" : "status endpoint returned a web page"));
      assert.equal(h.current.nginxStatusLoading, false);
    } finally { await h.dispose(); }
  });
}
