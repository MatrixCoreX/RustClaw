import assert from "node:assert/strict";
import test from "node:test";

import {
  backgroundProgressFrame,
  backgroundReportIntervalMs,
  createBackgroundProgressReporter,
} from "../src/progress.mjs";

test("background collection emits one structured report per fifteen-minute boundary", (t) => {
  let now = 1_000;
  const frames = [];
  const counts = { items: 2, videos: 1, images: 3, duplicates: 4, failures: 0 };
  const reporter = createBackgroundProgressReporter({
    requestId: "task-progress",
    run: { run_id: "run-progress", platforms: ["douyin", "xiaohongshu"] },
    counts,
    writeFrame: (frame) => frames.push(frame),
    now: () => now,
  });
  t.after(() => reporter.stop());

  now += backgroundReportIntervalMs - 1;
  assert.equal(reporter.emitIfDue(), false);
  now += 1;
  assert.equal(reporter.emitIfDue(), true);
  assert.equal(reporter.emitIfDue(), false);

  counts.items = 5;
  counts.videos = 2;
  now += backgroundReportIntervalMs;
  assert.equal(reporter.emitIfDue(), true);

  assert.deepEqual(frames.map((frame) => frame.sequence), [1, 2]);
  assert.equal(frames[0].record_type, "skill_progress");
  assert.equal(frames[0].kind, "heartbeat");
  assert.equal(frames[0].detail_key, "media_discovery.background.status");
  assert.equal(frames[0].params.notification_delivery, "runtime");
  assert.equal(frames[0].params.notification_interval_seconds, 900);
  assert.equal(frames[0].params.elapsed_minutes, 15);
  assert.equal(frames[1].params.elapsed_minutes, 30);
  assert.equal(frames[1].params.items, 5);
  assert.deepEqual(frames[1].params.platforms, ["douyin", "xiaohongshu"]);
});

test("background report payload contains only bounded machine fields", () => {
  const frame = backgroundProgressFrame({
    requestId: "task-1",
    sequence: 1,
    run: { run_id: "run-1", platforms: ["douyin"] },
    counts: {},
    elapsedMs: backgroundReportIntervalMs,
  });

  assert.equal(Object.keys(frame.params).length <= 16, true);
  assert.equal(frame.params.message_key, "channel.notice.media_discovery_background_progress");
  assert.equal("text" in frame, false);
  assert.equal("error_text" in frame, false);
});
