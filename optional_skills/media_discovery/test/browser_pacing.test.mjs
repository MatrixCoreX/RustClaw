import assert from "node:assert/strict";
import test from "node:test";

import { pacingDelayMs } from "../src/browser.mjs";

test("browser pacing stays inside configured bounds and can be tested deterministically", () => {
  const config = { pacing_min_delay_ms: 700, pacing_max_delay_ms: 1800 };
  assert.equal(pacingDelayMs(config, () => 0), 700);
  assert.equal(pacingDelayMs(config, () => 0.5), 1250);
  assert.equal(pacingDelayMs(config, () => 1), 1800);
  assert.equal(pacingDelayMs(config, () => 0.5, 0.5), 625);
});
