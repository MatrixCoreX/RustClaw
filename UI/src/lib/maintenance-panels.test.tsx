import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { MemoryJobReviewPanel } from "../components/MemoryJobReviewPanel";
import { RuntimeCheckpointPanel } from "../components/RuntimeCheckpointPanel";
import { UiDialogProvider } from "../components/UiDialogProvider";
const t = (_zh: string, en: string) => en;
const json = (data: unknown) => new Response(JSON.stringify({ ok: true, data }), { headers: { "Content-Type": "application/json" } });
function deferred() { let resolve!: (value: Response) => void; const promise = new Promise<Response>(done => { resolve = done; }); return { resolve, promise }; }
function panel(scope: string, apiFetch: (path: string, init?: RequestInit) => Promise<Response>) { return <UiDialogProvider><MemoryJobReviewPanel t={t} requestScope={scope} apiFetch={apiFetch} /></UiDialogProvider>; }

test("historical jobs read is owner scoped and late A responses cannot appear under B", async () => {
  const pending = deferred(); let calls = 0; const requests: RequestInit[] = [];
  const apiFetch = async (_path: string, init?: RequestInit) => { requests.push(init ?? {}); return ++calls === 1 ? pending.promise : json({ jobs: [], total_failed: 0, limit: 100 }); };
  let view!: ReactTestRenderer;
  await act(async () => { view = create(panel("A", apiFetch)); });
  await act(async () => { view.update(panel("B", apiFetch)); });
  await act(async () => { pending.resolve(json({ jobs: [{ job_id: "private-A", job_kind: "extract", attempt: 5, error_code: "private-A", archived: false, review_token: "old" }], total_failed: 1, limit: 100 })); });
  assert(!JSON.stringify(view.toJSON()).includes("private-A"));
  assert(JSON.stringify(view.toJSON()).includes("no failed memory"));
  assert(requests.every(value => !value.method || value.method === "GET"));
  await act(async () => { view.unmount(); });
});

test("unmount invalidates captured memory refresh callbacks", async () => {
  let calls = 0; const apiFetch = async () => { calls++; return json({ jobs: [], total_failed: 0, limit: 100 }); };
  let view!: ReactTestRenderer;
  await act(async () => { view = create(panel("A", apiFetch)); });
  const refresh = view.root.findAllByType("button")[0].props.onClick;
  await act(async () => { view.unmount(); await refresh(); });
  assert.equal(calls, 1);
});

test("checkpoint status hides old owner and reports missing status truthfully", async () => {
  const pending = deferred(); let calls = 0; const apiFetch = async () => ++calls === 1 ? pending.promise : json({ status: "unavailable" });
  let view!: ReactTestRenderer;
  await act(async () => { view = create(<RuntimeCheckpointPanel t={t} apiFetch={apiFetch} requestScope="A" />); });
  await act(async () => { view.update(<RuntimeCheckpointPanel t={t} apiFetch={apiFetch} requestScope="B" />); pending.resolve(json({ status: "verified", bytes: 99, created_at_ts: 1000 })); });
  assert(JSON.stringify(view.toJSON()).includes("No verified checkpoint"));
  assert(!JSON.stringify(view.toJSON()).includes("passed verification"));
  await act(async () => { view.unmount(); });
});
