import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import {
  createLoginCompanionHandoff,
  loginCompanionActive,
  loginCompanionBrowserArgs,
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
  };
  return { page, commands, broughtToFront: () => broughtToFront };
}

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

test("challenge screenshot stays private and a matching Pi action reveals Chromium", async t => {
  const fixture = await runtimeFixture(t);
  const browser = browserPage();
  const handoff = await createLoginCompanionHandoff({
    root: fixture.root,
    page: browser.page,
    platform: "douyin",
    reasonCode: "challenge_required",
    environment: fixture.environment,
  });
  assert.ok(handoff);

  const statePath = path.join(fixture.root, "login_challenge", "current.json");
  const state = JSON.parse(await fs.readFile(statePath, "utf8"));
  assert.equal(state.platform, "douyin");
  assert.equal(state.interaction, "browser_window");
  assert.equal(state.reason_code, "challenge_required");
  assert.equal(state.handoff_id, handoff.handoffId);
  assert.equal(JSON.stringify(state).includes("cookie"), false);
  assert.equal(JSON.stringify(state).includes("https://"), false);
  assert.equal((await fs.stat(statePath)).mode & 0o777, 0o600);

  await fs.writeFile(path.join(fixture.directory, "login-challenge-action.json"), JSON.stringify({
    schema_version: 1,
    action: "show_browser",
    handoff_id: handoff.handoffId,
    requested_at_epoch: Date.now() / 1000,
  }));
  assert.equal(await handoff.revealIfRequested(), true);
  assert.equal(browser.broughtToFront(), 1);
  assert.deepEqual(browser.commands.map(({ method }) => method), [
    "Browser.getWindowForTarget",
    "Browser.setWindowBounds",
  ]);
  await assert.rejects(fs.stat(path.join(fixture.directory, "login-challenge-action.json")),
    { code: "ENOENT" });

  await handoff.clear();
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
  await fs.writeFile(path.join(fixture.directory, "login-challenge-action.json"), JSON.stringify({
    schema_version: 1,
    action: "show_browser",
    handoff_id: "00000000-0000-0000-0000-000000000000",
    requested_at_epoch: Date.now() / 1000,
  }));
  assert.equal(await handoff.revealIfRequested(), false);
  assert.equal(browser.broughtToFront(), 0);
  await handoff.clear();
});
