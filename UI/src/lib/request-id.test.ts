import assert from "node:assert/strict";
import test from "node:test";
import { newRequestUuid } from "./request-id";

test("request UUIDs use native browser support when available", () => {
  const id = "00000000-0000-4000-8000-000000000001";
  assert.equal(newRequestUuid({ randomUUID: () => id, getRandomValues: () => { throw Error("unexpected"); } }), id);
});
test("LAN HTTP fallback uses random bytes and RFC version/variant bits", () => {
  assert.equal(newRequestUuid({ getRandomValues: array => { if (array) new Uint8Array(array.buffer, array.byteOffset, array.byteLength).fill(255); return array; } }), "ffffffff-ffff-4fff-bfff-ffffffffffff");
  const source = { getRandomValues: globalThis.crypto.getRandomValues.bind(globalThis.crypto) };
  const ids = new Set(Array.from({ length: 100 }, () => newRequestUuid(source)));
  assert.equal(ids.size, 100);
  for (const id of ids) assert.match(id, /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
});
