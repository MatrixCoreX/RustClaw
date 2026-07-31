const assert = require("assert");
const fs = require("fs");
const os = require("os");
const path = require("path");

const {
  adapterDiagnosticId,
  adapterError,
  bindIdentity,
  buildSubmitTaskBody,
  canonicalUserJid,
  extractBindKeyCandidate,
  extractTextContent,
  isGroupJid,
  isLoopbackHost,
  isSameAccountJid,
  deliverySourceAllowed,
  loginStatusSnapshot,
  messageTimestampSeconds,
  normalizeDeliverySource,
  normalizeInternalApiBaseUrl,
  parseListenAddress,
  queryTask,
  resolveIdentity,
  stableUserId,
  submitTask,
  updateLoginState,
  validateOutboundFile,
  shouldProcessUpsertMessage,
} = require("./index.js");

assert.strictEqual(normalizeDeliverySource(" Scheduled_Task "), "scheduled_task");
assert.strictEqual(deliverySourceAllowed("scheduled_task", false), false);
assert.strictEqual(deliverySourceAllowed("proactive_notice", false), false);
assert.strictEqual(deliverySourceAllowed("scheduled_task", true), true);
assert.strictEqual(deliverySourceAllowed("background_completion", false), true);
assert.strictEqual(deliverySourceAllowed("immediate_daemon", false), true);
assert.strictEqual(deliverySourceAllowed("unknown", false), false);
const adapterFailure = adapterError(
  "adapter_send_failed",
  "send_text",
  "private provider prose"
);
assert.strictEqual(adapterFailure.error_code, "adapter_send_failed");
assert.match(adapterFailure.diagnostic_id, /^whatsapp-web:[a-f0-9]{24}$/);
assert.strictEqual(adapterFailure.retryable, false);
assert.strictEqual(JSON.stringify(adapterFailure).includes("private provider prose"), false);
assert.strictEqual(
  adapterDiagnosticId("send_text", "same"),
  adapterDiagnosticId("send_text", "same")
);
const adapterStatus = loginStatusSnapshot();
assert.strictEqual(adapterStatus.adapter_mode, "experimental_unofficial");
assert.strictEqual(adapterStatus.official_bot_api, false);
assert.strictEqual(adapterStatus.transport, "baileys");
assert.strictEqual(adapterStatus.proactive_send_enabled, false);
assert.strictEqual(adapterStatus.local_safety_limits.image_bytes, 100 * 1024 * 1024);
assert.strictEqual(adapterStatus.last_error, undefined);
updateLoginState("reconnecting", {
  errorCode: "connection_closed",
  operation: "connection_update",
  diagnosticMaterial: "private disconnect prose",
});
const reconnectingStatus = loginStatusSnapshot();
assert.strictEqual(reconnectingStatus.phase, "reconnecting");
assert.strictEqual(reconnectingStatus.connected, false);
assert.strictEqual(reconnectingStatus.last_error_code, "connection_closed");
assert.match(reconnectingStatus.last_diagnostic_id, /^whatsapp-web:[a-f0-9]{24}$/);
assert.strictEqual(JSON.stringify(reconnectingStatus).includes("private disconnect prose"), false);
updateLoginState("connected", { clearQr: true });
const connectedStatus = loginStatusSnapshot();
assert.strictEqual(connectedStatus.phase, "connected");
assert.strictEqual(connectedStatus.connected, true);
assert.strictEqual(connectedStatus.last_error_code, null);

const mediaFixtureDir = fs.mkdtempSync(path.join(os.tmpdir(), "wa-web-outbound-media-"));
const videoFixture = path.join(mediaFixtureDir, "video.mp4");
fs.writeFileSync(videoFixture, "video");
assert.strictEqual(validateOutboundFile(videoFixture, "视频", 100), 5);
assert.throws(
  () => validateOutboundFile(videoFixture, "视频", 4),
  /本地安全上限/
);
fs.rmSync(mediaFixtureDir, { recursive: true, force: true });

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
  { user_key: "rk-admin", user_id: 42, role: "admin" },
  "message-1"
);
assert.strictEqual(body.user_id, 42);
assert.strictEqual(body.user_key, "rk-admin");
assert.strictEqual(body.channel, "whatsapp");
assert.strictEqual(body.external_user_id, "user@s.whatsapp.net");
assert.strictEqual(body.external_chat_id, "group@g.us");
assert.strictEqual(body.payload.adapter, "whatsapp_web");
assert.strictEqual(body.payload.text, "hello");
assert.strictEqual(body.ingress.adapter, "whatsapp_web");
assert.strictEqual(body.ingress.message_id, "message-1");
assert.strictEqual(body.idempotency_key, "whatsapp_web:message-1");
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
        json: async () => ({
          ok: true,
          data: {
            identity: { user_key: "rk-admin", user_id: 42 },
            pending_resume: null,
          },
        }),
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
  assert.strictEqual(bound.identity.user_id, 42);
  const taskId = await submitTask(
    "user@s.whatsapp.net",
    "group@g.us",
    "ask",
    { text: "hello" },
    identity,
    "message-2"
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
