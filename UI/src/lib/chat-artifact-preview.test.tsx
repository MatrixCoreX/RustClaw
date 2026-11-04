import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { TaskArtifactCard } from "../components/ChatPage";
import { withSessionArtifactStreaming } from "./artifact-session-stream";
import type { TaskArtifact } from "../types/api";

const artifact: TaskArtifact = { schema_version: 2, id: "video", artifact_ref: "artifact:task/stream-test/video", filename: "clip.webm", kind: "video", mime_type: "video/webm", size_bytes: 16, sha256: "a".repeat(64),
  preview_url: "/v1/tasks/stream-test/artifacts/video/content?disposition=inline", download_url: "/v1/tasks/stream-test/artifacts/video/content" };
const t = (_zh: string, en: string) => en;
const options = { authMode: "webd" as const, apiBase: "https://console.example", pageUrl: "https://console.example/", isCurrent: () => true };
function fixture() {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  const original = { create: URL.createObjectURL, revoke: URL.revokeObjectURL };
  const created: string[] = [], revoked: string[] = [];
  URL.createObjectURL = () => { const url = `blob:owned-${created.length}`; created.push(url); return url; };
  URL.revokeObjectURL = url => { revoked.push(url); };
  const media = { paused: 0, removed: 0, loaded: 0, pause() { this.paused++; }, removeAttribute() { this.removed++; }, load() { this.loaded++; } };
  return { created, revoked, media, restore() { URL.createObjectURL = original.create; URL.revokeObjectURL = original.revoke; } };
}

test("chat streams original session video, switches once to the compatible copy, and releases playback on removal", async () => {
  const f = fixture(); let view: ReactTestRenderer | undefined;
  const calls: Array<{ path: string; method: string }> = [];
  const api = withSessionArtifactStreaming(async (path, init) => {
    calls.push({path, method: init?.method || "GET"});
    return new Response("video", { headers: { "content-type": "video/webm", "accept-ranges": "bytes" } });
  }, options);
  try {
    await act(async () => { view = create(<TaskArtifactCard artifact={artifact} artifactFetch={api} t={t} />, { createNodeMock: element => element.type === "video" ? f.media : null }); });
    assert.equal(view!.root.findByType("video").props.src, `https://console.example${artifact.preview_url}`);
    assert.equal(view!.root.findByType("video").props.preload, "metadata");
    assert.equal(calls.filter(call => !call.path.includes("preview=poster")).length, 1);
    assert.equal(calls[0].method, "HEAD");
    await act(async () => view!.root.findByType("video").props.onError());
    assert.ok(view!.root.findByType("video").props.src.startsWith("blob:"));
    assert.equal(calls.filter(call => call.path.includes("preview=browser")).length, 1);
    await act(async () => view!.root.findByType("video").props.onError());
    assert.equal(view!.root.findAllByType("video").length, 0);
    assert.equal(calls.filter(call => call.path.includes("preview=browser")).length, 1, "decode failure must not loop");
    await act(async () => view!.unmount());
    assert.ok(f.media.paused > 0 && f.media.removed > 0 && f.media.loaded > 0);
    assert.deepEqual([...f.revoked].sort(), f.created.sort());
  } finally { await act(async () => view?.unmount()); f.restore(); }
});

test("large chat videos stay manual and key-based login retains its authenticated compatible-copy path", async () => {
  const f = fixture(); let view: ReactTestRenderer | undefined;
  const calls: string[] = [];
  try {
    await act(async () => { view = create(<TaskArtifactCard artifact={{...artifact, size_bytes: 1024 ** 3}} artifactFetch={async path => { calls.push(path); return new Response("video"); }} t={t} />); });
    assert.equal(calls.length, 0);
    const load = view!.root.findAllByType("button").find(button => button.props.children === "Load preview")!;
    await act(async () => load.props.onClick());
    assert.equal(calls.filter(path => path.includes("preview=browser")).length, 1);
    assert.ok(view!.root.findByType("video").props.src.startsWith("blob:"));
  } finally { await act(async () => view?.unmount()); f.restore(); }
});

test("Retry really reissues a failed chat preview, with or without an available poster", async () => {
  for (const poster of [true, false]) {
    const f = fixture(); let view: ReactTestRenderer | undefined, fail = true, reads = 0;
    try {
      await act(async () => { view = create(<TaskArtifactCard artifact={artifact} artifactFetch={async path => {
        const isPoster = path.includes("preview=poster"); if (!isPoster) reads++;
        return new Response("content", { status: isPoster ? poster ? 200 : 503 : fail ? 503 : 200 });
      }} t={t} />); });
      assert.equal(reads, 1);
      fail = false;
      await act(async () => view!.root.findAllByType("button").find(button => button.props.children === "Retry")!.props.onClick());
      assert.equal(reads, 2); assert.ok(view!.root.findByType("video").props.src.startsWith("blob:"));
    } finally { await act(async () => view?.unmount()); f.restore(); }
  }
});

test("removing a chat artifact aborts a pending authenticated stream probe and ignores its late result", async () => {
  const f = fixture(); let view: ReactTestRenderer | undefined, signal: AbortSignal | undefined;
  let finish!: (response: Response) => void;
  const pending = new Promise<Response>(resolve => { finish = resolve; });
  const api = withSessionArtifactStreaming(async (path, init) => {
    if (path.includes("preview=poster")) return new Response(null, { status: 503 });
    signal = init?.signal ?? undefined; return pending;
  }, options);
  try {
    await act(async () => { view = create(<TaskArtifactCard artifact={artifact} artifactFetch={api} t={t} />); });
    assert.ok(signal && !signal.aborted);
    await act(async () => view!.unmount()); assert.equal(signal.aborted, true);
    await act(async () => finish(new Response(null, { headers: {"content-type":"video/webm", "accept-ranges":"bytes"} })));
    assert.equal(f.created.length, 0);
  } finally { await act(async () => view?.unmount()); f.restore(); }
});
