import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { acquireStateLock } from "../src/storage_lock.mjs";

async function fixture(t) {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "discovery-lock-"));
  t.after(() => fs.rm(root, { recursive: true, force: true }));
  const file = path.join(root, ".state.lock");
  await fs.writeFile(file, "owner");
  return { root, file };
}

test("healthy lock turnover can queue beyond ten seconds without losing a commit", async t => {
  const { root, file } = await fixture(t);
  const start = Date.now();
  let clock = start, waits = 0;
  const lock = await acquireStateLock(root, { now: () => clock, sleep: async () => {
    clock += 6000;
    if (++waits === 3) await fs.unlink(file);
    else await fs.writeFile(file, "owner".repeat(waits + 1));
  } });
  assert.equal(clock - start, 18000);
  await lock.handle.close();
  await fs.unlink(lock.lockPath);
});

test("an unchanged lock owner still fails with a structured stall error", async t => {
  const { root } = await fixture(t);
  let clock = Date.now();
  await assert.rejects(acquireStateLock(root, { now: () => clock, sleep: async () => { clock += 6000; } }),
    { message: "storage_lock_timeout" });
});
