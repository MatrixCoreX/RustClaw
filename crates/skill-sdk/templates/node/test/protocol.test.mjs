import test from "node:test";
import assert from "node:assert/strict";
import { respond } from "../src/main.mjs";

test("response echoes request id", () => {
  assert.equal(respond({ request_id: "test-1" }).request_id, "test-1");
});
