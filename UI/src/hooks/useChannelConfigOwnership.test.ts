import assert from "node:assert/strict";
import test from "node:test";
import { act } from "react-test-renderer";
import { channelConfig, channelConfigHarness, channels, configKeys } from "../../test/helpers/channel-config-harness";

for (const channel of channels) {
  const keys = configKeys[channel];
  test(`${channel} configuration clears the previous scope snapshot`, async (t) => {
    const h = await channelConfigHarness(); t.after(() => h.dispose());
    await h.load(channel);
    await h.update({ requestScope: "device-b:admin:epoch-1" });
    assert.equal(h.current[keys.data], null);
    assert.equal(h.current[keys.loading], false);
    assert.equal(h.current[keys.error], null);
  });
  test(`${channel} ignores a late old-scope read and finally`, async (t) => {
    const h = await channelConfigHarness(); t.after(() => h.dispose());
    const old = await h.read(channel);
    await h.update({ requestScope: "device-b:admin:epoch-1" });
    const fresh = await h.read(channel);
    await h.reply(old.index, channelConfig(channel));
    assert.equal(h.current[keys.data], null);
    assert.equal(h.current[keys.loading], true);
    await h.reply(fresh.index, channelConfig(channel, "device-b"));
    assert.equal(h.current[keys.data]?.config_path, "device-b");
  });
  test(`${channel} latest same-scope read owns data and loading`, async (t) => {
    const h = await channelConfigHarness(); t.after(() => h.dispose());
    const old = await h.read(channel), fresh = await h.read(channel);
    await h.reply(old.index, channelConfig(channel, "older"));
    assert.equal(h.current[keys.data], null);
    assert.equal(h.current[keys.loading], true);
    await h.reply(fresh.index, channelConfig(channel, "newer"));
    assert.equal(h.current[keys.data]?.config_path, "newer");
    assert.equal(h.current[keys.loading], false);
  });
  test(`${channel} rejects stale errors after current success`, async (t) => {
    const h = await channelConfigHarness(); t.after(() => h.dispose());
    const old = await h.read(channel), fresh = await h.read(channel);
    await h.reply(fresh.index, channelConfig(channel, "newer"));
    await h.reject(old.index);
    assert.equal(h.current[keys.error], null);
    assert.equal(h.current[keys.data]?.config_path, "newer");
  });
  test(`${channel} rechecks ownership after the response body arrives`, async (t) => {
    const h = await channelConfigHarness(); t.after(() => h.dispose());
    const old = await h.read(channel), finishBody = await h.heldBody(old.index);
    await h.update({ requestScope: "device-b:admin:epoch-1" });
    await h.load(channel, "device-b");
    await finishBody(channelConfig(channel));
    assert.equal(h.current[keys.data]?.config_path, "device-b");
  });
  test(`${channel} captured callbacks cannot dispatch after A/B/A or unmount`, async (t) => {
    const h = await channelConfigHarness(); t.after(() => h.dispose());
    const old = h.current;
    await h.update({ requestScope: "device-b" });
    await h.update({ requestScope: "device-a:admin:epoch-0" });
    const call = await h.read(channel, old);
    assert.equal(h.pending.length, 0);
    await call.completion;
    const current = h.current;
    await h.dispose();
    const disposed = await h.read(channel, current);
    assert.equal(h.pending.length, 0);
    await disposed.completion;
  });
  test(`${channel} unauthenticated callbacks do not dispatch; ordinary reads still work`, async (t) => {
    const h = await channelConfigHarness(); t.after(() => h.dispose());
    await h.update({ requestScope: "" });
    const absent = await h.read(channel);
    assert.equal(h.pending.length, 0);
    await absent.completion;
    await h.update({ requestScope: "device-a:user:epoch-2", isAdminIdentity: false });
    await h.load(channel, "ordinary-user");
    assert.equal(h.current[keys.data]?.config_path, "ordinary-user");
  });
  test(`${channel} current failures remain visible and retryable`, async (t) => {
    const h = await channelConfigHarness(); t.after(() => h.dispose());
    const failed = await h.read(channel);
    await h.reject(failed.index);
    assert.ok(h.current[keys.error]);
    assert.equal(h.current[keys.loading], false);
    await h.load(channel);
    assert.equal(h.current[keys.error], null);
    assert.equal(h.current[keys.data]?.config_path, "device-a");
  });
  for (const change of [{ isAdminIdentity: false }, { requestScope: "device-a:admin:epoch-2" }]) {
    test(`${channel} pending reads belong to the original role/login epoch ${JSON.stringify(change)}`, async (t) => {
      const h = await channelConfigHarness(); t.after(() => h.dispose());
      const old = await h.read(channel);
      await h.update(change);
      await h.load(channel, "current");
      await h.reply(old.index, channelConfig(channel, "obsolete"));
      assert.equal(h.current[keys.data]?.config_path, "current");
    });
  }
  test(`${channel} A/B/A never reactivates an old pending read`, async (t) => {
    const h = await channelConfigHarness(); t.after(() => h.dispose());
    const old = await h.read(channel);
    await h.update({ requestScope: "device-b" });
    await h.update({ requestScope: "device-a:admin:epoch-0" });
    await h.load(channel, "current-a");
    await h.reply(old.index, channelConfig(channel, "obsolete-a"));
    assert.equal(h.current[keys.data]?.config_path, "current-a");
  });
  test(`${channel} current HTTP rejection does not erase an existing snapshot`, async (t) => {
    const h = await channelConfigHarness(); t.after(() => h.dispose());
    await h.load(channel);
    const failed = await h.read(channel);
    await h.reply(failed.index, null, 403);
    assert.ok(h.current[keys.error]);
    assert.equal(h.current[keys.data]?.config_path, "device-a");
    assert.equal(h.current[keys.loading], false);
  });
}

