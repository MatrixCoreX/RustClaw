const assert = require("assert");

const {
  bindIdentity,
  buildSubmitTaskBody,
  canonicalUserJid,
  extractBindKeyCandidate,
  extractTextContent,
  isGroupJid,
  isLoopbackHost,
  isSameAccountJid,
  messageTimestampSeconds,
  normalizeInternalApiBaseUrl,
  parseListenAddress,
  queryTask,
  resolveIdentity,
  stableUserId,
  submitTask,
  shouldProcessUpsertMessage,
} = require("./index.js");

assert.strictEqual(extractBindKeyCandidate("/key rk-admin", false), "rk-admin");
assert.strictEqual(extractBindKeyCandidate("rk-admin", true), "rk-admin");
assert.strictEqual(extractBindKeyCandidate("hello", false), null);
assert.strictEqual(extractBindKeyCandidate("/key", true), null);
assert.strictEqual(extractTextContent({ extendedTextMessage: { text: " hello " } }), "hello");
assert.strictEqual(isGroupJid("group@g.us"), true);
assert.strictEqual(isGroupJid("user@s.whatsapp.net"), false);
assert.deepStrictEqual(parseListenAddress("127.0.0.1:8092", 1), { host: "127.0.0.1", port: 8092 });
assert.deepStrictEqual(parseListenAddress("[::1]:8092", 1), { host: "::1", port: 8092 });
assert.strictEqual(isLoopbackHost("::1"), true);
assert.strictEqual(isLoopbackHost("0.0.0.0"), false);
assert.strictEqual(normalizeInternalApiBaseUrl("http://127.0.0.1:8787/"), "http://127.0.0.1:8787");
assert.strictEqual(normalizeInternalApiBaseUrl("https://internal.example:9443"), "https://internal.example:9443");
assert.throws(() => normalizeInternalApiBaseUrl(""), /missing/);
assert.throws(() => normalizeInternalApiBaseUrl("ws://127.0.0.1:8787"), /http or https/);
assert.throws(() => normalizeInternalApiBaseUrl("http://user:pass@127.0.0.1:8787"), /credentials/);
assert.throws(() => normalizeInternalApiBaseUrl("http://127.0.0.1:8787/v1"), /must not contain a path/);
assert.strictEqual(canonicalUserJid("8613800138000:7@s.whatsapp.net"), "8613800138000@s.whatsapp.net");
assert.strictEqual(canonicalUserJid("8613800138000@s.whatsapp.net"), "8613800138000@s.whatsapp.net");
assert.strictEqual(
  isSameAccountJid("12345@lid", "8613800138000:7@s.whatsapp.net", "12345:7@lid"),
  true
);
assert.strictEqual(
  isSameAccountJid("other@lid", "8613800138000:7@s.whatsapp.net", "12345:7@lid"),
  false
);
assert.strictEqual(messageTimestampSeconds({ messageTimestamp: "123" }), 123);
assert.strictEqual(messageTimestampSeconds({ messageTimestamp: { toNumber: () => 456 } }), 456);
assert.strictEqual(messageTimestampSeconds({}), null);
const upsertStartedAtMs = 1_000_000;
const recentSelfAppend = {
  key: { fromMe: true, remoteJid: "12345@lid" },
  messageTimestamp: 1005,
};
assert.strictEqual(
  shouldProcessUpsertMessage(
    recentSelfAppend,
    "append",
    "8613800138000:7@s.whatsapp.net",
    "12345:7@lid",
    upsertStartedAtMs,
    1_006_000
  ),
  true
);
assert.strictEqual(
  shouldProcessUpsertMessage(
    { ...recentSelfAppend, messageTimestamp: 900 },
    "append",
    "8613800138000:7@s.whatsapp.net",
    "12345:7@lid",
    upsertStartedAtMs,
    1_006_000
  ),
  false
);
assert.strictEqual(
  shouldProcessUpsertMessage(
    { ...recentSelfAppend, key: { fromMe: true, remoteJid: "other@lid" } },
    "append",
    "8613800138000:7@s.whatsapp.net",
    "12345:7@lid",
    upsertStartedAtMs,
    1_006_000
  ),
  false
);
assert.strictEqual(
  shouldProcessUpsertMessage({}, "notify", "own@s.whatsapp.net", "own@lid", 0, 0),
  true
);
assert.strictEqual(stableUserId("user@s.whatsapp.net"), stableUserId("user@s.whatsapp.net"));

const body = buildSubmitTaskBody(
  "user@s.whatsapp.net",
  "group@g.us",
  "ask",
  { text: "hello" },
  { user_key: "rk-admin", user_id: 42, role: "admin" }
);
assert.strictEqual(body.user_id, 42);
assert.strictEqual(body.user_key, "rk-admin");
assert.strictEqual(body.channel, "whatsapp");
assert.strictEqual(body.external_user_id, "user@s.whatsapp.net");
assert.strictEqual(body.external_chat_id, "group@g.us");
assert.strictEqual(body.payload.adapter, "whatsapp_web");
assert.strictEqual(body.payload.text, "hello");
assert.throws(
  () => buildSubmitTaskBody("user", "chat", "ask", { text: "x" }, null),
  /bound user key is required/
);

(async () => {
  const calls = [];
  global.fetch = async (url, options = {}) => {
    calls.push({ url: String(url), options });
    if (String(url).endsWith("/v1/auth/channel/resolve")) {
      return {
        ok: true,
        status: 200,
        json: async () => ({ ok: true, data: { identity: { user_key: "rk-admin", user_id: 42 } } }),
      };
    }
    if (String(url).endsWith("/v1/auth/channel/bind")) {
      return {
        ok: true,
        status: 200,
        json: async () => ({ ok: true, data: { user_key: "rk-admin", user_id: 42 } }),
      };
    }
    if (String(url).endsWith("/v1/tasks")) {
      return {
        ok: true,
        status: 200,
        text: async () => JSON.stringify({ ok: true, data: { task_id: "task-1" } }),
      };
    }
    return {
      ok: true,
      status: 200,
      text: async () => JSON.stringify({ ok: true, data: { status: "succeeded" } }),
    };
  };

  const identity = await resolveIdentity("user@s.whatsapp.net", "group@g.us");
  assert.strictEqual(identity.user_key, "rk-admin");
  const bound = await bindIdentity("user@s.whatsapp.net", "group@g.us", "rk-admin");
  assert.strictEqual(bound.user_id, 42);
  const taskId = await submitTask(
    "user@s.whatsapp.net",
    "group@g.us",
    "ask",
    { text: "hello" },
    identity
  );
  assert.strictEqual(taskId, "task-1");
  await queryTask(taskId, identity);

  const resolveBody = JSON.parse(calls[0].options.body);
  assert.strictEqual(resolveBody.external_user_id, "user@s.whatsapp.net");
  assert.strictEqual(resolveBody.external_chat_id, "group@g.us");
  const submitCall = calls.find((call) => call.url.endsWith("/v1/tasks"));
  assert.strictEqual(submitCall.options.headers["x-agent-key"], "rk-admin");
  const queryCall = calls.find((call) => call.url.endsWith("/v1/tasks/task-1"));
  assert.strictEqual(queryCall.options.headers["x-agent-key"], "rk-admin");

  console.log("WA_WEB_BRIDGE_TESTS ok");
})().catch((error) => {
  console.error(error);
  process.exit(1);
});
