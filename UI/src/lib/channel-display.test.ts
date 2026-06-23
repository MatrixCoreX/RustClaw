import test from "node:test";
import assert from "node:assert/strict";

import { boundChannelsLabel, channelLabel } from "./channel-display.ts";

test("formats known channel labels", () => {
  assert.equal(channelLabel("telegram", "en"), "Telegram");
  assert.equal(channelLabel("wechat", "zh"), "微信");
  assert.equal(channelLabel("wechat", "en"), "WeChat");
});

test("formats bound channel lists while preserving unknown channel tokens", () => {
  assert.equal(boundChannelsLabel(["telegram", "wechat", "custom"], "en"), "Telegram / WeChat / custom");
  assert.equal(boundChannelsLabel([], "zh"), "");
});
