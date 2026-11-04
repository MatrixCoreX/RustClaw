import assert from "node:assert/strict";
import test from "node:test";
import { AIPP_DOWNLOAD_MAX_BYTES, AIPP_IMAGE_MAX_BYTES, downloadAippGallery, fetchAippImage, fetchAippImageDownload } from "./aipp-image-download";

function clockFixture() {
  const originalSet = globalThis.setTimeout, originalClear = globalThis.clearTimeout;
  let now = 0, next = 0;
  const timers = new Map<number, { at: number; callback: () => void }>();
  globalThis.setTimeout = ((callback: () => void, delay = 0) => {
    const id = ++next; timers.set(id, { at: now + delay, callback }); return id;
  }) as unknown as typeof setTimeout;
  globalThis.clearTimeout = ((id: unknown) => { timers.delete(Number(id)); }) as typeof clearTimeout;
  return {
    timers,
    advance: (ms: number) => {
      now += ms;
      for (const [id, timer] of [...timers]) if (timer.at <= now) { timers.delete(id); timer.callback(); }
    },
    restore: () => { globalThis.setTimeout = originalSet; globalThis.clearTimeout = originalClear; },
  };
}

function streamedImage() {
  let controller: ReadableStreamDefaultController<Uint8Array>;
  let signal: AbortSignal | null | undefined;
  let requests = 0;
  const body = new ReadableStream<Uint8Array>({ start(value) { controller = value; } });
  return {
    body,
    get signal() { assert.ok(signal); return signal; },
    get requests() { return requests; },
    enqueue: (bytes: Uint8Array) => controller.enqueue(bytes),
    close: () => controller.close(),
    fetch: async (_path: string, init?: RequestInit) => {
      requests++; signal = init?.signal; assert.ok(signal);
      signal.addEventListener("abort", () => controller.error(signal!.reason), { once: true });
      return new Response(body, { headers: { "content-type": "image/png" } });
    },
  };
}
const settled = () => new Promise<void>(resolve => setImmediate(resolve));

test("healthy original downloads may exceed a minute when bytes keep arriving", async () => {
  const clock = clockFixture();
  try {
    const stream = streamedImage();
    const result = fetchAippImageDownload(stream.fetch, "/original", new AbortController().signal);
    await settled();
    for (let i = 0; i < 3; i++) {
      clock.advance(40_000); assert.equal(stream.signal.aborted, false);
      stream.enqueue(new Uint8Array([i])); await settled();
    }
    stream.close();
    assert.equal((await result).size, 3);
    assert.equal(clock.timers.size, 0);
    assert.equal(stream.body.locked, false);
  } finally { clock.restore(); }
});

test("an idle body stops, releases its reader and returns a stable timeout reason without retrying", async () => {
  const clock = clockFixture();
  try {
    const stream = streamedImage();
    const failed = assert.rejects(fetchAippImageDownload(stream.fetch, "/original", new AbortController().signal), /aipp_image_idle_timeout/u);
    await settled(); clock.advance(60_001); await failed;
    assert.equal(stream.signal.aborted, true);
    assert.equal(stream.requests, 1);
    assert.equal(stream.body.locked, false);
    assert.equal(clock.timers.size, 0);
  } finally { clock.restore(); }
});

test("zero-byte chunks do not keep a stalled image download alive", async () => {
  const clock = clockFixture();
  try {
    const stream = streamedImage();
    const failed = assert.rejects(fetchAippImageDownload(stream.fetch, "/original", new AbortController().signal), /aipp_image_idle_timeout/u);
    await settled(); clock.advance(59_999);
    stream.enqueue(new Uint8Array()); await settled();
    clock.advance(2); await failed;
    assert.equal(clock.timers.size, 0);
  } finally { clock.restore(); }
});

test("explicit cancellation preserves its reason, stops the body and removes timers", async () => {
  const clock = clockFixture();
  try {
    const stream = streamedImage(), controller = new AbortController();
    const reason = new Error("explicit_download_cancel");
    const failed = assert.rejects(fetchAippImageDownload(stream.fetch, "/original", controller.signal), error => error === reason);
    await settled(); controller.abort(reason); await failed;
    assert.equal(stream.requests, 1);
    assert.equal(stream.body.locked, false);
    assert.equal(clock.timers.size, 0);
  } finally { clock.restore(); }
});

test("original and gallery downloads accept images above the preview budget without removing memory protection", async () => {
  const bytes = new Uint8Array(AIPP_IMAGE_MAX_BYTES + 1); bytes[0] = 7; bytes[bytes.length - 1] = 9;
  const fetch = async () => new Response(bytes, { headers: { "content-type": "image/png", "content-length": String(bytes.length) } });
  const signal = new AbortController().signal;
  await assert.rejects(fetchAippImage(fetch, "/preview", signal), /aipp_image_size_limit/u);
  const original = await fetchAippImageDownload(fetch, "/original", signal);
  assert.equal(original.size, bytes.length);
  assert.deepEqual(new Uint8Array(await original.slice(-1).arrayBuffer()), new Uint8Array([9]));
  const zip = await downloadAippGallery([{ title: "Large", filename: "large.png", previewUrl: "/preview", downloadUrl: "/original" }], fetch, signal, () => {});
  assert.ok(zip.size > bytes.length);
  assert.ok(zip.size < AIPP_DOWNLOAD_MAX_BYTES);
});

test("oversized originals are rejected from headers or actual chunks with a machine-readable budget error", async () => {
  const signal = new AbortController().signal;
  await assert.rejects(fetchAippImageDownload(async () => new Response("x", { headers: {
    "content-type": "image/png", "content-length": String(AIPP_DOWNLOAD_MAX_BYTES + 1),
  } }), "/original", signal), /aipp_image_size_limit/u);
  await assert.rejects(fetchAippImageDownload(async () => new Response(new Uint8Array(11), {
    headers: { "content-type": "image/png" },
  }), "/original", signal, 10), /aipp_image_size_limit/u);
});
