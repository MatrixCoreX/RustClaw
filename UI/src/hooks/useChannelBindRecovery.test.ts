import assert from "node:assert/strict";
import test from "node:test";
import { bindSession, channelBindHarness } from "../../test/helpers/channel-bind-harness";
import { channelBindRecoveryGuidance } from "../lib/feishu-bind";

const transportCodes = [
  "channel_bind_registration_request_timeout",
  "channel_bind_registration_transport_unavailable",
  "channel_bind_registration_response_unavailable",
  "channel_bind_registration_response_too_large",
  "channel_bind_registration_response_invalid",
  "channel_bind_registration_redirect_rejected",
  "channel_bind_registration_http_status_rejected",
];
const codes = [
  "channel_bind_application_recovery_required",
  "channel_bind_registration_superseded",
  "channel_bind_registration_prepare_required",
  ...transportCodes,
];

for (const platform of ["feishu", "lark"] as const) {
  for (const language of ["zh", "en"] as const) {
    for (const code of codes) {
      test(`${platform} ${language} presents owned ${code} without automatic replay`, async (context) => {
        const h = await channelBindHarness(context, platform);
        context.after(() => h.dispose());
        const translate = (zh: string, en: string) => language === "zh" ? zh : en;
        await h.update({ t: translate });
        await h.load();
        await h.advance();
        await h.reply(1, null, transportCodes.includes(code) ? 502 : 409, code);
        assert.equal(h.current.bindError, channelBindRecoveryGuidance(new Error(code), translate));
        assert.ok(h.current.bindError);
        assert.equal(h.current.bindSession?.session_id, 1);
        await h.advance(60_000);
        assert.equal(h.pending.length, 2);
        assert.deepEqual(h.reads, []);
        const replacement = await h.begin();
        assert.equal(h.current.bindError, null);
        await h.reply(replacement.index, bindSession(platform, 2));
        await replacement.completion;
        assert.equal(h.current.bindSession?.session_id, 2);
      });
    }
  }
  for (const code of codes) {
    test(`${platform} a stale ${code} cannot appear in a new owner's view`, async (context) => {
      const h = await channelBindHarness(context, platform);
      context.after(() => h.dispose());
      await h.load();
      await h.advance();
      await h.update({ requestScope: "device-b:admin:epoch-1" });
      await h.reply(1, null, transportCodes.includes(code) ? 502 : 409, code);
      assert.equal(h.current.bindError, null);
      assert.equal(h.current.bindSession, null);
      assert.deepEqual(h.reads, []);
    });
  }
}

test("channel recovery guidance never parses arbitrary human diagnostics", () => {
  assert.equal(channelBindRecoveryGuidance(new Error("Please retry after recovery"), (_zh, en) => en), null);
  assert.equal(channelBindRecoveryGuidance(new Error("channel_bind_future_error"), (_zh, en) => en), null);
});

const sessionCodes = [
  "channel_bind_registration_access_denied",
  "channel_bind_registration_rejected",
  "channel_bind_registration_start_failed",
];

for (const platform of ["feishu", "lark"] as const) {
  for (const code of sessionCodes) {
    for (const language of ["zh", "en"] as const) for (const phase of ["begin", "poll"] as const) {
      test(`${platform} ${language} ${phase} failed session has owned recovery guidance for ${code}`, async context => {
        const h = await channelBindHarness(context, platform);
        context.after(() => h.dispose());
        const translate = (zh: string, en: string) => language === "zh" ? zh : en;
        await h.update({ t: translate });
        const failed = { ...bindSession(platform), status: "failed", error_text: code };
        if (phase === "begin") {
          const begin = await h.begin();
          await h.reply(begin.index, failed);
          await begin.completion;
        } else {
          await h.load();
          await h.advance();
          await h.reply(1, failed);
        }
        const guidance = channelBindRecoveryGuidance(code, translate);
        assert.ok(guidance);
        assert.equal(h.current.bindError, guidance);
        assert.equal(h.current.bindSession?.status, "failed");
        const requests = h.pending.length;
        await h.advance(60_000);
        assert.equal(h.pending.length, requests);
        assert.deepEqual(h.reads, []);
      });
    }
    test(`${platform} a stale failed session with ${code} cannot enter the new owner's view`, async context => {
      const h = await channelBindHarness(context, platform);
      context.after(() => h.dispose());
      await h.load();
      await h.advance();
      await h.update({ requestScope: "device-b:admin:epoch-1" });
      await h.reply(1, { ...bindSession(platform), status: "failed", error_text: code });
      assert.equal(h.current.bindError, null);
      assert.equal(h.current.bindSession, null);
      assert.deepEqual(h.reads, []);
    });
  }
}
