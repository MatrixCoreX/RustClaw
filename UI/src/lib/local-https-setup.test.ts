import assert from "node:assert/strict";
import test from "node:test";

import {
  buildSecureDeviceUrl,
  certificateInstallInstructions,
  resolveClientBrowser,
  resolveClientPlatform,
} from "../components/LocalHttpsSetupPanel";

const zh = (value: string) => value;
const en = (_zh: string, value: string) => value;

test("detects supported client operating systems without relying on UI language", () => {
  assert.equal(resolveClientPlatform("Mozilla/5.0 (Windows NT 10.0)", "Win32"), "windows");
  assert.equal(resolveClientPlatform("Mozilla/5.0 (X11; Linux x86_64)", "Linux x86_64"), "linux");
  assert.equal(resolveClientPlatform("Mozilla/5.0 (Linux; Android 16)", "Linux armv8l"), "android");
  assert.equal(resolveClientPlatform("Mozilla/5.0 (X11; CrOS x86_64)", "Linux x86_64"), "chromeos");
  assert.equal(resolveClientPlatform("Mozilla/5.0 (Macintosh)", "MacIntel"), "macos");
  assert.equal(resolveClientPlatform("Mozilla/5.0 (Macintosh)", "MacIntel", 5), "ios");
});

test("detects browser engine labels from stable user-agent tokens", () => {
  assert.equal(resolveClientBrowser("Mozilla/5.0 Firefox/142.0"), "firefox");
  assert.equal(resolveClientBrowser("Mozilla/5.0 Chrome/140.0 Edg/140.0"), "edge");
  assert.equal(resolveClientBrowser("Mozilla/5.0 Version/18.0 Safari/605.1.15"), "safari");
  assert.equal(resolveClientBrowser("Mozilla/5.0 Chrome/140.0 Safari/537.36"), "chrome");
});

test("secure device URL uses the standard HTTPS port and clears transient routes", () => {
  assert.equal(
    buildSecureDeviceUrl("http://192.168.31.243:8788/dashboard?tab=host#nginx"),
    "https://192.168.31.243/",
  );
  assert.equal(buildSecureDeviceUrl("https://device.example/settings"), "https://device.example/");
});

test("desktop certificate help follows the selected browser certificate store", () => {
  const linuxChrome = certificateInstallInstructions("linux", "chrome", zh).join("\n");
  const linuxEdge = certificateInstallInstructions("linux", "edge", zh).join("\n");
  const linuxFirefox = certificateInstallInstructions("linux", "firefox", zh).join("\n");

  assert.match(linuxChrome, /Chrome/);
  assert.match(linuxChrome, /管理证书/);
  assert.match(linuxEdge, /Edge/);
  assert.match(linuxEdge, /隐私、搜索和服务/);
  assert.match(linuxFirefox, /查看证书/);
  assert.match(linuxFirefox, /证书颁发机构/);
  assert.doesNotMatch(linuxFirefox, /Chrome|Edge/);
});

test("Windows and macOS help names the correct system trust store", () => {
  const windowsEdge = certificateInstallInstructions("windows", "edge", en).join("\n");
  const macSafari = certificateInstallInstructions("macos", "safari", en).join("\n");

  assert.match(windowsEdge, /Trusted Root Certification Authorities/);
  assert.match(windowsEdge, /Current User/);
  assert.match(macSafari, /System keychain/);
  assert.match(macSafari, /Always Trust/);
});

test("mobile certificate help includes the extra system trust confirmation", () => {
  assert.match(
    certificateInstallInstructions("ios", "safari", en).join("\n"),
    /Enable Full Trust for Root Certificates/,
  );
  assert.match(
    certificateInstallInstructions("android", "chrome", en).join("\n"),
    /Trusted credentials/,
  );
});
