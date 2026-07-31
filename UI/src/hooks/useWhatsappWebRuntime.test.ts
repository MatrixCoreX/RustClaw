import assert from "node:assert/strict";
import test from "node:test";

import { whatsappWebRequestError } from "./useWhatsappWebRuntime";

const translate = (zh: string, _en: string) => zh;

test("WhatsApp Web request errors map machine codes without exposing bridge prose", () => {
  assert.equal(
    whatsappWebRequestError(translate, "whatsapp_web.login_status_invalid"),
    "连接状态返回异常，请重启服务。",
  );
  assert.equal(
    whatsappWebRequestError(translate, "whatsapp_web.logout_failed"),
    "退出登录失败，请重启服务后重试。",
  );
  assert.equal(
    whatsappWebRequestError(translate, "private bridge response body"),
    "暂时无法读取 WhatsApp Web 连接状态。",
  );
});
