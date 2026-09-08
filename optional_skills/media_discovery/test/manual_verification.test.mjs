import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { waitForManualAccess } from "../src/browser.mjs";
import { handleRequest } from "../src/main.mjs";

function manualPage() {
  let challenge = true;
  const page = {
    isClosed: () => false,
    url: () => "https://www.douyin.com/video/1234",
    locator: selector => ({
      evaluateAll: async () => challenge ? ["https://www.douyin.com/verifycenter/captcha/v2"] : [],
      count: async () => selector.includes("input[") ? 0 : 1,
    }),
    waitForTimeout: async () => { challenge = false; },
  };
  return page;
}

test("manual challenge must disappear on a ready platform page before silent retry", async () => {
  const page = manualPage();
  let waits = 0;
  const wait = page.waitForTimeout;
  page.waitForTimeout = async () => { waits += 1; await wait(); };
  const result = await waitForManualAccess({ page, context: {}, platform: "douyin",
    errorCode: "challenge_required", timeoutMs: 3000 });
  assert.equal(result.ready, true);
  assert.equal(waits, 2);
});

test("closing a manual window is not authentication success and stop is interruptible", async () => {
  const page = manualPage();
  page.isClosed = () => true;
  const options = { page, context: {}, platform: "douyin", errorCode: "login_required", timeoutMs: 1000 };
  assert.equal((await waitForManualAccess(options)).error_code, "interactive_verification_cancelled");
  assert.equal((await waitForManualAccess({ ...options, shouldStop: async () => true })).error_code, "collection_stopped");
});

test("network rejection during a manual window does not wait for a slider", async () => {
  const page = manualPage();
  page.url = () => "https://www.xiaohongshu.com/website-login/error?error_code=300012";
  const result = await waitForManualAccess({ page, context: {}, platform: "xiaohongshu",
    errorCode: "challenge_required", timeoutMs: 1000 });
  assert.equal(result.error_code, "network_access_restricted");
});

test("closing or timing out verification pauses its platform without repeated popups", async t => {
  for (const outcome of ["interactive_verification_cancelled", "interactive_verification_timeout"]) {
    const root = await fs.mkdtemp(path.join(os.tmpdir(), "media-discovery-manual-"));
    t.after(() => fs.rm(root, { recursive: true, force: true }));
    const context = { skill_storage: { storage_kind: "directory", directory_path: root } };
    await handleRequest({ args: { action: "enable", platform: "douyin", confirm: true }, context });
    let popups = 0;
    const result = await handleRequest({ args: { action: "run_once", platform: "douyin" }, context }, {
      collectPlatform: async () => { throw new Error("login_required"); },
      waitForInteractiveLogin: async ({ errorCode, shouldStop }) => {
        popups += 1;
        assert.equal(errorCode, "login_required");
        assert.equal(await shouldStop(), false);
        return { ready: false, error_code: outcome };
      },
    });
    assert.equal(popups, 1);
    assert.equal(result.extra.state, "waiting_for_manual_verification");
    const status = await handleRequest({ args: { action: "status" }, context });
    assert.equal(status.extra.platforms.douyin.paused, true);
  }
});
