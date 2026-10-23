import assert from "node:assert/strict";
import path from "node:path";
import test from "node:test";
import { browserProfileDirectory, selectMacOSBrowser } from "../src/macos_browser_selection.mjs";

function fixture({ chromeVersion = "156.0.1", chromeError, available = ["chrome", "edge"] } = {}) {
  const calls = [];
  const options = {
    hostPlatform: "darwin", applications: ["/fixtures/Applications"],
    access: async file => {
      const kind = file.includes("Google Chrome.app") ? "chrome" : "edge";
      if (!available.includes(kind)) throw new Error("ENOENT");
    },
    execute: async (program, args, settings) => {
      calls.push({ program, args, settings });
      const chrome = args.at(-1).includes("Google Chrome.app");
      if (program.endsWith("plutil")) {
        return { stdout: JSON.stringify({
          CFBundleIdentifier: chrome ? "com.google.Chrome" : "com.microsoft.edgemac",
          CFBundleExecutable: chrome ? "Google Chrome" : "Microsoft Edge",
          CFBundleShortVersionString: chrome ? chromeVersion : "155.0.1",
        }) };
      }
      if (chrome && chromeError) throw chromeError;
      return { stdout: "" };
    },
  };
  return { options, calls };
}

test("signed system Chrome wins without launching a browser or reading personal cookies", async () => {
  const { options, calls } = fixture();
  const result = await selectMacOSBrowser(options);
  assert.equal(result.bundleId, "com.google.Chrome");
  assert.equal(result.executablePath, "/fixtures/Applications/Google Chrome.app/Contents/MacOS/Google Chrome");
  assert.deepEqual(calls.map(call => call.program), ["/usr/bin/plutil", "/usr/bin/codesign"]);
  assert.match(calls[1].args[3], /EQHXZ8M8AV/u);
  assert.equal(calls[1].settings.timeout, 5000);
});

test("obsolete or signature-invalid Chrome does not hide signed Edge", async () => {
  for (const settings of [{ chromeVersion: "65.0.1" }, { chromeError: new Error("signature_invalid") },
    { chromeError: Object.assign(new Error("timeout"), { killed: true }) }]) {
    const { options } = fixture(settings);
    assert.equal((await selectMacOSBrowser(options)).bundleId, "com.microsoft.edgemac");
  }
});

test("invalid metadata and missing native apps never fall back to a testing browser", async () => {
  const { options } = fixture({ chromeVersion: "invalid", available: ["chrome"] });
  assert.equal(await selectMacOSBrowser(options), null);
  assert.equal(await selectMacOSBrowser(fixture({ available: [] }).options), null);
});

test("Linux does not execute macOS commands", async () => {
  const { options, calls } = fixture();
  assert.equal(await selectMacOSBrowser({ ...options, hostPlatform: "linux" }), null);
  assert.equal(calls.length, 0);
});

test("browser profile identity is stable across upgrades but isolated from old Chromium and Edge", () => {
  const root = "/private/skill";
  const chrome = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
  const edge = "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge";
  assert.equal(browserProfileDirectory(root, "douyin", chrome, "darwin"),
    path.join(root, "browser-profile/macos-com.google.Chrome/douyin"));
  assert.notEqual(browserProfileDirectory(root, "douyin", chrome, "darwin"),
    browserProfileDirectory(root, "douyin", edge, "darwin"));
  assert.equal(browserProfileDirectory(root, "douyin", chrome, "linux"),
    path.join(root, "browser-profile/douyin"));
});
