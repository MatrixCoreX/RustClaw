import fs from "node:fs/promises";
import path from "node:path";
import { randomUUID } from "node:crypto";
import { SUPPORTED_PLATFORMS } from "./platforms.mjs";
import { readStatusState, withLock } from "./storage.mjs";
import { activeRuns } from "./run_leases.mjs";
import { cookieState } from "./account_cookie_state.mjs";

export function publicAccount(value, platform) {
  const status = ["verified", "saved_session", "expired", "signed_out", "unknown"].includes(value?.status) ? value.status : "unknown";
  const name = typeof value?.display_name === "string" && value.display_name.length <= 256
    && !/[\u0000-\u001f\u007f]/u.test(value.display_name) ? value.display_name : null;
  return { schema_version: 1, platform, status, display_name: status === "verified" ? name : null,
    checked_at: Number.isFinite(value?.checked_at) && value.checked_at >= 0 && value.checked_at < 1e12 ? value.checked_at : null,
    session_present: typeof value?.session_present === "boolean" ? value.session_present : null };
}

async function profileDirectories(root, platform) {
  const base = path.join(root, "browser-profile");
  if (!await absentOrDirectory(base)) return [];
  const parents = [base];
  for (const item of await fs.readdir(base, { withFileTypes: true })) {
    if (!item.name.startsWith("macos-")) continue;
    const parent = path.join(base, item.name);
    if (await absentOrDirectory(parent)) parents.push(parent);
  }
  const profiles = [];
  for (const parent of parents) {
    const profile = path.join(parent, platform);
    if (await absentOrDirectory(profile)) profiles.push(profile);
  }
  return profiles;
}

async function absentOrDirectory(file) {
  try {
    const stat = await fs.lstat(file);
    if (stat.isSymbolicLink() || !stat.isDirectory()) throw new Error("account_storage_invalid");
    return true;
  } catch (error) {
    if (error.code === "ENOENT") return false;
    throw error;
  }
}

export async function writeAccount(root, value) {
  if (!SUPPORTED_PLATFORMS.includes(value.platform)) throw new Error("platform_unsupported");
  const directory = path.join(root, "accounts");
  await absentOrDirectory(directory);
  await fs.mkdir(directory, { recursive: true, mode: 0o700 });
  const temporary = path.join(directory, `.account-${randomUUID()}`);
  try {
    await fs.writeFile(temporary, JSON.stringify(publicAccount(value, value.platform)), { flag: "wx", mode: 0o600 });
    await fs.rename(temporary, path.join(directory, `${value.platform}.json`));
  } finally {
    await fs.unlink(temporary).catch(error => { if (error.code !== "ENOENT") throw error; });
  }
}

export async function accountStatus(root, platforms = SUPPORTED_PLATFORMS) {
  await absentOrDirectory(path.join(root, "accounts"));
  return Promise.all(platforms.map(async platform => {
    if (!SUPPORTED_PLATFORMS.includes(platform)) throw new Error("platform_unsupported");
    const file = path.join(root, "accounts", `${platform}.json`);
    let value = {};
    try {
      const stat = await fs.lstat(file);
      if (!stat.isFile() || stat.isSymbolicLink() || stat.size > 16384) throw new Error("account_storage_invalid");
      value = JSON.parse(await fs.readFile(file, "utf8"));
    } catch (error) {
      if (error.code !== "ENOENT" && !(error instanceof SyntaxError)) throw error;
    }
    const snapshot = publicAccount(value, platform);
    const states = await Promise.all((await profileDirectories(root, platform)).map(profile => cookieState(profile, platform)));
    if (states.includes("saved_session")) {
      snapshot.status = snapshot.status === "verified" ? "verified" : "saved_session";
      snapshot.session_present = true;
    } else if (states.includes("expired")) {
      snapshot.status = "expired";
      snapshot.session_present = false;
    } else if (["verified", "saved_session"].includes(snapshot.status)) {
      snapshot.status = "unknown";
      snapshot.session_present = null;
    }
    return publicAccount(snapshot, platform);
  }));
}

export async function logoutAccount(root, platform) {
  if (!SUPPORTED_PLATFORMS.includes(platform)) throw new Error("platform_unsupported");
  return withLock(root, async () => {
    const state = await readStatusState(root);
    if (activeRuns(state).some(run => run.platforms.includes(platform))) throw new Error("account_session_busy");
    if (state.background_worker && state.platforms[platform]?.enabled && !state.platforms[platform]?.paused) throw new Error("account_session_busy");
    const profiles = await profileDirectories(root, platform);
    for (const profile of profiles) {
      try {
        await fs.lstat(path.join(profile, "SingletonLock"));
        throw new Error("account_session_busy");
      } catch (error) { if (error.code !== "ENOENT") throw error; }
    }
    let changed = false;
    try {
      // Invalidate the old identity before clearing credentials. If cleanup
      // later fails, Refresh must not resurrect the previous verified state.
      await writeAccount(root, { platform, status: "unknown", checked_at: Math.floor(Date.now() / 1000) });
      changed = true;
      for (const profile of profiles) {
        for (const item of await fs.readdir(profile)) {
          if (item === "browser-environment.json") continue;
          // Recursive removal itself may be partially applied before throwing.
          changed = true;
          await fs.rm(path.join(profile, item), { recursive: true, force: false });
        }
      }
      const snapshot = { schema_version: 1, platform, status: "signed_out", display_name: null,
        checked_at: Math.floor(Date.now() / 1000), session_present: false };
      await writeAccount(root, snapshot);
      return snapshot;
    } catch (cause) {
      const error = new Error("account_operation_failed", { cause });
      error.extra = { side_effect_applied: changed, failure_phase: changed ? "post_dispatch" : "pre_dispatch" };
      throw error;
    }
  });
}