for (const change of [{ requestScope: "device-b:admin:epoch-1" }, { isAdminIdentity: false }, { requestScope: "device-a:admin:epoch-2" }]) {
  test(`WeChat enable acknowledgment is scoped to its caller ${JSON.stringify(change)}`, async (t) => {
    const h = await channelConfigHarness(); t.after(() => h.dispose());
    await h.load("wechat");
    const old = await h.enable();
    await h.update(change);
    await h.load("wechat", "current");
    await h.reply(old.index, { ...channelConfig("wechat"), enabled: true });
    assert.equal(await old.completion, false);
    assert.equal(h.current.wechatConfigData?.config_path, "current");
    assert.equal(h.current.wechatConfigSaving, false);
  });
}

test("WeChat saves are single-flight and fence reads started before or during the save", async (t) => {
  const h = await channelConfigHarness(); t.after(() => h.dispose());
  await h.load("wechat");
  const read = await h.read("wechat"), save = await h.enable();
  const duplicate = await h.enable(false), during = await h.read("wechat");
  assert.equal(h.pending.length, 3);
  assert.equal(await duplicate.completion, false);
  await during.completion;
  await h.reply(read.index, channelConfig("wechat", "obsolete"));
  assert.equal(h.current.wechatConfigData?.config_path, "device-a");
  assert.equal(h.current.wechatConfigSaving, true);
  await h.reply(save.index, { ...channelConfig("wechat", "accepted"), enabled: true });
  assert.equal(await save.completion, true);
  assert.equal(h.current.wechatConfigData?.config_path, "accepted");
  assert.equal(h.current.wechatConfigLoading, false);
});

test("ordinary users and old callbacks cannot write channel configuration", async (t) => {
  const h = await channelConfigHarness(); t.after(() => h.dispose());
  await h.load("wechat"); await h.load("telegram");
  const old = h.current;
  await h.update({ isAdminIdentity: false });
  await h.load("wechat"); await h.load("telegram");
  const count = h.pending.length;
  const enable = await h.enable(), save = await h.save();
  await h.edit("forbidden");
  const oldEnable = await h.enable(true, old), oldSave = await h.save(old);
  await h.edit("also-forbidden", old);
  assert.equal(h.pending.length, count);
  assert.equal(await enable.completion, false);
  assert.equal(await oldEnable.completion, false);
  await save.completion; await oldSave.completion;
  assert.equal(h.current.primaryTelegramBot.name, "primary");
});

test("Telegram refresh updates the server snapshot without destroying a dirty draft", async (t) => {
  const h = await channelConfigHarness(); t.after(() => h.dispose());
  await h.load("telegram"); await h.edit("my-draft");
  await h.load("telegram", "refreshed");
  assert.equal(h.current.telegramConfigData?.config_path, "refreshed");
  assert.equal(h.current.primaryTelegramBot.name, "my-draft");
  assert.equal(h.current.hasUnsavedTelegramConfigChanges, true);
});

