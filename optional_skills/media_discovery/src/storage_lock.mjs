import fs from "node:fs/promises";
import path from "node:path";

// Detect a stalled owner, not total time queued behind healthy commits.
export async function acquireStateLock(root, {
  now = Date.now, sleep = ms => new Promise(resolve => setTimeout(resolve, ms)),
  stallMs = 10_000,
} = {}) {
  const lockPath = path.join(root, ".state.lock");
  let deadline = now() + stallMs;
  let observedOwner;
  for (;;) {
    try {
      const handle = await fs.open(lockPath, "wx", 0o600);
      try {
        await handle.writeFile(JSON.stringify({ pid: process.pid, created_at: now() }));
      } catch (error) {
        await handle.close().catch(() => {});
        await fs.unlink(lockPath).catch(() => {});
        throw error;
      }
      return { handle, lockPath };
    } catch (error) {
      if (error?.code !== "EEXIST") throw error;
      try {
        const stat = await fs.stat(lockPath);
        if (now() - stat.mtimeMs > 30 * 60 * 1000) {
          await fs.unlink(lockPath);
          continue;
        }
        const owner = `${stat.dev}:${stat.ino}:${stat.mtimeMs}:${stat.size}`;
        if (owner !== observedOwner) {
          observedOwner = owner;
          deadline = now() + stallMs;
        } else if (now() >= deadline) throw new Error("storage_lock_timeout");
      } catch (statError) {
        if (statError?.code !== "ENOENT") throw statError;
      }
      await sleep(40);
    }
  }
}
