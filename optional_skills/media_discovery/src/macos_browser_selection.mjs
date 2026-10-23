import fs from "node:fs/promises";
import { constants } from "node:fs";
import os from "node:os";
import path from "node:path";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { createHash } from "node:crypto";

const run = promisify(execFile);
const BROWSERS = [
  ["Google Chrome.app", "Google Chrome", "com.google.Chrome", "EQHXZ8M8AV"],
  ["Microsoft Edge.app", "Microsoft Edge", "com.microsoft.edgemac", "UBF8T346G9"],
];
export const MINIMUM_CHROMIUM_MAJOR = 120;

export async function selectMacOSBrowser({
  hostPlatform = process.platform,
  applications = [path.join(os.homedir(), "Applications"), "/Applications"],
  execute = run,
  access = file => fs.access(file, constants.X_OK),
} = {}) {
  if (hostPlatform !== "darwin") return null;
  for (const [appName, executableName, bundleId, team] of BROWSERS) {
    for (const root of applications) {
      const app = path.join(root, appName);
      const executablePath = path.join(app, "Contents", "MacOS", executableName);
      try {
        await access(executablePath);
        const info = {};
        // Project only scalar fields; unrelated plist types need not support JSON.
        for (const key of ["CFBundleIdentifier", "CFBundleExecutable", "CFBundleShortVersionString"]) {
          const { stdout } = await execute("/usr/bin/plutil", [
            "-extract", key, "raw", "-o", "-", path.join(app, "Contents", "Info.plist"),
          ], { timeout: 5000, maxBuffer: 1024 * 1024 });
          info[key] = stdout.trim();
        }
        const major = String(info.CFBundleShortVersionString || "").split(".")[0];
        if (info.CFBundleIdentifier !== bundleId || info.CFBundleExecutable !== executableName
          || !/^[0-9]+$/u.test(major) || Number(major) < MINIMUM_CHROMIUM_MAJOR) continue;
        const requirement = `=anchor apple generic and identifier "${bundleId}" and certificate leaf[subject.OU] = "${team}"`;
        await execute("/usr/bin/codesign", ["--verify", "--strict", "-R", requirement, app],
          { timeout: 5000 });
        return { executablePath, bundleId };
      } catch {
        // Missing, obsolete, invalid or timed-out candidates never become a browser launch.
      }
    }
  }
  return null;
}

export function browserProfileDirectory(root, platform, executablePath, hostPlatform = process.platform) {
  const base = path.join(root, "browser-profile");
  if (hostPlatform !== "darwin") return path.join(base, platform);
  for (const [appName, executableName, bundleId] of BROWSERS) {
    if (executablePath?.endsWith(path.join(appName, "Contents", "MacOS", executableName))) {
      return path.join(base, `macos-${bundleId}`, platform);
    }
  }
  const identity = createHash("sha256").update(String(executablePath || "")).digest("hex").slice(0, 16);
  return path.join(base, `macos-browser-${identity}`, platform);
}
