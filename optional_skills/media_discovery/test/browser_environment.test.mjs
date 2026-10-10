import assert from "node:assert/strict";
import fs from "node:fs/promises";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import {
  AUTOMATION_BLINK_FLAG,
  AUTOMATION_DEFAULT_ARG,
  closePlatformBrowser,
  desktopSessionEnvironment,
  launchPlatformBrowser,
  nativeBrowserEnvironment,
  persistentContextLaunchOptions,
} from "../src/browser_environment.mjs";
import { handleRequest } from "../src/main.mjs";

const host = { platform: "linux", arch: "x64", locale: "zh-CN", timeZone: "Asia/Shanghai" };
const fileFor = (root, platform = "douyin") => path.join(root, "browser-profile", platform, "browser-environment.json");

async function fixture(t) {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "discovery-browser-environment-"));
  t.after(() => fs.rm(root, { recursive: true, force: true }));
  const launches = [];
  let closed = 0;
  const chromium = { launchPersistentContext: async (profile, options) => {
    launches.push({ profile, options });
    return { close: async () => { closed += 1; } };
  } };
  const options = { chromium, root, platform: "douyin", executablePath: "/fixture/browser",
    headless: true, native: nativeBrowserEnvironment(host) };
  return { root, launches, chromium, options, closed: () => closed };
}

test("launch options drop Chromium automation switch without spoofing identity", () => {
  const environment = nativeBrowserEnvironment(host);
  const headed = persistentContextLaunchOptions({
    executablePath: "/fixture/chrome",
    headless: false,
    environment,
    hostEnv: { WAYLAND_DISPLAY: "wayland-0" },
    hostPlatform: "linux",
  });
  assert.deepEqual(headed.ignoreDefaultArgs, [AUTOMATION_DEFAULT_ARG]);
  assert.equal(headed.args.includes(AUTOMATION_BLINK_FLAG), true);
  assert.equal(headed.args.includes("--ozone-platform=wayland"), true);
  assert.equal(headed.env.WAYLAND_DISPLAY, "wayland-0");
  assert.equal(headed.userAgent, undefined);
  assert.equal(headed.proxy, undefined);
});

test("system-service execution discovers the current user's Wayland session", async t => {
  const runtime = await fs.mkdtemp(path.join(os.tmpdir(), "discovery-desktop-session-"));
  const socketPath = path.join(runtime, "wayland-7");
  const authorityPath = path.join(runtime, ".mutter-Xwaylandauth.fixture");
  const server = net.createServer();
  t.after(async () => {
    await new Promise(resolve => server.close(resolve));
    await fs.rm(runtime, { recursive: true, force: true });
  });
  await new Promise((resolve, reject) => server.listen(socketPath, resolve).once("error", reject));
  await fs.writeFile(authorityPath, "fixture");

  const environment = desktopSessionEnvironment({}, {
    hostPlatform: "linux",
    effectiveUid: null,
    runtimeDirectory: runtime,
    x11SocketDirectory: path.join(runtime, "missing-x11"),
  });
  assert.equal(environment.XDG_RUNTIME_DIR, runtime);
  assert.equal(environment.WAYLAND_DISPLAY, "wayland-7");
  assert.equal(environment.XAUTHORITY, authorityPath);
  assert.equal(environment.DISPLAY, undefined);
});

test("managed profiles repair stale crash markers and record a clean graceful close", async t => {
  const { root, options } = await fixture(t);
  const preferencesFile = path.join(root, "browser-profile", "douyin", "Default", "Preferences");
  await fs.mkdir(path.dirname(preferencesFile), { recursive: true });
  await fs.writeFile(preferencesFile, JSON.stringify({ profile: { exit_type: "Crashed" }, retained: true }));
  let closeReason = null;
  const context = await launchPlatformBrowser({ ...options, chromium: {
    launchPersistentContext: async () => ({
      close: async ({ reason }) => { closeReason = reason; },
      browser: () => ({ isConnected: () => false }),
    }),
  } });
  const repaired = JSON.parse(await fs.readFile(preferencesFile, "utf8"));
  assert.equal(repaired.profile.exit_type, "Normal");
  assert.equal(repaired.profile.exited_cleanly, true);
  assert.equal(repaired.retained, true);

  repaired.profile.exit_type = "Crashed";
  repaired.profile.exited_cleanly = false;
  await fs.writeFile(preferencesFile, JSON.stringify(repaired));
  await closePlatformBrowser({ context, root, platform: "douyin" });
  const closed = JSON.parse(await fs.readFile(preferencesFile, "utf8"));
  assert.equal(closeReason, "media_discovery_collection_complete");
  assert.equal(closed.profile.exit_type, "Normal");
  assert.equal(closed.profile.exited_cleanly, true);
  assert.equal(closed.retained, true);
});