test("Telegram edits made while a refresh is pending survive its response", async (t) => {
  const h = await channelConfigHarness(); t.after(() => h.dispose());
  await h.load("telegram");
  const refresh = await h.read("telegram");
  await h.edit("newer-edit");
  await h.reply(refresh.index, channelConfig("telegram", "refreshed"));
  assert.equal(h.current.primaryTelegramBot.name, "newer-edit");
});

test("Telegram save acknowledgments preserve newer edits and do not mark them saved", async (t) => {
  const h = await channelConfigHarness(); t.after(() => h.dispose());
  await h.load("telegram"); await h.edit("submitted");
  const save = await h.save();
  assert.equal(JSON.parse(String(h.pending[save.index].init?.body)).bots[0].name, "submitted");
  await h.edit("newer-edit");
  const accepted = channelConfig("telegram");
  assert.ok("bots" in accepted); accepted.bots[0].name = "submitted";
  await h.reply(save.index, accepted);
  assert.equal(h.current.telegramConfigData?.bots[0].name, "submitted");
  assert.equal(h.current.primaryTelegramBot.name, "newer-edit");
  assert.equal(h.current.hasUnsavedTelegramConfigChanges, true);
  assert.equal(h.current.telegramConfigSaveMessage, null);
});

test("Telegram same-event edit and save use the latest draft without awaiting a rerender", async (t) => {
  const h = await channelConfigHarness(); t.after(() => h.dispose());
  await h.load("telegram");
  const captured = h.current;
  await act(async () => {
    captured.setTelegramPrimaryBotDraftField("name", "same-event");
    void captured.saveTelegramConfig();
  });
  assert.equal(JSON.parse(String(h.pending[1].init?.body)).bots[0].name, "same-event");
  await h.reply(1, channelConfig("telegram"));
});

test("Telegram save fences older reads and allows only one save without losing the draft", async (t) => {
  const h = await channelConfigHarness(); t.after(() => h.dispose());
  await h.load("telegram"); await h.edit("submitted");
  const read = await h.read("telegram"), save = await h.save(), duplicate = await h.save();
  const during = await h.read("telegram");
  assert.equal(h.pending.length, 3);
  await duplicate.completion; await during.completion;
  await h.reply(read.index, channelConfig("telegram", "obsolete"));
  assert.equal(h.current.primaryTelegramBot.name, "submitted");
  await h.reject(save.index);
  assert.equal(h.current.telegramConfigSaving, false);
  assert.equal(h.current.primaryTelegramBot.name, "submitted");
  assert.ok(h.current.telegramConfigError);
});

test("Telegram old save cannot update a new scope or release a newer save", async (t) => {
  const h = await channelConfigHarness(); t.after(() => h.dispose());
  await h.load("telegram"); await h.edit("old");
  const old = await h.save();
  await h.update({ requestScope: "device-b:admin:epoch-1" });
  await h.load("telegram", "device-b"); await h.edit("current");
  const fresh = await h.save();
  await h.reply(old.index, channelConfig("telegram"));
  assert.equal(h.current.telegramConfigData?.config_path, "device-b");
  assert.equal(h.current.primaryTelegramBot.name, "current");
  assert.equal(h.current.telegramConfigSaving, true);
  assert.equal(h.current.telegramConfigSaveMessage, null);
  await h.reply(fresh.index, channelConfig("telegram", "accepted"));
});

test("all four configuration reads remain independent", async (t) => {
  const h = await channelConfigHarness(); t.after(() => h.dispose());
  const reads = [];
  for (const channel of channels) reads.push([channel, await h.read(channel)] as const);
  for (const [channel, read] of reads) {
    await h.reply(read.index, channelConfig(channel));
    assert.equal(h.current[configKeys[channel].data]?.config_path, "device-a");
  }
});

test("a failed WeChat save retains the loaded config and permits an explicit retry", async (t) => {
  const h = await channelConfigHarness(); t.after(() => h.dispose());
  await h.load("wechat");
  const failed = await h.enable();
  await h.reject(failed.index);
  assert.equal(await failed.completion, false);
  assert.equal(h.current.wechatConfigData?.config_path, "device-a");
  assert.equal(h.current.wechatConfigSaving, false);
  assert.ok(h.current.wechatConfigError);
  const retry = await h.enable();
  await h.reply(retry.index, { ...channelConfig("wechat"), enabled: true });
  assert.equal(await retry.completion, true);
  assert.equal(h.current.wechatConfigError, null);
  assert.equal(h.pending.filter(row => row.init?.method === "POST").length, 2);
});

