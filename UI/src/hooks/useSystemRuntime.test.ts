import assert from "node:assert/strict";
import test from "node:test";
import { act } from "react-test-renderer";
import { systemRuntimeHarness as harness } from "../../test/helpers/system-runtime-harness";

function summary(architecture: string) { return { schema_version: 4, architecture }; }
function dependencies(platform: string) {
  return { schema_version: 1, platform, dependencies: [], operations: [], summary: {} };
}

test("host summary applies the latest refresh rather than the last completed request", async () => {
  const h = await harness();
  try {
    const old = await h.start("summary");
    const latest = await h.start("summary");
    await h.respond(latest, summary("current"));
    await h.respond(old, summary("obsolete"));
    assert.equal(h.current.hostSystemSummary?.architecture, "current");
  } finally { await h.dispose(); }
});

test("obsolete summary errors cannot clear a newer request's loading state", async () => {
  const h = await harness();
  try {
    const old = await h.start("summary");
    const latest = await h.start("summary");
    await h.respond(old, null, 500);
    assert.equal(h.current.hostSystemLoading, true);
    assert.equal(h.current.hostSystemErrorCode, null);
    await h.respond(latest, summary("current"));
    assert.equal(h.current.hostSystemLoading, false);
  } finally { await h.dispose(); }
});

test("dependency refreshes reject obsolete success and transport errors", async () => {
  const h = await harness();
  try {
    const old = await h.start("dependencies");
    const failed = await h.start("dependencies");
    const latest = await h.start("dependencies");
    await h.respond(latest, dependencies("current"));
    await h.respond(old, dependencies("obsolete"));
    await h.reject(failed);
    assert.equal(h.current.hostDependencies?.platform, "current");
    assert.equal(h.current.hostDependenciesErrorCode, null);
  } finally { await h.dispose(); }
});

test("endpoint changes clear loaded host data and invalidate pending old reads", async () => {
  const h = await harness();
  try {
    await h.respond(await h.start("summary"), summary("old-host"));
    await h.respond(await h.start("dependencies"), dependencies("old-host"));
    const old = await h.start("summary");
    await h.update({ apiBase: "http://localhost:9876" });
    assert.equal(h.current.hostSystemSummary, null);
    assert.equal(h.current.hostDependencies, null);
    assert.equal(h.current.hostSystemLoading, false);
    await h.respond(old, summary("obsolete"));
    assert.equal(h.current.hostSystemSummary, null);
    await h.respond(await h.start("summary"), summary("new-host"));
    assert.equal(h.current.hostSystemSummary?.architecture, "new-host");
  } finally { await h.dispose(); }
});

test("authentication loss clears host data and prevents a late dependency result from restoring it", async () => {
  const h = await harness();
  try {
    await h.respond(await h.start("summary"), summary("authenticated"));
    await h.respond(await h.start("dependencies"), dependencies("authenticated"));
    const old = await h.start("dependencies");
    await h.update({ uiAuthReady: false });
    assert.equal(h.current.hostSystemSummary, null);
    assert.equal(h.current.hostDependencies, null);
    await h.respond(old, dependencies("obsolete"));
    assert.equal(h.current.hostDependencies, null);
    assert.equal(h.current.hostDependenciesLoading, false);
  } finally { await h.dispose(); }
});

test("current summary and dependency failures retain existing UI error classification", async () => {
  const h = await harness();
  try {
    await h.respond(await h.start("summary"), null, 403);
    assert.equal(h.current.hostSystemErrorCode, "permission_denied");
    assert.equal(h.current.hostSystemLoading, false);
    await h.reject(await h.start("dependencies"));
    assert.equal(h.current.hostDependenciesErrorCode, "disconnected");
    assert.equal(h.current.hostDependenciesLoading, false);
    await h.respond(await h.start("summary"), summary("current"));
    await h.respond(await h.start("dependencies"), dependencies("current"));
    assert.equal(h.current.hostSystemErrorCode, null);
    assert.equal(h.current.hostDependenciesErrorCode, null);
  } finally { await h.dispose(); }
});

test("a silent dependency poll does not overtake an active manual refresh", async () => {
  const h = await harness();
  try {
    const manual = await h.start("dependencies");
    const count = h.pending.length;
    await h.start("dependencies", true);
    assert.equal(h.pending.length, count);
    assert.equal(h.current.hostDependenciesLoading, true);
    await h.respond(manual, dependencies("current"));
    assert.equal(h.current.hostDependencies?.platform, "current");
    assert.equal(h.current.hostDependenciesLoading, false);
    await h.respond(await h.start("dependencies", true), dependencies("polled"));
    assert.equal(h.current.hostDependencies?.platform, "polled");
    assert.equal(h.current.hostDependenciesLoading, false);
  } finally { await h.dispose(); }
});

test("host reads do not start after authentication has been lost", async () => {
  const h = await harness();
  try {
    await h.update({ uiAuthReady: false });
    await h.start("summary");
    await h.start("dependencies");
    assert.equal(h.pending.length, 0);
    assert.equal(h.current.hostSystemLoading, false);
    assert.equal(h.current.hostDependenciesLoading, false);
  } finally { await h.dispose(); }
});

test("privilege changes invalidate previously loaded and pending host reads", async () => {
  const h = await harness();
  try {
    await h.update({ isAdminIdentity: true });
    await h.respond(await h.start("summary"), summary("admin-view"));
    const old = await h.start("dependencies");
    await h.update({ isAdminIdentity: false });
    assert.equal(h.current.hostSystemSummary, null);
    await h.respond(old, dependencies("obsolete-admin-view"));
    assert.equal(h.current.hostDependencies, null);
    await h.respond(await h.start("dependencies"), dependencies("ordinary-view"));
    assert.equal(h.current.hostDependencies?.platform, "ordinary-view");
  } finally { await h.dispose(); }
});

test("retained old-scope callbacks cannot start host requests on a new endpoint", async () => {
  const h = await harness();
  try {
    const summaryRead = h.current.fetchHostSystemSummary;
    const dependencyRead = h.current.fetchHostDependencies;
    await h.update({ apiBase: "http://localhost:9876" });
    await act(async () => { await summaryRead(); await dependencyRead(); });
    assert.equal(h.pending.length, 0);
    assert.equal(h.current.hostSystemLoading, false);
    assert.equal(h.current.hostDependenciesLoading, false);
  } finally { await h.dispose(); }
});

test("an unmounted dependency request cannot deliver an obsolete snapshot to its caller", async () => {
  const h = await harness();
  try {
    let request!: ReturnType<typeof h.current.fetchHostDependencies>;
    await act(async () => { request = h.current.fetchHostDependencies(); });
    await h.dispose();
    await h.respond(0, dependencies("obsolete"));
    assert.equal(await request, null);
  } finally { await h.dispose(); }
});

test("authenticated dashboard entry still loads both current host snapshots", async () => {
  const h = await harness();
  try {
    await h.update({ currentPage: "dashboard" });
    assert.deepEqual(h.pending.map(request => request.path), [
      "/v1/system/host-summary", "/v1/system/dependencies",
    ]);
    await h.respond(0, summary("current"));
    await h.respond(1, dependencies("current"));
    assert.equal(h.current.hostSystemSummary?.architecture, "current");
    assert.equal(h.current.hostDependencies?.platform, "current");
    assert.equal(h.current.hostSystemLoading, false);
    assert.equal(h.current.hostDependenciesLoading, false);
  } finally { await h.dispose(); }
});
