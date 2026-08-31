import test from "node:test";
import assert from "node:assert/strict";

import {
  copyAuthKeyValue,
  formatAuthenticationError,
  maskStoredKey,
  restorePersistedAuthMode,
  responseIndicatesExpiredAuthentication,
  writeTextToClipboard,
} from "./auth-keys.ts";

test("direct auth keys are purged instead of restored from browser storage", () => {
  const values = new Map<string, string>([
    ["auth-mode", "key"],
    ["direct-key", "rk-persisted-secret"],
  ]);
  const storage = {
    getItem: (key: string) => values.get(key) ?? null,
    removeItem: (key: string) => values.delete(key),
  };

  assert.equal(restorePersistedAuthMode(storage, "direct-key", "auth-mode"), null);
  assert.equal(values.has("direct-key"), false);
  assert.equal(values.has("auth-mode"), false);

  values.set("auth-mode", "webd");
  assert.equal(restorePersistedAuthMode(storage, "direct-key", "auth-mode"), "webd");
  assert.equal(values.get("auth-mode"), "webd");
});

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

test("invalidates only structured authentication failures", async () => {
  assert.equal(
    await responseIndicatesExpiredAuthentication(
      Response.json({ ok: false, data: { error_code: "auth_key_invalid" } }, { status: 401 }),
    ),
    true,
  );
  assert.equal(
    await responseIndicatesExpiredAuthentication(
      Response.json({ ok: false, error: "task_owner_mismatch" }, { status: 401 }),
    ),
    false,
  );
  assert.equal(
    await responseIndicatesExpiredAuthentication(
      Response.json({ ok: false, error: "auth_key_invalid" }, { status: 403 }),
    ),
    false,
  );
});

test("localizes stable authentication codes without matching natural-language errors", () => {
  const t = (zh: string, _en: string) => zh;
  assert.equal(
    formatAuthenticationError("auth_key_invalid", 401, t),
    "访问凭证无效或已停用，请重新登录。",
  );
  assert.equal(
    formatAuthenticationError("auth_key_required", 401, t),
    "请先登录。",
  );
  assert.equal(
    formatAuthenticationError("upstream_unavailable", 503, t),
    "登录服务暂时不可用，请稍后重试。",
  );
  assert.equal(
    formatAuthenticationError("webd_login_upstream_unavailable", 502, t),
    "登录服务暂时无法连接核心服务，请稍后重试。",
  );
  assert.equal(
    formatAuthenticationError("future_auth_machine_code", 500, t),
    "身份验证失败 (500)，请重新登录。",
  );
});
