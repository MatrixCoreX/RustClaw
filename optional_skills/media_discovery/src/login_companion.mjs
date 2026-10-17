import { randomUUID } from "node:crypto";
import fsNative from "node:fs";
import fs from "node:fs/promises";
import path from "node:path";

const HEARTBEAT_MAX_AGE_SECONDS = 10;
const ACTION_MAX_AGE_SECONDS = 30;
const REFRESH_INTERVAL_MS = 2_000;
const MAX_STATE_BYTES = 16 * 1024;
const MAX_IMAGE_BYTES = 5 * 1024 * 1024;
const PNG_SIGNATURE = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
const PLATFORM_RE = /^[a-z][a-z0-9_]{0,47}$/u;
const HANDOFF_RE = /^[a-f0-9-]{16,64}$/u;

function runtimeDirectory(environment = process.env, explicitDirectory) {
  if (explicitDirectory) return explicitDirectory;
  const configured = String(environment.XDG_RUNTIME_DIR || "").trim();
  if (configured) return configured;
  if (process.platform === "linux" && typeof process.geteuid === "function") {
    return `/run/user/${process.geteuid()}`;
  }
  return "";
}

function companionFile(name, options = {}) {
  const runtime = runtimeDirectory(options.environment, options.runtimeDirectory);
  return runtime ? path.join(runtime, "agent-runtime", name) : "";
}

function boundedJson(file, maximumBytes) {
  try {
    const stat = fsNative.lstatSync(file);
    if (stat.isSymbolicLink() || !stat.isFile() || stat.size <= 1 || stat.size > maximumBytes) return null;
    return JSON.parse(fsNative.readFileSync(file, "utf8"));
  } catch {
    return null;
  }
}

export function loginCompanionActive({
  environment = process.env,
  runtimeDirectory: explicitDirectory,
  now = Date.now() / 1000,
  hostPlatform = process.platform,
} = {}) {
  if (hostPlatform !== "linux") return false;
  const file = companionFile("login-challenge-companion.json", {
    environment,
    runtimeDirectory: explicitDirectory,
  });
  if (!file) return false;
  const state = boundedJson(file, 4 * 1024);
  const updated = state?.updated_at_epoch;
  return state?.schema_version === 1
    && state.status === "active"
    && state.device_class === "raspberry_pi"
    && typeof updated === "number"
    && Number.isFinite(updated)
    && updated <= now + 5
    && now - updated <= HEARTBEAT_MAX_AGE_SECONDS;
}

export function loginCompanionBrowserArgs(options = {}) {
  if (!loginCompanionActive(options)) return [];
  return [
    "--start-minimized",
    "--disable-backgrounding-occluded-windows",
    "--disable-renderer-backgrounding",
  ];
}

async function atomicWrite(file, content) {
  await fs.mkdir(path.dirname(file), { recursive: true, mode: 0o700 });
  await fs.chmod(path.dirname(file), 0o700).catch(() => {});
  const temporary = path.join(path.dirname(file), `.${path.basename(file)}.${process.pid}.${randomUUID()}.tmp`);
  const handle = await fs.open(temporary, "wx", 0o600);
  try {
    await handle.writeFile(content);
    await handle.sync();
  } finally {
    await handle.close();
  }
  await fs.rename(temporary, file);
}

function validPng(image) {
  return Buffer.isBuffer(image)
    && image.length > PNG_SIGNATURE.length
    && image.length <= MAX_IMAGE_BYTES
    && image.subarray(0, PNG_SIGNATURE.length).equals(PNG_SIGNATURE);
}

async function elementScreenshot(page) {
  const selectors = [
    'iframe[src*="/verifycenter/captcha/"]:visible',
    '[class*="captcha" i]:visible',
    '[id*="captcha" i]:visible',
    '[class*="qrcode" i]:visible',
    '[class*="qr-code" i]:visible',
    '[id*="qrcode" i]:visible',
  ];
  for (const selector of selectors) {
    try {
      const locator = page.locator(selector).first();
      if (await locator.count() === 0) continue;
      const box = await locator.boundingBox();
      if (!box || box.width < 72 || box.height < 72 || box.width > 1_600 || box.height > 1_600) continue;
      const image = await locator.screenshot({ type: "png" });
      if (validPng(image)) return { image, cropped: true };
    } catch {
      // A cross-origin verification frame can be replaced while it is captured.
    }
  }
  try {
    const image = await page.screenshot({ type: "png", fullPage: false });
    if (validPng(image)) return { image, cropped: false };
  } catch {
    // The caller falls back to the ordinary visible browser flow.
  }
  return null;
}

