import assert from "node:assert/strict";
import test from "node:test";
import { bindSession, channelBindHarness } from "../../test/helpers/channel-bind-harness";

const invalidIntervals: [string, unknown][] = [
  ["signed timer overflow", 2_147_483.648],
  ["int64-sized interval", 9_223_372_036_854_775_807],
  ["extreme finite interval", 1e300],
  ["text instead of number", "9"],
  ["object instead of number", {}],
  ["array instead of number", []],
];

for (const platform of ["feishu", "lark"] as const) {
  for (const [label, interval] of invalidIntervals) {
    test(`${platform} ${label} cannot accelerate binding polling`, async (context) => {
      const h = await channelBindHarness(context, platform);
      context.after(() => h.dispose());
      const delays: (number | undefined)[] = [];
      const setTimer = window.setTimeout.bind(window);
      context.mock.method(window, "setTimeout", (handler: TimerHandler, delay?: number, ...args: unknown[]) => {
        delays.push(delay);
        // Model browser signed-32-bit conversion before driving mock timers.
        return setTimer(handler, Math.max(0, Number(delay) | 0), ...args);
      });
      const request = await h.begin();
      await h.reply(request.index, { ...bindSession(platform), poll_interval_seconds: interval });
      await request.completion;
      assert.deepEqual(delays, [5000]);
      assert.equal(h.current.bindSession?.status, "pending");
      await h.advance(4999);
      assert.equal(h.pending.length, 1);
      await h.advance(1);
      assert.equal(h.pending.length, 2);
      assert.deepEqual(h.reads, []);
    });
  }

  for (const [interval, expected] of [[1, 1000], [0, 1000], [-4, 1000], [2.5, 2500], [null, 5000], [undefined, 5000], [2_147_483, 2_147_483_000]] as const) {
    test(`${platform} preserves representable interval ${interval}`, async (context) => {
      const h = await channelBindHarness(context, platform);
      context.after(() => h.dispose());
      const delays: (number | undefined)[] = [];
      const setTimer = window.setTimeout.bind(window);
      context.mock.method(window, "setTimeout", (handler: TimerHandler, delay?: number, ...args: unknown[]) => {
        delays.push(delay);
        return setTimer(handler, delay, ...args);
      });
      const request = await h.begin();
      await h.reply(request.index, { ...bindSession(platform), poll_interval_seconds: interval });
      await request.completion;
      assert.deepEqual(delays, [expected]);
      if (expected < 10_000) {
        await h.advance(expected - 1);
        assert.equal(h.pending.length, 1);
        await h.advance(1);
        assert.equal(h.pending.length, 2);
      }
    });
  }

  test(`${platform} guarded polling is cancelled when ownership changes`, async (context) => {
    const h = await channelBindHarness(context, platform);
    context.after(() => h.dispose());
    const request = await h.begin();
    await h.reply(request.index, { ...bindSession(platform), poll_interval_seconds: 1e300 });
    await request.completion;
    await h.update({ requestScope: "device-b:admin:epoch-1" });
    await h.advance(10_000);
    assert.equal(h.pending.length, 1);
    assert.equal(h.current.bindSession, null);
  });

  test(`${platform} refreshed session interval is validated without restarting the binding`, async (context) => {
    const h = await channelBindHarness(context, platform);
    context.after(() => h.dispose());
    await h.load();
    await h.advance(1000);
    assert.equal(h.pending.length, 2);
    await h.reply(1, { ...bindSession(platform), poll_interval_seconds: 2_147_483.648 });
    await h.advance(4999);
    assert.equal(h.pending.length, 2);
    await h.advance(1);
    assert.equal(h.pending.length, 3);
    assert.equal(h.pending.filter(row => row.init?.method === "POST").length, 1);
    assert.deepEqual(h.reads, []);
  });

  test(`${platform} failed guarded poll still stops instead of silently retrying`, async (context) => {
    const h = await channelBindHarness(context, platform);
    context.after(() => h.dispose());
    const request = await h.begin();
    await h.reply(request.index, { ...bindSession(platform), poll_interval_seconds: 1e300 });
    await request.completion;
    await h.advance(5000);
    assert.equal(h.pending.length, 2);
    await h.reject(1);
    await h.advance(60_000);
    assert.equal(h.pending.length, 2);
    assert.equal(h.current.bindSession?.status, "pending");
    assert.deepEqual(h.reads, []);
  });

  test(`${platform} terminal binding does not schedule a timer from irrelevant metadata`, async (context) => {
    const h = await channelBindHarness(context, platform);
    context.after(() => h.dispose());
    const request = await h.begin();
    await h.reply(request.index, { ...bindSession(platform, 1, "bound"), poll_interval_seconds: 1e300 });
    await request.completion;
    await h.advance(10_000);
    assert.equal(h.pending.length, 1);
    assert.equal(h.current.bindSession?.status, "bound");
  });
}
