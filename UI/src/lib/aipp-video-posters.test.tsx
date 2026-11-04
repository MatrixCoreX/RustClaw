import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { ActivityVideoPlayer, ActivityVideoPreview } from "../components/AippTaskActivityMedia";
import type { AippTaskActivityArtifact } from "../types/api";

const artifact: AippTaskActivityArtifact = { schema_version: 1, id: "video", filename: "clip.webm", kind: "video", mime_type: "video/webm", size_bytes: 16,
  preview_url: "/v1/tasks/poster-test/artifacts/video/content?disposition=inline", download_url: "/v1/tasks/poster-test/artifacts/video/content" };
const t = (_zh: string, en: string) => en;
function urls() {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  const createUrl = URL.createObjectURL, revokeUrl = URL.revokeObjectURL;
  const created: Array<{ url: string; blob: Blob }> = [], revoked: string[] = [];
  URL.createObjectURL = blob => { const url = `blob:owned-${created.length}`; created.push({ url, blob: blob as Blob }); return url; };
  URL.revokeObjectURL = url => { revoked.push(url); };
  return { created, revoked, restore: () => { URL.createObjectURL = createUrl; URL.revokeObjectURL = revokeUrl; } };
}

test("the thumbnail shares poster bytes and the player owns an independent URL without a second fetch", async () => {
  const fixture = urls();
  let thumbnail: ReactTestRenderer | undefined, player: ReactTestRenderer | undefined;
  let poster: Blob | undefined;
  const requests: string[] = [];
  const apiFetch = async (path: string) => {
    requests.push(path);
    return new Response(path.includes("preview=poster") ? "poster" : "video", { headers: { "content-type": path.includes("preview=poster") ? "image/png" : "video/webm" } });
  };
  try {
    await act(async () => { thumbnail = create(<ActivityVideoPreview artifact={artifact} apiFetch={apiFetch} t={t}
      onOpen={() => undefined} onPosterLoaded={blob => { poster = blob; }} />); });
    assert.ok(poster);
    const borrowedUrl = thumbnail!.root.findByType("img").props.src;
    await act(async () => { player = create(<ActivityVideoPlayer artifact={artifact} apiFetch={apiFetch} t={t} initialPoster={poster} />); });
    assert.equal(requests.filter(path => path.includes("preview=poster")).length, 1);
    const ownedUrl = player!.root.findByType("video").props.poster;
    assert.notEqual(ownedUrl, borrowedUrl);
    assert.equal(fixture.created.find(row => row.url === ownedUrl)?.blob, poster);
    await act(async () => thumbnail!.unmount());
    assert.ok(fixture.revoked.includes(borrowedUrl));
    assert.ok(!fixture.revoked.includes(ownedUrl), "releasing the list thumbnail must not invalidate the player poster");
    await act(async () => player!.unmount());
    assert.deepEqual([...fixture.revoked].sort(), fixture.created.map(row => row.url).sort());
  } finally {
    await act(async () => { thumbnail?.unmount(); player?.unmount(); }); fixture.restore();
  }
});

test("opening without cached poster bytes still fetches a poster and does not block video on poster failure", async () => {
  const fixture = urls(); let player: ReactTestRenderer | undefined;
  const requests: string[] = [];
  try {
    await act(async () => { player = create(<ActivityVideoPlayer artifact={artifact} t={t} apiFetch={async path => {
      requests.push(path); return new Response(path.includes("preview=poster") ? "missing" : "video", { status: path.includes("preview=poster") ? 503 : 200 });
    }} />); });
    assert.equal(requests.filter(path => path.includes("preview=poster")).length, 1);
    assert.ok(player!.root.findByType("video").props.src.startsWith("blob:"));
    assert.equal(player!.root.findByType("video").props.poster, undefined);
  } finally { await act(async () => player?.unmount()); fixture.restore(); }
});
