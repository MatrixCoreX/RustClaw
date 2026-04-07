import test from "node:test";
import assert from "node:assert/strict";

import { copyAuthKeyValue } from "./auth-keys.ts";

test("copies plaintext key directly when it is already available", async () => {
  const writes: string[] = [];
  let fetched = false;

  const copied = await copyAuthKeyValue({
    plaintextKey: "rk-plain",
    fetchFullAuthKey: async () => {
      fetched = true;
      return "rk-fetched";
    },
    writeClipboard: async (value) => {
      writes.push(value);
    },
  });

  assert.equal(copied, "rk-plain");
  assert.deepEqual(writes, ["rk-plain"]);
  assert.equal(fetched, false);
});

test("fetches and copies the full key when only key id is available", async () => {
  const writes: string[] = [];

  const copied = await copyAuthKeyValue({
    keyId: 42,
    fetchFullAuthKey: async (keyId) => {
      assert.equal(keyId, 42);
      return "rk-full";
    },
    writeClipboard: async (value) => {
      writes.push(value);
    },
  });

  assert.equal(copied, "rk-full");
  assert.deepEqual(writes, ["rk-full"]);
});

test("throws when neither plaintext key nor key id is provided", async () => {
  await assert.rejects(
    copyAuthKeyValue({
      fetchFullAuthKey: async () => "rk-full",
      writeClipboard: async () => undefined,
    }),
    /missing auth key id/,
  );
});