export async function showBrowserWindow(page) {
  let session;
  try {
    session = await page.context().newCDPSession(page);
    const window = await session.send("Browser.getWindowForTarget");
    if (Number.isInteger(window?.windowId)) {
      await session.send("Browser.setWindowBounds", {
        windowId: window.windowId,
        bounds: { windowState: "normal" },
      });
    }
  } catch {
    // Some Chromium builds/window managers do not expose window bounds.
  } finally {
    await session?.detach?.().catch(() => {});
  }
  if (typeof page?.bringToFront === "function") await page.bringToFront().catch(() => {});
}

async function readBrowserAction(handoffId, options = {}) {
  const file = companionFile("login-challenge-action.json", options);
  if (!file) return false;
  const action = boundedJson(file, 4 * 1024);
  const requestedAt = action?.requested_at_epoch;
  const now = options.now ?? Date.now() / 1000;
  const valid = action?.schema_version === 1
    && action.action === "show_browser"
    && action.handoff_id === handoffId
    && typeof requestedAt === "number"
    && Number.isFinite(requestedAt)
    && requestedAt <= now + 5
    && now - requestedAt <= ACTION_MAX_AGE_SECONDS;
  if (valid || (typeof requestedAt === "number" && now - requestedAt > ACTION_MAX_AGE_SECONDS)) {
    await fs.unlink(file).catch(() => {});
  }
  return valid;
}

export async function createLoginCompanionHandoff({
  root,
  page,
  platform,
  reasonCode,
  environment = process.env,
  runtimeDirectory: explicitDirectory,
  now = () => Date.now(),
} = {}) {
  const companionOptions = { environment, runtimeDirectory: explicitDirectory };
  if (!root || !page || !PLATFORM_RE.test(String(platform || ""))
    || !loginCompanionActive(companionOptions)) return null;

  const handoffId = randomUUID();
  const challengeDirectory = path.join(root, "login_challenge");
  const imageFile = `challenge-${handoffId}.png`;
  const imagePath = path.join(challengeDirectory, imageFile);
  const statePath = path.join(challengeDirectory, "current.json");
  let lastRefresh = 0;
  let active = true;

  const clear = async () => {
    if (!active) return;
    active = false;
    const state = boundedJson(statePath, MAX_STATE_BYTES);
    if (state?.handoff_id === handoffId) await fs.unlink(statePath).catch(() => {});
    await fs.unlink(imagePath).catch(() => {});
  };

  const refresh = async ({ force = false } = {}) => {
    if (!active) return false;
    if (!loginCompanionActive(companionOptions)) {
      await showBrowserWindow(page);
      await clear();
      return false;
    }
    const current = now();
    if (!force && current - lastRefresh < REFRESH_INTERVAL_MS) return true;
    const captured = await elementScreenshot(page);
    if (!captured) return false;
    await atomicWrite(imagePath, captured.image);
    await atomicWrite(statePath, Buffer.from(JSON.stringify({
      schema_version: 1,
      status: "awaiting_user",
      platform,
      image_file: imageFile,
      cropped: captured.cropped,
      updated_at_epoch: current / 1000,
      handoff_id: handoffId,
      interaction: "browser_window",
      reason_code: String(reasonCode || "challenge_required"),
    })));
    lastRefresh = current;
    return true;
  };

  if (!await refresh({ force: true })) {
    await clear();
    return null;
  }

  return {
    handoffId,
    refresh,
    clear,
    revealIfRequested: async () => {
      if (!active || !await readBrowserAction(handoffId, companionOptions)) return false;
      await showBrowserWindow(page);
      return true;
    },
  };
}
