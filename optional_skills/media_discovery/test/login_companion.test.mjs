import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import {
  createLoginCompanionHandoff,
  compactVerificationScale,
  loginChallengeSelectors,
  loginCompanionActive,
  loginCompanionBrowserArgs,
  raspberryPiDisplaySize,
} from "../src/login_companion.mjs";

const PNG = Buffer.concat([
  Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
  Buffer.from("fixture"),
]);

async function runtimeFixture(t) {
  const runtime = await fs.mkdtemp(path.join(os.tmpdir(), "media-discovery-companion-runtime-"));
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "media-discovery-companion-storage-"));
  t.after(() => fs.rm(runtime, { recursive: true, force: true }));
  t.after(() => fs.rm(root, { recursive: true, force: true }));
  const directory = path.join(runtime, "agent-runtime");
  await fs.mkdir(directory, { recursive: true });
  await fs.writeFile(path.join(directory, "login-challenge-companion.json"), JSON.stringify({
    schema_version: 1,
    status: "active",
    device_class: "raspberry_pi",
    updated_at_epoch: Date.now() / 1000,
  }));
  return { runtime, root, directory, environment: { XDG_RUNTIME_DIR: runtime } };
}

function browserPage() {
  const commands = [];
  const viewportChanges = [];
  const zoomChanges = [];
  let broughtToFront = 0;
  const session = {
    send: async (method, params) => {
      commands.push({ method, params });
      return method === "Browser.getWindowForTarget" ? { windowId: 7 } : {};
    },
    detach: async () => {},
  };
  const locator = {
    first: () => locator,
    count: async () => 1,
    boundingBox: async () => ({ x: 0, y: 0, width: 380, height: 384 }),
    screenshot: async () => PNG,
  };
  const page = {
    locator: () => locator,
    screenshot: async () => PNG,
    context: () => ({ newCDPSession: async () => session }),
    bringToFront: async () => { broughtToFront += 1; },
    viewportSize: () => ({ width: 1280, height: 900 }),
    setViewportSize: async viewport => { viewportChanges.push(viewport); },
    evaluate: async (_function, value) => {
      zoomChanges.push(value);
      return typeof value === "number" ? "" : null;
    },
  };
  return { page, commands, viewportChanges, zoomChanges, broughtToFront: () => broughtToFront };
}

test("compact verification scale leaves room for Pi browser notices and touch targets", () => {
  assert.equal(compactVerificationScale(
    { width: 480, height: 320 },
    { width: 380, height: 384 },
  ), 0.65);
  assert.equal(compactVerificationScale(
    { width: 800, height: 480 },
    { width: 380, height: 384 },
  ), 1);
  assert.equal(compactVerificationScale({ width: 480, height: 320 }, null), 1);
});

test("Weibo login capture prefers the first-party QR image", () => {
  const selectors = loginChallengeSelectors("weibo");
  assert.equal(selectors[0], 'img[src*="qr.weibo.cn" i]:visible');
  assert.ok(selectors.includes('[class*="qrcode" i]:visible'));
  assert.equal(
    loginChallengeSelectors("douyin")[0],
    'iframe[src*="/verifycenter/captcha/"]:visible',
  );
});

test("Raspberry Pi framebuffer size is validated before compact verification use", async t => {
  const directory = await fs.mkdtemp(path.join(os.tmpdir(), "media-discovery-display-"));
  t.after(() => fs.rm(directory, { recursive: true, force: true }));
  const file = path.join(directory, "virtual_size");
  await fs.writeFile(file, "480,320\n");
  assert.deepEqual(raspberryPiDisplaySize({ framebufferFile: file }), { width: 480, height: 320 });
  await fs.writeFile(file, "128,64\n");
  assert.equal(raspberryPiDisplaySize({ framebufferFile: file }), null);
  await fs.writeFile(file, "not-a-size\n");
  assert.equal(raspberryPiDisplaySize({ framebufferFile: file }), null);
});

