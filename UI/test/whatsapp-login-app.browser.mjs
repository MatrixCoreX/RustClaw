import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { isolatedUiServer } from "./helpers/isolated-ui-server.mjs";
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || "playwright");
const output = process.env.UI_BROWSER_ARTIFACTS || "/tmp/whatsapp-login-app";
await mkdir(output, { recursive: true });
const ui = await isolatedUiServer(fileURLToPath(new URL("../", import.meta.url)));
const origins = ["https://device-a.example.test", "https://device-b.example.test"];
const png = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/hS4AAAAASUVORK5CYII=";
const results = [];
let browser;
try {
  browser = await chromium.launch({ executablePath: process.env.BROWSER_EXECUTABLE || undefined, headless: true });
  for (const theme of ["light", "dark"]) for (const phase of ["status", "logout"]) {
    const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
    const requests = [], errors = [];
    let loggedIn = false, watch = false, held;
    const snapshot = origin => ({ connected: phase === "logout" && origin === origins[0], qr_ready: true,
      qr_data_url: `${png}#${origin === origins[0] ? "device-a" : "device-b"}`, phase: "qr_ready" });
    const wait = async predicate => {
      const deadline = Date.now() + 5000;
      while (!(await predicate())) { if (Date.now() >= deadline) throw new Error("fixture_state_timeout"); await page.waitForTimeout(10); }
    };
    const reply = (route, data, status = 200) => route.fulfill({ status, contentType: "application/json", headers: {
      "Access-Control-Allow-Origin": ui.base, "Access-Control-Allow-Credentials": "true",
      "Access-Control-Allow-Headers": "content-type,x-agent-key,x-webd-csrf", "Access-Control-Allow-Methods": "GET,POST,OPTIONS",
    }, body: JSON.stringify({ ok: status === 200, data, error: status === 200 ? null : "fixture_unavailable" }) });
    await page.addInitScript(value => {
      localStorage.setItem("agent-runtime.monitor.currentPage", "services");
      localStorage.setItem("agent-runtime.monitor.pollingSeconds", "0");
      localStorage.setItem("agent-runtime.monitor.lang", "en");
      localStorage.setItem("agent-runtime.monitor.themeMode", value);
    }, theme);
    page.on("pageerror", error => errors.push(error.message));
    await page.route("**/*", async route => {
      const request = route.request(), url = new URL(request.url());
      if (url.origin === ui.base && !url.pathname.startsWith("/v1/") && !url.pathname.startsWith("/webd/")) return route.continue();
      if (!origins.includes(url.origin)) return route.abort();
      if (request.method() === "OPTIONS") return reply(route, null);
      requests.push({ origin: url.origin, path: url.pathname, method: request.method() });
      if (url.pathname === "/webd/login") { loggedIn = true; return reply(route, { csrf_token: "ab".repeat(16) }); }
      if (url.pathname === "/webd/logout") { loggedIn = false; return reply(route, {}); }
      if (url.pathname === "/webd/session") return reply(route, { logged_in: loggedIn, csrf_token: "ab".repeat(16) });
      if (url.pathname === "/v1/auth/me") return reply(route, { user_id: 101, chat_id: 301, role: "admin", user_key: "fixture-admin-key" });
      if (url.pathname === "/v1/health") return reply(route, { version: "fixture", queue_length: 0, running_length: 0,
        worker_state: "idle", uptime_seconds: 1, task_timeout_seconds: 0, running_oldest_age_seconds: 0, whatsapp_web_healthy: true, wechatd_healthy: false });
      if (url.pathname === "/v1/whatsapp-web/login-status") {
        if (watch && phase === "status" && url.origin === origins[0] && !held) { held = route; return; }
        return reply(route, snapshot(url.origin));
      }
      if (url.pathname === "/v1/whatsapp-web/logout" && watch && phase === "logout") { held = route; return; }
      return reply(route, null, 503);
    });
    const card = page.locator(".channel-setup-card").filter({ has: page.getByRole("heading", { name: "WhatsApp Web", exact: true }) });
    const login = async (origin, connected = false) => {
      await page.getByLabel("Webd URL (optional)", { exact: true }).fill(origin);
      await page.getByLabel("Username", { exact: true }).fill("fixture-user");
      await page.getByLabel("Password", { exact: true }).fill("fixture-password");
      await page.getByRole("button", { name: "Enter Console", exact: true }).click();
      await page.getByRole("button", { name: "Log out", exact: true }).waitFor();
      await card.getByRole("button", { name: connected ? "Sign out" : "Show QR code", exact: true }).waitFor();
    };
    try {
      await page.goto(ui.base); await page.getByLabel("Webd URL (optional)", { exact: true }).waitFor(); page.setDefaultTimeout(5000);
      await login(origins[0], phase === "logout");
      if (phase === "status") {
        await card.getByRole("button", { name: "Show QR code", exact: true }).click();
        await wait(async () => (await page.getByAltText("WhatsApp Web QR").getAttribute("src"))?.endsWith("#device-a"));
        watch = true; await card.getByRole("button", { name: "Refresh status", exact: true }).click();
      } else {
        watch = true; await card.getByRole("button", { name: "Sign out", exact: true }).click();
      }
      await wait(() => held);
      await page.getByRole("button", { name: "Log out", exact: true }).click();
      await login(origins[1]); assert.equal(await page.getByAltText("WhatsApp Web QR").count(), 0);
      const before = requests.length, response = page.waitForResponse(value => value.request() === held.request());
      await reply(held, phase === "status" ? snapshot(origins[0]) : {});
      await (await response).finished(); await page.waitForTimeout(1300);
      assert.deepEqual(requests.slice(before).filter(row => row.origin === origins[0]), []);
      assert.equal(await page.getByAltText("WhatsApp Web QR").count(), 0);
      await card.getByRole("button", { name: "Show QR code", exact: true }).click();
      await wait(async () => (await page.getByAltText("WhatsApp Web QR").getAttribute("src"))?.endsWith("#device-b"));
      assert.equal(await page.getByAltText("WhatsApp Web QR").evaluate(node => node.naturalWidth), 1);
      assert.deepEqual(errors, []);
      await page.screenshot({ path: `${output}/app-${theme}-${phase}.png` });
      results.push({ theme, phase, passed: true });
    } catch (error) { results.push({ theme, phase, passed: false, error: String(error), errors, requests }); }
    finally { await page.close(); }
  }
  await writeFile(`${output}/report.json`, JSON.stringify({ results, real_provider_calls: 0, actual_signouts: 0 }, null, 2));
  console.log(JSON.stringify({ results, real_provider_calls: 0, actual_signouts: 0 }));
  assert.deepEqual(results.filter(row => !row.passed), []);
} finally { await browser?.close(); await ui.close(); }
