import fs from "node:fs/promises";
import path from "node:path";
import { writeAtomic } from "./csv.mjs";
import { SUPPORTED_PLATFORMS } from "./platforms.mjs";

const ENVIRONMENT_FILE = "browser-environment.json";

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

export async function launchPlatformBrowser({ chromium, root, platform, executablePath, headless,
  native = nativeBrowserEnvironment() }) {
  if (!SUPPORTED_PLATFORMS.includes(platform)) throw new Error("platform_unsupported");
  native = validateEnvironment(native);
  const profile = path.join(root, "browser-profile", platform);
  await fs.mkdir(profile, { recursive: true });
  const file = path.join(profile, ENVIRONMENT_FILE);
  const { environment, needsSave } = await readEnvironment(file, native);
  const context = await chromium.launchPersistentContext(profile, {
    executablePath,
    headless,
    locale: environment.locale,
    timezoneId: environment.timezone_id,
    viewport: environment.viewport,
    // UA/client hints, OS, fonts and graphics remain native to the installed browser.
    args: !headless && process.platform === "linux" && process.env.WAYLAND_DISPLAY
      ? ["--ozone-platform=wayland"] : [],
  });
  try {
    // Only persist after Chromium has successfully acquired this profile's lock.
    if (needsSave) await writeAtomic(file, `${JSON.stringify(environment, null, 2)}\n`);
    return context;
  } catch {
    await context.close().catch(() => {});
    throw new Error("browser_environment_unavailable");
  }
}