test("a connected browser close failure is machine-visible and never marked clean", async t => {
  const { root } = await fixture(t);
  const preferencesFile = path.join(root, "browser-profile", "douyin", "Default", "Preferences");
  await fs.mkdir(path.dirname(preferencesFile), { recursive: true });
  await fs.writeFile(preferencesFile, JSON.stringify({ profile: { exit_type: "Crashed" } }));
  const browser = { isConnected: () => true, close: async () => { throw new Error("still_running"); } };
  const context = { close: async () => { throw new Error("close_failed"); }, browser: () => browser };
  await assert.rejects(closePlatformBrowser({ context, root, platform: "douyin" }),
    { message: "browser_close_failed" });
  const preferences = JSON.parse(await fs.readFile(preferencesFile, "utf8"));
  assert.equal(preferences.profile.exit_type, "Crashed");
});

test("a rejected close is not mistaken for success after an unexpected disconnect", async t => {
  const { root } = await fixture(t);
  const preferencesFile = path.join(root, "browser-profile", "douyin", "Default", "Preferences");
  await fs.mkdir(path.dirname(preferencesFile), { recursive: true });
  await fs.writeFile(preferencesFile, JSON.stringify({ profile: { exit_type: "Crashed" } }));
  const browser = { isConnected: () => false };
  const context = { close: async () => { throw new Error("unexpected_disconnect"); }, browser: () => browser };
  await assert.rejects(closePlatformBrowser({ context, root, platform: "douyin" }),
    { message: "browser_close_failed" });
  const preferences = JSON.parse(await fs.readFile(preferencesFile, "utf8"));
  assert.equal(preferences.profile.exit_type, "Crashed");
});

test("native settings cover Linux/macOS architectures without fabricating browser identity", () => {
  for (const platform of ["linux", "darwin"]) for (const arch of ["x64", "arm64"]) {
    const environment = nativeBrowserEnvironment({ ...host, platform, arch });
    assert.equal(environment.host_platform, platform);
    assert.equal(environment.host_arch, arch);
    assert.equal(environment.locale, "zh-CN");
    assert.equal(environment.timezone_id, "Asia/Shanghai");
    assert.deepEqual(environment.viewport, { width: 1280, height: 900 });
    assert.equal(environment.userAgent, undefined);
  }
  assert.throws(() => nativeBrowserEnvironment({ ...host, platform: "win32" }),
    { message: "browser_platform_unsupported" });
});

test("manual and silent launches reuse saved settings and leave login data untouched", async t => {
  const { root, launches, options } = await fixture(t);
  await (await launchPlatformBrowser(options)).close();
  const file = fileFor(root);
  const saved = await fs.readFile(file, "utf8");
  const login = path.join(path.dirname(file), "fixture-login-state");
  await fs.writeFile(login, "preserve");
  await (await launchPlatformBrowser({ ...options, headless: false,
    native: nativeBrowserEnvironment({ ...host, locale: "en-US", timeZone: "UTC" }) })).close();
  assert.equal(launches[0].profile, launches[1].profile);
  for (const { options: actual } of launches) {
    assert.equal(actual.locale, "zh-CN");
    assert.equal(actual.timezoneId, "Asia/Shanghai");
    assert.deepEqual(actual.viewport, { width: 1280, height: 900 });
    assert.equal(actual.userAgent, undefined);
    assert.deepEqual(actual.ignoreDefaultArgs, [AUTOMATION_DEFAULT_ARG]);
    assert.equal(actual.args.includes(AUTOMATION_BLINK_FLAG), true);
    assert.equal(actual.proxy, undefined);
  }
  assert.equal(launches[0].options.headless, true);
  assert.equal(launches[1].options.headless, false);
  assert.equal(await fs.readFile(file, "utf8"), saved);
  assert.equal(await fs.readFile(login, "utf8"), "preserve");
  assert.equal((await fs.stat(file)).mode & 0o777, 0o600);
});

