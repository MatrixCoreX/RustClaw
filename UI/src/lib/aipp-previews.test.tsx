import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { ActivityImagePreview } from "../components/AippTaskActivityMedia";
import type { AippTaskActivityArtifact } from "../types/api";

const t = (_zh: string, en: string) => en;
const artifact = (id: string): AippTaskActivityArtifact => ({
  schema_version: 1, id, filename: `${id}.png`, kind: "image", mime_type: "image/png", size_bytes: 16,
  preview_url: `/v1/tasks/preview-test/artifacts/${id}/content?disposition=inline`,
  download_url: `/v1/tasks/preview-test/artifacts/${id}/content?disposition=attachment`,
});

function previewFixture() {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  const createUrl = URL.createObjectURL;
  const revokeUrl = URL.revokeObjectURL;
  const created: string[] = [], revoked: string[] = [];
  const requests: Array<{ path: string; signal: AbortSignal; resolve: (response: Response) => void }> = [];
  URL.createObjectURL = () => { const url = `blob:preview-${created.length}`; created.push(url); return url; };
  URL.revokeObjectURL = url => { revoked.push(url); };
  return {
    created, revoked, requests,
    apiFetch: (path: string, init?: RequestInit) => new Promise<Response>(resolve => {
      assert.ok(init?.signal, "each preview must carry its own cancellation signal");
      requests.push({ path, signal: init.signal, resolve });
    }),
    restore: () => { URL.createObjectURL = createUrl; URL.revokeObjectURL = revokeUrl; },
  };
}

test("removing an image preview aborts its request and ignores a late response", async () => {
  const fixture = previewFixture();
  let root: ReactTestRenderer | undefined;
  try {
    await act(async () => { root = create(<ActivityImagePreview artifact={artifact("first")}
      apiFetch={fixture.apiFetch} t={t} onOpen={() => undefined} />); });
    assert.equal(fixture.requests.length, 1);
    assert.equal(fixture.requests[0].signal.aborted, false);
    await act(async () => root!.unmount());
    assert.equal(fixture.requests[0].signal.aborted, true);
    // A transport may finish even after cancellation; it still must not publish a blob URL.
    await act(async () => { fixture.requests[0].resolve(new Response("old image")); });
    assert.deepEqual(fixture.created, []);
    assert.deepEqual(fixture.revoked, []);
  } finally {
    await act(async () => root?.unmount());
    fixture.restore();
  }
});

test("changing an image aborts only the old preview, ignores stale results and releases its blob", async () => {
  const fixture = previewFixture();
  let root: ReactTestRenderer | undefined;
  const opened: Array<string | null> = [];
  const render = (id: string) => <ActivityImagePreview artifact={artifact(id)} apiFetch={fixture.apiFetch}
    t={t} onOpen={source => opened.push(source)} />;
  try {
    await act(async () => { root = create(render("old")); });
    await act(async () => root!.update(render("new")));
    assert.equal(fixture.requests.length, 2);
    assert.equal(fixture.requests[0].signal.aborted, true);
    assert.equal(fixture.requests[1].signal.aborted, false);
    await act(async () => { fixture.requests[1].resolve(new Response("new image")); });
    assert.equal(root!.root.findByType("img").props.src, "blob:preview-0");
    await act(async () => { fixture.requests[0].resolve(new Response("late old image")); });
    assert.equal(root!.root.findByType("img").props.src, "blob:preview-0");
    assert.deepEqual(fixture.created, ["blob:preview-0"]);
    await act(async () => root!.update(render("new")));
    assert.equal(fixture.requests.length, 2, "unchanged endpoints must reuse the preview");
    await act(async () => root!.root.findByType("button").props.onClick());
    assert.deepEqual(opened, ["blob:preview-0"], "opening reuses the selected thumbnail");
    await act(async () => root!.unmount());
    assert.equal(fixture.requests[1].signal.aborted, true);
    assert.deepEqual(fixture.revoked, ["blob:preview-0"]);
  } finally {
    await act(async () => root?.unmount());
    fixture.restore();
  }
});

test("a failed image preview still allows opening the original viewer", async () => {
  const fixture = previewFixture();
  let root: ReactTestRenderer | undefined;
  const opened: Array<string | null> = [];
  try {
    await act(async () => { root = create(<ActivityImagePreview artifact={artifact("failed")}
      apiFetch={fixture.apiFetch} t={t} onOpen={source => opened.push(source)} />); });
    await act(async () => { fixture.requests[0].resolve(new Response("busy", { status: 503 })); });
    assert.equal(root!.root.findAllByType("img").length, 0);
    await act(async () => root!.root.findByType("button").props.onClick());
    assert.deepEqual(opened, [null]);
    assert.deepEqual(fixture.created, []);
  } finally {
    await act(async () => root?.unmount());
    fixture.restore();
  }
});
