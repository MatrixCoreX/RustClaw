import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { launchPlatformBrowser, nativeBrowserEnvironment } from "../src/browser_environment.mjs";
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
    assert.equal(actual.ignoreDefaultArgs, undefined);
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