test("platform profiles remain isolated; host migration uses the destination environment", async t => {
  const { root, launches, options } = await fixture(t);
  await (await launchPlatformBrowser(options)).close();
  const saved = await fs.readFile(fileFor(root), "utf8");
  const native = nativeBrowserEnvironment({ ...host, platform: "darwin", arch: "arm64", locale: "en-GB", timeZone: "UTC" });
  await (await launchPlatformBrowser({ ...options, platform: "xiaohongshu", native })).close();
  assert.equal(await fs.readFile(fileFor(root), "utf8"), saved);
  await (await launchPlatformBrowser({ ...options, native })).close();
  assert.notEqual(launches[0].profile, launches[1].profile);
  assert.equal(launches[2].options.locale, "en-GB");
  assert.deepEqual(JSON.parse(await fs.readFile(fileFor(root), "utf8")), native);
});

test("stored data cannot inject browser flags, user agents or proxies", async t => {
  const { root, launches, options } = await fixture(t);
  await (await launchPlatformBrowser(options)).close();
  await fs.writeFile(fileFor(root), JSON.stringify({ ...options.native,
    args: ["--untrusted-flag"], userAgent: "untrusted-agent", proxy: { server: "untrusted" } }));
  await (await launchPlatformBrowser(options)).close();
  assert.equal(launches[1].options.userAgent, undefined);
  assert.equal(launches[1].options.proxy, undefined);
  assert.equal(launches[1].options.args.includes("--untrusted-flag"), false);
});

test("corrupt, future or invalid environment state fails without replacing the file", async t => {
  const { root, launches, options } = await fixture(t);
  const file = fileFor(root);
  await fs.mkdir(path.dirname(file), { recursive: true });
  for (const value of ["{", "null", JSON.stringify({ ...options.native, schema_version: 2 }),
    JSON.stringify({ ...options.native, timezone_id: "invalid-zone" }),
    JSON.stringify({ ...options.native, locale: "not a locale" }),
    JSON.stringify({ ...options.native, viewport: { width: -1, height: 900 } })]) {
    await fs.writeFile(file, value);
    await assert.rejects(launchPlatformBrowser(options), { message: "browser_environment_invalid" });
    assert.equal(await fs.readFile(file, "utf8"), value);
  }
  assert.equal(launches.length, 0);
});

test("unreadable state and invalid platform do not launch a browser", async t => {
  const { root, launches, options } = await fixture(t);
  await fs.mkdir(fileFor(root), { recursive: true });
  await assert.rejects(launchPlatformBrowser(options), { message: "browser_environment_unavailable" });
  await assert.rejects(launchPlatformBrowser({ ...options, platform: "../invalid" }), { message: "platform_unsupported" });
  assert.equal(launches.length, 0);
});

test("failed browser launch does not commit environment state", async t => {
  const { root, options } = await fixture(t);
  const chromium = { launchPersistentContext: async () => { throw new Error("profile_locked"); } };
  await assert.rejects(launchPlatformBrowser({ ...options, chromium }), { message: "profile_locked" });
  await assert.rejects(fs.stat(fileFor(root)), { code: "ENOENT" });
});

test("failed environment persistence closes the browser and returns a stable error", async t => {
  const { options, closed } = await fixture(t);
  t.mock.method(fs, "rename", async () => { throw new Error("write_failed"); });
  await assert.rejects(launchPlatformBrowser(options), { message: "browser_environment_unavailable" });
  assert.equal(closed(), 1);
});

test("environment errors remain machine-visible and do not open verification windows", async t => {
  const { root } = await fixture(t);
  let windows = 0;
  for (const errorCode of ["browser_environment_invalid", "browser_environment_unavailable", "browser_platform_unsupported"]) {
    const result = await handleRequest({ args: { action: "run_once", platform: "douyin" },
      context: { skill_storage: { storage_kind: "directory", directory_path: root } } }, {
      collectPlatform: async () => { throw new Error(errorCode); },
      waitForInteractiveLogin: async () => { windows += 1; },
    });
    assert.equal(result.extra.error_code, errorCode);
    assert.equal(result.extra.retryable, false);
  }
  assert.equal(windows, 0);
});
