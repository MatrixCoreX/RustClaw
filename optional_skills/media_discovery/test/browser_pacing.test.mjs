import assert from "node:assert/strict";
import test from "node:test";

import { pacingDelayMs } from "../src/browser.mjs";

test("browser pacing stays inside configured bounds and can be tested deterministically", () => {
  const config = { pacing_min_delay_ms: 700, pacing_max_delay_ms: 1800 };
  assert.equal(pacingDelayMs(config, () => 0), 700);
  assert.equal(pacingDelayMs(config, () => 0.5), 1250);
  assert.equal(pacingDelayMs(config, () => 1), 1800);
  assert.equal(pacingDelayMs(config, () => 0.5, 0.5), 700);
  assert.equal(pacingDelayMs(config, () => 1, 1.25), 1800);
});

test("browser pacing shares production defaults and keeps every interaction within user bounds", () => {
  assert.equal(pacingDelayMs({}, () => 0), 1000);
  assert.equal(pacingDelayMs({}, () => 1), 2800);
  for (const multiplier of [0.5, 0.75, 1, 1.25]) {
    for (const sample of [0, 0.25, 0.5, 0.75, 1]) {
      const delay = pacingDelayMs({ pacing_min_delay_ms: 1000, pacing_max_delay_ms: 2800 }, () => sample, multiplier);
      assert.ok(delay >= 1000 && delay <= 2800);
    }
  }
  assert.equal(pacingDelayMs({ pacing_min_delay_ms: 600, pacing_max_delay_ms: 600 }, () => 1, 0.5), 600);
});
