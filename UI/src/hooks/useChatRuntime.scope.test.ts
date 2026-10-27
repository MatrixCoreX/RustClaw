import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import test from "node:test";
import React from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { UiDialogProvider } from "../components/UiDialogProvider";
import { DEFAULT_CHAT_ATTACHMENT_CONSTRAINTS } from "../lib/chat-attachments";
import { useChatRuntime, type UseChatRuntimeParams } from "./useChatRuntime";

const response = (data: unknown) => new Response(JSON.stringify({ ok: true, data }));
const history = () => ({ schema_version: 1, status: "ok", turns: [], truncated: true,
  next_cursor: "earlier", content_sha256: createHash("sha256").update("[]").digest("hex") });

async function probe() {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  const requests: Array<{ path: string; resolve: (response: Response) => void }> = [];
  const params: UseChatRuntimeParams = {
    apiFetch: async path => {
      if (path === "/v1/ui/attachment-constraints") return response(DEFAULT_CHAT_ATTACHMENT_CONSTRAINTS);
      if (path.startsWith("/v1/tasks/conversation-history") && !path.includes("cursor=")) return response(history());
      if (path.startsWith("/v1/conversation-inputs?")) return response({ schema_version: 1, items: [], next_after_input_seq: null });
      if (path.includes("/events?")) return new Response(new ReadableStream({ start() {} }), { headers: { "content-type": "text/event-stream" } });
      return new Promise(resolve => requests.push({ path, resolve }));
    },
    t: (_zh, en) => en, lang: "en", interactionAdapter: "generic", interactionChannel: "ui",
    activeUserKey: "fixture-one", activeIdentityIds: {}, conversationHistoryScope: "node-one:owner-one",
    interactionExternalUserId: "", interactionExternalChatId: "", availableAgents: [{ id: "main", name: "Main" }],
    fetchTaskById: async id => ({ task_id: id, status: "succeeded", result_json: null, error_text: null }),
    onTaskSubmitted: () => undefined, onTaskResult: () => undefined,
  };
  let runtime!: ReturnType<typeof useChatRuntime>;
  function Probe() { runtime = useChatRuntime(params); return null; }
  const tree = () => React.createElement(UiDialogProvider, null, React.createElement(Probe));
  let renderer!: ReactTestRenderer;
  await act(async () => { renderer = create(tree()); });
  return {
    get runtime() { return runtime; }, requests,
    update: async (change: Partial<UseChatRuntimeParams>) => {
      Object.assign(params, change);
      await act(async () => { renderer.update(tree()); });
    },
    close: async () => { await act(async () => renderer.unmount()); },
  };
}

for (const mode of ["owner", "node", "logout", "roundtrip", "login-epoch"] as const) {
  test(`chat history and teaching responses cannot cross scope: ${mode}`, async t => {
    const p = await probe();
    t.after(() => p.close());
    const old = p.runtime;
    let page!: Promise<unknown>;
    let trace!: Promise<unknown>;
    await act(async () => {
      page = old.loadEarlierConversationHistory();
      trace = old.queryChatTeachingLlmDebug("private-task");
    });
    assert.equal(p.requests.length, 2);
    await p.update(mode === "login-epoch" ? { requestEpoch: 1 } : mode === "node" ? { conversationHistoryScope: "node-two:owner-one" }
      : mode === "logout" ? { conversationHistoryScope: "", activeUserKey: "" }
      : { conversationHistoryScope: "node-one:owner-two", activeUserKey: "fixture-two" });
    if (mode === "roundtrip") await p.update({ conversationHistoryScope: "node-one:owner-one", activeUserKey: "fixture-one" });
    let currentTrace!: Promise<unknown>;
    if (mode !== "logout") await act(async () => { currentTrace = p.runtime.queryChatTeachingLlmDebug("current-task"); });
    await act(async () => {
      p.requests[0].resolve(new Response(JSON.stringify({ ok: false, error: "old-page-error" }), { status: 500 }));
      p.requests[1].resolve(response({ calls: [{ response: "private-old-trace" }], call_count: 1 }));
      await Promise.all([page, trace]);
    });
    assert.equal(p.runtime.chatError, null);
    assert.equal(p.runtime.chatTeachingLlmDebug, null);
    assert.equal(p.runtime.chatTeachingLlmDebugLoading, mode !== "logout", "obsolete finally must not clear current loading");
    const count = p.requests.length;
    await act(async () => {
      await old.queryChatTeachingLlmDebug("must-not-request");
      await old.compactChatContext();
      old.createNewChatThread();
    });
    assert.equal(p.requests.length, count, "obsolete callbacks cannot initiate requests");
    if (mode !== "logout") await act(async () => { p.requests[2].resolve(response({ calls: [], call_count: 0 })); await currentTrace; });
    assert.equal(p.runtime.chatTeachingLlmDebugLoading, false);
  });
}

test("newer teaching selection wins and unmount invalidates saved callbacks", async () => {
  const p = await probe();
  let first!: Promise<unknown>;
  let second!: Promise<unknown>;
  await act(async () => { first = p.runtime.queryChatTeachingLlmDebug("task-first"); });
  await act(async () => { second = p.runtime.queryChatTeachingLlmDebug("task-second"); });
  await act(async () => {
    p.requests[1].resolve(response({ calls: [], call_count: 2 })); await second;
    p.requests[0].resolve(new Response("invalid JSON")); await first;
  });
  assert.equal(p.runtime.chatTeachingLlmDebugError, null);
  assert.equal(p.runtime.chatTeachingLlmDebugLoading, false);
  const saved = p.runtime;
  await p.close();
  await saved.queryChatTeachingLlmDebug("after-unmount");
  assert.equal(p.requests.length, 2);
});

test("late microphone permission stops its tracks without recording for another owner", async () => {
  const oldWindow = Object.getOwnPropertyDescriptor(globalThis, "window");
  const oldNavigator = Object.getOwnPropertyDescriptor(globalThis, "navigator");
  let grant!: (stream: MediaStream) => void;
  let stopped = 0;
  Object.defineProperty(globalThis, "window", { configurable: true, value: {
    isSecureContext: true, AudioContext: class { constructor() { throw new Error("obsolete recorder started"); } },
    setTimeout, clearTimeout, setInterval, clearInterval,
  } });
  Object.defineProperty(globalThis, "navigator", { configurable: true, value: {
    mediaDevices: { getUserMedia: () => new Promise<MediaStream>(resolve => { grant = resolve; }) },
  } });
  let p: Awaited<ReturnType<typeof probe>> | undefined;
  try {
    p = await probe();
    let recording!: Promise<void>;
    await act(async () => { recording = p!.runtime.startChatVoiceRecording(); });
    assert.equal(p.runtime.chatRecording, true);
    await p.update({ conversationHistoryScope: "node-one:owner-two", activeUserKey: "fixture-two" });
    await act(async () => {
      grant({ getTracks: () => [{ stop: () => { stopped += 1; } }] } as unknown as MediaStream);
      await recording;
    });
    assert.equal(stopped, 1);
    assert.equal(p.runtime.chatRecording, false);
    assert.equal(p.runtime.chatError, null);
    assert.equal(p.requests.length, 0);
  } finally {
    await p?.close();
    if (oldWindow) Object.defineProperty(globalThis, "window", oldWindow);
    else Reflect.deleteProperty(globalThis, "window");
    if (oldNavigator) Object.defineProperty(globalThis, "navigator", oldNavigator);
    else Reflect.deleteProperty(globalThis, "navigator");
  }
});
