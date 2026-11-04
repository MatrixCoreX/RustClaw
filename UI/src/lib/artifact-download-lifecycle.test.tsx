import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { TaskArtifactCard } from "../components/ChatPage";
import { AippTaskActivityCard } from "../components/AippPage";
import type { AippTaskActivityItem, TaskArtifact } from "../types/api";

const t = (_zh: string, en: string) => en;
const artifact: TaskArtifact = { schema_version:2,id:"file",artifact_ref:"artifact:task/download-test/file",filename:"report.txt",kind:"file",mime_type:"text/plain",size_bytes:16,sha256:"a".repeat(64),
  download_url:"/v1/tasks/download-test/artifacts/file/content",preview_url:null };
const item: AippTaskActivityItem = {schema_version:1,sequence:1,task_id:"download-test",channel:"ui",status:"succeeded",actions:[],input_text:"Download a file",result_text:"Saved",error_text:null,source_urls:[],created_at:"1788912000",updated_at:"1788912000",event_at_ms:1788912000000,artifacts:[{...artifact, preview_url:artifact.preview_url ?? null}]};

for (const kind of ["chat", "aipp"] as const) {
  test(`${kind} cancels a pending download on removal and ignores late bytes without opening a save`, async () => {
    globalThis.IS_REACT_ACT_ENVIRONMENT = true;
    let signal: AbortSignal | undefined, reads = 0, cancelled = 0, view: ReactTestRenderer | undefined;
    let finish!: (response: Response) => void;
    const pending = new Promise<Response>(resolve => { finish = resolve; });
    const fetcher = async (_path: string, init?: RequestInit) => { reads++; signal = init?.signal ?? undefined; return pending; };
    const render = () => kind === "chat" ? <TaskArtifactCard artifact={artifact} artifactFetch={fetcher} t={t} />
      : <AippTaskActivityCard item={item} apiFetch={fetcher} t={t} lang="en" />;
    const createUrl = URL.createObjectURL; let saves = 0;
    URL.createObjectURL = () => { saves++; return "blob:should-not-be-saved"; };
    try {
      await act(async () => { view = create(render()); });
      const button = view!.root.findAllByType("button").find(button => kind === "chat" ? button.props["aria-label"] === "Download report.txt" : button.props.title === "Download")!;
      await act(async () => { button.props.onClick(); button.props.onClick(); });
      assert.equal(reads, 1, "even two callbacks before a render share one download");
      assert.ok(signal && !signal.aborted);
      await act(async () => view!.unmount()); assert.equal(signal.aborted, true);
      const body = new ReadableStream({ cancel() { cancelled++; } });
      await act(async () => finish(new Response(body)));
      assert.equal(cancelled, 1); assert.equal(saves, 0);
    } finally { await act(async () => view?.unmount()); URL.createObjectURL = createUrl; }
  });

  test(`${kind} failed downloads unlock retry and don't claim authentication failure`, async () => {
    globalThis.IS_REACT_ACT_ENVIRONMENT = true;
    let reads = 0, view: ReactTestRenderer | undefined;
    const fetcher = async () => { reads++; return new Response(null, {status:503}); };
    try {
      await act(async () => { view = create(kind === "chat" ? <TaskArtifactCard artifact={artifact} artifactFetch={fetcher} t={t} />
        : <AippTaskActivityCard item={item} apiFetch={fetcher} t={t} lang="en" />); });
      const button = () => view!.root.findAllByType("button").find(button => kind === "chat" ? button.props["aria-label"] === "Download report.txt" : button.props.title === "Download")!;
      await act(async () => button().props.onClick());
      assert.equal(button().props.disabled ?? false, false);
      assert.equal(JSON.stringify(view!.toJSON()).includes("Sign in again"), false);
      await act(async () => button().props.onClick()); assert.equal(reads, 2);
    } finally { await act(async () => view?.unmount()); }
  });
}
