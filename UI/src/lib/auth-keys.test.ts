import test from "node:test";
import assert from "node:assert/strict";

import { copyAuthKeyValue, maskStoredKey, writeTextToClipboard } from "./auth-keys.ts";

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

test("uses clipboard api when available", async () => {
  const writes: string[] = [];

  await writeTextToClipboard("rk-plain", {
    clipboard: {
      writeText: async (value) => {
        writes.push(value);
      },
    },
  });

  assert.deepEqual(writes, ["rk-plain"]);
});

test("falls back to execCommand copy when clipboard api is unavailable", async () => {
  const operations: string[] = [];
  const textarea = {
    value: "",
    setAttribute: (name: string, value: string) => {
      operations.push(`set:${name}=${value}`);
    },
    style: {} as Record<string, string>,
    focus: () => {
      operations.push("focus");
    },
    select: () => {
      operations.push("select");
    },
  };

  await writeTextToClipboard("rk-fallback", {
    document: {
      body: {
        appendChild: () => {
          operations.push("append");
        },
        removeChild: () => {
          operations.push("remove");
        },
      },
      createElement: (tag) => {
        assert.equal(tag, "textarea");
        return textarea;
      },
      execCommand: (command) => {
        operations.push(`exec:${command}`);
        return true;
      },
    },
  });

  assert.equal(textarea.value, "rk-fallback");
  assert.deepEqual(operations, ["set:readonly=", "append", "focus", "select", "exec:copy", "remove"]);
});

test("masks stored auth keys for display", () => {
  assert.equal(maskStoredKey("abcdef123456", 4), "abcd********");
  assert.equal(maskStoredKey("  "), "");
});