test("fresh Raspberry Pi companion heartbeat enables a minimized browser handoff", async t => {
  const fixture = await runtimeFixture(t);
  const options = { environment: fixture.environment, hostPlatform: "linux" };
  assert.equal(loginCompanionActive(options), true);
  assert.deepEqual(loginCompanionBrowserArgs(options), [
    "--start-minimized",
    "--disable-backgrounding-occluded-windows",
    "--disable-renderer-backgrounding",
  ]);
  assert.equal(loginCompanionActive({ ...options, now: Date.now() / 1000 + 30 }), false);
  assert.equal(loginCompanionActive({ ...options, hostPlatform: "darwin" }), false);
});

for (const platform of ["douyin", "kuaishou", "weibo"]) test(
  `${platform} login QR stays private and a matching Pi action reveals Chromium`,
  async t => {
  const fixture = await runtimeFixture(t);
  const browser = browserPage();
  const handoff = await createLoginCompanionHandoff({
    root: fixture.root,
    page: browser.page,
    platform,
    reasonCode: "login_required",
    environment: fixture.environment,
    compactDisplay: { width: 480, height: 320 },
  });
  assert.ok(handoff);

  const statePath = path.join(fixture.root, "login_challenge", "current.json");
  const state = JSON.parse(await fs.readFile(statePath, "utf8"));
  assert.equal(state.platform, platform);
  assert.equal(state.interaction, "browser_window");
  assert.equal(state.reason_code, "login_required");
  assert.equal(state.handoff_id, handoff.handoffId);
  assert.equal(JSON.stringify(state).includes("cookie"), false);
  assert.equal(JSON.stringify(state).includes("https://"), false);
  assert.equal((await fs.stat(statePath)).mode & 0o777, 0o600);

  const actionPath = path.join(
    fixture.root,
    "login_challenge",
    `action-${handoff.handoffId}.json`,
  );
  await fs.writeFile(actionPath, JSON.stringify({
    schema_version: 1,
    action: "show_browser",
    handoff_id: handoff.handoffId,
    requested_at_epoch: Date.now() / 1000,
  }));
  assert.equal(await handoff.revealIfRequested(), true);
  assert.equal(browser.broughtToFront(), 1);
  assert.deepEqual(browser.viewportChanges, [{ width: 480, height: 320 }]);
  assert.deepEqual(browser.zoomChanges, [0.65]);
  assert.deepEqual(browser.commands.map(({ method }) => method), [
    "Browser.getWindowForTarget",
    "Browser.setWindowBounds",
  ]);
  assert.deepEqual(browser.commands[1].params.bounds, { windowState: "fullscreen" });
  await assert.rejects(fs.stat(actionPath), { code: "ENOENT" });

  await handoff.clear();
  assert.deepEqual(browser.viewportChanges, [
    { width: 480, height: 320 },
    { width: 1280, height: 900 },
  ]);
  assert.deepEqual(browser.zoomChanges, [0.65, ""]);
  assert.deepEqual(browser.commands.slice(2).map(({ method }) => method), [
    "Browser.getWindowForTarget",
    "Browser.setWindowBounds",
    "Browser.setWindowBounds",
  ]);
  assert.deepEqual(browser.commands.at(-1).params.bounds, { windowState: "minimized" });
  await assert.rejects(fs.stat(statePath), { code: "ENOENT" });
  await assert.rejects(fs.stat(path.join(fixture.root, "login_challenge", state.image_file)),
    { code: "ENOENT" });
});

test("a mismatched browser action cannot reveal a verification window", async t => {
  const fixture = await runtimeFixture(t);
  const browser = browserPage();
  const handoff = await createLoginCompanionHandoff({
    root: fixture.root,
    page: browser.page,
    platform: "douyin",
    environment: fixture.environment,
  });
  assert.ok(handoff);
  const actionPath = path.join(
    fixture.root,
    "login_challenge",
    `action-${handoff.handoffId}.json`,
  );
  await fs.writeFile(actionPath, JSON.stringify({
    schema_version: 1,
    action: "show_browser",
    handoff_id: "00000000-0000-0000-0000-000000000000",
    requested_at_epoch: Date.now() / 1000,
  }));
  assert.equal(await handoff.revealIfRequested(), false);
  assert.equal(browser.broughtToFront(), 0);
  await handoff.clear();
});