test("unloaded WeChat configuration never issues a write", async (t) => {
  const h = await channelConfigHarness(); t.after(() => h.dispose());
  const save = await h.enable();
  assert.equal(await save.completion, false);
  assert.equal(h.pending.length, 0);
  assert.ok(h.current.wechatConfigError);
});

test("Telegram successful save normalizes payload, clears the submitted secret draft and stays clean on refresh", async (t) => {
  const h = await channelConfigHarness(); t.after(() => h.dispose());
  await h.load("telegram");
  await act(async () => {
    h.current.setTelegramPrimaryBotDraftField("name", "  edited  ");
    h.current.setTelegramPrimaryBotDraftField("bot_token", " fixture-only ");
    h.current.setTelegramPrimaryBotDraftField("agent_id", "  worker  ");
  });
  const save = await h.save();
  const sent = JSON.parse(String(h.pending[save.index].init?.body));
  assert.equal(sent.bots[0].name, "edited");
  assert.equal(sent.bots[0].bot_token, "fixture-only");
  assert.equal(sent.bots[0].agent_id, "worker");
  assert.deepEqual(sent.bots[0].allowlist, []);
  const accepted = channelConfig("telegram"); assert.ok("bots" in accepted);
  accepted.bots[0].name = "edited";
  await h.reply(save.index, accepted);
  assert.equal(h.current.primaryTelegramBot.bot_token, "");
  assert.equal(h.current.hasUnsavedTelegramConfigChanges, false);
  assert.ok(h.current.telegramConfigSaveMessage);
  await h.load("telegram", "refreshed");
  assert.equal(h.current.primaryTelegramBot.bot_token_masked, "refreshed");
  assert.equal(h.current.hasUnsavedTelegramConfigChanges, false);
});

test("Telegram dirty draft, secret input and save feedback are cleared on a new owner", async (t) => {
  const h = await channelConfigHarness(); t.after(() => h.dispose());
  await h.load("telegram");
  await act(async () => { h.current.setTelegramPrimaryBotDraftField("bot_token", "fixture-only"); });
  await h.update({ requestScope: "device-b" });
  assert.equal(h.current.primaryTelegramBot.bot_token, "");
  assert.equal(h.current.telegramConfigData, null);
  assert.equal(h.current.hasUnsavedTelegramConfigChanges, false);
  assert.equal(h.current.telegramConfigSaveMessage, null);
});

for (const channel of ["wechat", "telegram"] as const) {
  test(`${channel} a read predating the save cannot overwrite its accepted acknowledgment`, async (t) => {
    const h = await channelConfigHarness(); t.after(() => h.dispose());
    await h.load(channel);
    const old = await h.read(channel);
    const save = channel === "wechat" ? await h.enable() : await h.save();
    await h.reply(save.index, channelConfig(channel, "accepted"));
    await h.reply(old.index, channelConfig(channel, "obsolete"));
    assert.equal(h.current[configKeys[channel].data]?.config_path, "accepted");
  });
  test(`${channel} write checks owner after a delayed JSON body and ignores stale errors`, async (t) => {
    const h = await channelConfigHarness(); t.after(() => h.dispose());
    await h.load(channel);
    const save = channel === "wechat" ? await h.enable() : await h.save();
    const finish = await h.heldBody(save.index);
    await h.update({ requestScope: "device-b" });
    await h.load(channel, "current");
    await finish(channelConfig(channel, "obsolete"));
    assert.equal(h.current[configKeys[channel].data]?.config_path, "current");
    assert.equal(h.current[configKeys[channel].error], null);
  });
  test(`${channel} old write error cannot replace a new-scope error or busy state`, async (t) => {
    const h = await channelConfigHarness(); t.after(() => h.dispose());
    await h.load(channel);
    const old = channel === "wechat" ? await h.enable() : await h.save();
    await h.update({ requestScope: "device-b" });
    await h.load(channel, "current");
    const fresh = channel === "wechat" ? await h.enable() : await h.save();
    await h.reject(old.index);
    assert.equal(h.current[configKeys[channel].error], null);
    assert.equal(channel === "wechat" ? h.current.wechatConfigSaving : h.current.telegramConfigSaving, true);
    await h.reply(fresh.index, channelConfig(channel, "accepted"));
  });
}
