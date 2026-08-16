import assert from "node:assert/strict";
import test from "node:test";

import { serviceControlActions } from "./communication-service-controls";

test("offers start for a stopped communication service", () => {
  assert.deepEqual(serviceControlActions(false), ["start", "reset"]);
});

test("keeps restart and stop available for a running communication service", () => {
  assert.deepEqual(serviceControlActions(true), ["restart", "stop", "reset"]);
});

test("allows cards with a dedicated reset flow to suppress the generic reset", () => {
  assert.deepEqual(serviceControlActions(true, false), ["restart", "stop"]);
});
