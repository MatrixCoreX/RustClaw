import assert from "node:assert/strict";
import test from "node:test";
import { ChatMessageQueue } from "./chat-message-queue";

const flush = () => new Promise<void>(resolve => setImmediate(resolve));
const message = (id: string, threadId = "a") => ({ id, threadId, text: id, attachments: [] });
function deferred() {
  let resolve!: (value: boolean) => void;
  const promise = new Promise<boolean>(done => { resolve = done; });
  return { promise, resolve };
}

test("FIFO within a conversation; other conversations can run independently", async () => {
  const queue = new ChatMessageQueue();
  const first = deferred(), second = deferred();
  const started: string[] = [];
  queue.enqueue(message("1"), async () => { started.push("1"); return first.promise; });
  queue.enqueue(message("2"), async () => { started.push("2"); return second.promise; });
  queue.enqueue(message("3"), async () => { started.push("3"); return true; });
  queue.enqueue(message("other", "b"), async () => { started.push("other"); return true; });
  assert.deepEqual(started, ["1", "other"]);
  first.resolve(true);
  await flush();
  assert.deepEqual(started, ["1", "other", "2"]);
  second.resolve(true);
  await flush();
  assert.deepEqual(started, ["1", "other", "2", "3"]);
  assert.deepEqual(queue.getSnapshot().messages, []);
});

test("restored tasks and compaction block their own conversation only", async () => {
  const queue = new ChatMessageQueue();
  queue.setBlockedThreads(["a"]);
  const started: string[] = [];
  queue.enqueue(message("1"), async () => { started.push("1"); return true; });
  queue.enqueue(message("2", "b"), async () => { started.push("2"); return true; });
  await flush();
  assert.deepEqual(started, ["2"]);
  queue.setBlockedThreads([]);
  await flush();
  assert.deepEqual(started, ["2", "1"]);
});

test("failure pauses dependent messages until explicit continuation", async () => {
  for (const throws of [true, false]) {
    const queue = new ChatMessageQueue();
    let calls = 0;
    queue.enqueue(message("1"), async () => { if (throws) throw Error("transport"); return false; });
    queue.enqueue(message("2"), async () => { calls++; return true; });
    await flush();
    assert.equal(calls, 0);
    assert.deepEqual(queue.getSnapshot().paused, ["a"]);
    queue.resume("a");
    await flush();
    assert.equal(calls, 1);
  }
});

test("removing a pending message never cancels an active task or reorders others", async () => {
  const queue = new ChatMessageQueue();
  const first = deferred();
  const started: string[] = [];
  queue.enqueue(message("1"), async () => first.promise);
  queue.enqueue(message("2"), async () => { started.push("2"); return true; });
  queue.enqueue(message("3"), async () => { started.push("3"); return true; });
  queue.remove("1");
  queue.remove("2");
  assert.deepEqual(queue.getSnapshot().messages.map(item => item.id), ["1", "3"]);
  first.resolve(true);
  await flush();
  assert.deepEqual(started, ["3"]);
});

test("deleting a conversation drops its pending messages without affecting another", async () => {
  const queue = new ChatMessageQueue();
  queue.setBlockedThreads(["a", "b"]);
  queue.enqueue(message("1"), async () => true);
  queue.enqueue(message("2", "b"), async () => true);
  queue.removeThread("a");
  assert.deepEqual(queue.getSnapshot().messages.map(item => item.id), ["2"]);
});

test("account switch/unmount aborts followers and never submits remaining messages", async () => {
  const queue = new ChatMessageQueue();
  const first = deferred();
  let signal!: AbortSignal;
  let calls = 0;
  queue.enqueue(message("1"), async value => { signal = value; return first.promise; });
  queue.enqueue(message("2"), async () => { calls++; return true; });
  queue.dispose();
  assert.equal(signal.aborted, true);
  first.resolve(true);
  await flush();
  assert.equal(calls, 0);
  assert.equal(queue.enqueue(message("3"), async () => true), false);
});

test("duplicate queue IDs are rejected; development effect remount can reactivate", async () => {
  const queue = new ChatMessageQueue();
  queue.setBlockedThreads(["a"]);
  assert.equal(queue.enqueue(message("1"), async () => true), true);
  assert.equal(queue.enqueue(message("1"), async () => true), false);
  queue.dispose();
  queue.activate();
  assert.equal(queue.enqueue(message("2"), async () => true), true);
  queue.setBlockedThreads([]);
  await flush();
  assert.equal(queue.getSnapshot().messages.length, 0);
});

test("waiting for confirmation is a barrier, not a failure, and resumes after authoritative completion", async () => {
  const queue = new ChatMessageQueue();
  let calls = 0;
  queue.enqueue(message("1"), async () => {
    queue.setBlockedThreads(["a"]);
    return "waiting";
  });
  queue.enqueue(message("2"), async () => { calls++; return true; });
  await flush();
  assert.equal(calls, 0);
  assert.deepEqual(queue.getSnapshot().paused, []);
  queue.setBlockedThreads([]);
  await flush();
  assert.equal(calls, 1);
});
