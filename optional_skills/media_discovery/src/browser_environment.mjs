import fsNative from "node:fs";
import fs from "node:fs/promises";
import path from "node:path";
import { writeAtomic } from "./csv.mjs";
import { SUPPORTED_PLATFORMS } from "./platforms.mjs";

const ENVIRONMENT_FILE = "browser-environment.json";

function isDirectory(value) {
  try { return fsNative.statSync(value).isDirectory(); } catch { return false; }
}

function firstMatchingEntry(directory, pattern, predicate) {
  try {
    for (const name of fsNative.readdirSync(directory).sort()) {
      if (!pattern.test(name)) continue;
      const candidate = path.join(directory, name);
      try {
        if (predicate(fsNative.statSync(candidate))) return { name, path: candidate };
      } catch {
        // A desktop session can disappear while its runtime directory is read.
      }
    }
  } catch {
    // Missing or unreadable session directories mean no discovered display.
  }
  return null;
}

export function desktopSessionEnvironment(environment = process.env, {
  hostPlatform = process.platform,
  effectiveUid = typeof process.geteuid === "function" ? process.geteuid() : null,
  runtimeDirectory,
  x11SocketDirectory = "/tmp/.X11-unix",
} = {}) {
  const resolved = { ...environment };
  if (hostPlatform !== "linux") return resolved;
  const runtime = runtimeDirectory || resolved.XDG_RUNTIME_DIR
    || (Number.isInteger(effectiveUid) ? `/run/user/${effectiveUid}` : "");
  if (!runtime || !isDirectory(runtime)) return resolved;

  resolved.XDG_RUNTIME_DIR ||= runtime;
  if (!resolved.WAYLAND_DISPLAY) {
    const wayland = firstMatchingEntry(runtime, /^wayland-\d+$/u, stat => stat.isSocket());
    if (wayland) resolved.WAYLAND_DISPLAY = wayland.name;
  }
  if (!resolved.DISPLAY && isDirectory(x11SocketDirectory)) {
    const x11 = firstMatchingEntry(x11SocketDirectory, /^X\d+$/u, stat => stat.isSocket());
    if (x11) resolved.DISPLAY = `:${x11.name.slice(1)}`;
  }
  if (!resolved.XAUTHORITY) {
    const authority = firstMatchingEntry(runtime, /^\.mutter-Xwaylandauth\..+$/u, stat => stat.isFile());
    if (authority) resolved.XAUTHORITY = authority.path;
  }
  return resolved;
}

export function nativeBrowserEnvironment(host = {
  platform: process.platform,
  arch: process.arch,
  ...Intl.DateTimeFormat().resolvedOptions(),
}) {
  if (!["linux", "darwin"].includes(host.platform)) throw new Error("browser_platform_unsupported");
  return validateEnvironment({
    schema_version: 1,
    host_platform: host.platform,
    host_arch: host.arch,
    locale: host.locale,
    timezone_id: host.timeZone,
    // This is the existing collection window, not a claim about the physical monitor.
    viewport: { width: 1280, height: 900 },
  });
}

function validateEnvironment(value) {
  try {
    if (value?.schema_version !== 1 || !["linux", "darwin"].includes(value.host_platform)
      || typeof value.host_arch !== "string" || !value.host_arch
      || typeof value.locale !== "string" || !value.locale
      || typeof value.timezone_id !== "string" || !value.timezone_id
      || ![value.viewport?.width, value.viewport?.height].every(size =>
        Number.isInteger(size) && size > 0 && size <= 8192)) throw new Error();
    const locale = Intl.getCanonicalLocales(value.locale)[0];
    const timezoneId = new Intl.DateTimeFormat(locale, { timeZone: value.timezone_id })
      .resolvedOptions().timeZone;
    // Project known fields only; stored data cannot add browser flags, proxies or scripts.
    return { schema_version: 1, host_platform: value.host_platform, host_arch: value.host_arch,
      locale, timezone_id: timezoneId,
      viewport: { width: value.viewport.width, height: value.viewport.height } };
  } catch {
    throw new Error("browser_environment_invalid");
  }
}

async function readEnvironment(file, native) {
  let saved;
  try {
    saved = await fs.readFile(file, "utf8");
  } catch (error) {
    if (error.code === "ENOENT") return { environment: native, needsSave: true };
    throw new Error("browser_environment_unavailable");
  }
  let environment;
  try { environment = validateEnvironment(JSON.parse(saved)); }
  catch { throw new Error("browser_environment_invalid"); }
  // A profile copied to another OS/architecture must not emulate the old host.
  if (environment.host_platform !== native.host_platform || environment.host_arch !== native.host_arch) {
    return { environment: native, needsSave: true };
  }
  return { environment, needsSave: false };
}

export const AUTOMATION_DEFAULT_ARG = "--enable-automation";
export const AUTOMATION_BLINK_FLAG = "--disable-blink-features=AutomationControlled";

export function persistentContextLaunchOptions({
  executablePath,
  headless,
  environment,
  hostEnv = desktopSessionEnvironment(),
  hostPlatform = process.platform,
}) {
  const args = [AUTOMATION_BLINK_FLAG];
  if (!headless && hostPlatform === "linux" && hostEnv.WAYLAND_DISPLAY) {
    args.unshift("--ozone-platform=wayland");
  }
  return {
    executablePath,
    headless,
    locale: environment.locale,
    timezoneId: environment.timezone_id,
    viewport: environment.viewport,
    env: hostEnv,
    // Drop Chromium's default automation switch. Stored profile data still cannot
    // add a user-agent, proxy, or extra flags.
    ignoreDefaultArgs: [AUTOMATION_DEFAULT_ARG],
    args,
  };
}

export async function concealAutomationMarkers(context) {
  if (typeof context?.addInitScript !== "function") return;
  await context.addInitScript(() => {
    Object.defineProperty(navigator, "webdriver", { get: () => undefined });
  });
}

export async function launchPlatformBrowser({ chromium, root, platform, executablePath, headless,
  native = nativeBrowserEnvironment() }) {
  if (!SUPPORTED_PLATFORMS.includes(platform)) throw new Error("platform_unsupported");
  native = validateEnvironment(native);
  const profile = path.join(root, "browser-profile", platform);
  await fs.mkdir(profile, { recursive: true });
  const file = path.join(profile, ENVIRONMENT_FILE);
  const { environment, needsSave } = await readEnvironment(file, native);
  const context = await chromium.launchPersistentContext(profile, persistentContextLaunchOptions({
    executablePath,
    headless,
    environment,
  }));
  try {
    await concealAutomationMarkers(context);
    // Only persist after Chromium has successfully acquired this profile's lock.
    if (needsSave) await writeAtomic(file, `${JSON.stringify(environment, null, 2)}\n`);
    return context;
  } catch {
    await context.close().catch(() => {});
    throw new Error("browser_environment_unavailable");
  }
}
