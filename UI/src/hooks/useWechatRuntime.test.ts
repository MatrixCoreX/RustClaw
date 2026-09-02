import assert from "node:assert/strict";
import test from "node:test";

import {
  wechatLoginRequestError,
  wechatLoginSessionRequiresRestart,
} from "./useWechatRuntime";

const translate = (zh: string, _en: string) => zh;

test("WeChat login maps session ownership errors to actionable UI text", () => {
  assert.equal(
    wechatLoginRequestError(translate, "wechat.login_session_in_use", "fallback", "fallback"),
    "另一个用户正在完成微信扫码，请稍后再试。",
  );
  assert.equal(
    wechatLoginRequestError(translate, "wechat.login_session_owner_mismatch", "fallback", "fallback"),
    "本次二维码不属于当前登录用户，请重新生成。",
  );
  assert.equal(
    wechatLoginRequestError(translate, "unrecognized", "默认提示", "Fallback"),
    "默认提示",
  );
});

test("WeChat login stops polling only for terminal client-session errors", () => {
  assert.equal(wechatLoginSessionRequiresRestart("wechat.login_session_expired"), true);
  assert.equal(wechatLoginSessionRequiresRestart("wechat.login_session_owner_mismatch"), true);
  assert.equal(wechatLoginSessionRequiresRestart("wechat.qr_wait_unavailable"), false);
});
