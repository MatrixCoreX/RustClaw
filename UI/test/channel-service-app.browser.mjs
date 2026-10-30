import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { isolatedUiServer } from "./helpers/isolated-ui-server.mjs";
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || "playwright");

const output = process.env.UI_BROWSER_ARTIFACTS || "/tmp/channel-service-app-boundary";
await mkdir(output, { recursive: true });
const ui = await isolatedUiServer(fileURLToPath(new URL("../", import.meta.url)));
const origins = ["https://device-a.example.test", "https://device-b.example.test"];
const config = { config_path: "configs/channels/wechat.toml", enabled: false, listen: "127.0.0.1:8792",
  clawd_base_url: "http://127.0.0.1:8787", api_base_url: "https://ilinkai.weixin.qq.com", wechat_uin_base64: "",
  request_timeout_seconds: 30, longpoll_timeout_ms: 35000, text_chunk_chars: 1200,
  bot_token_configured: false, saved_session_present: false, restart_required: true };
const results = [];
let browser;
async function waitFor(predicate) {
  const deadline = Date.now() + 10000;
  while (!predicate()) {
    if (Date.now() >= deadline) throw new Error("fixture_request_not_received");
    await new Promise(resolve => setTimeout(resolve, 10));
  }
}
try {
  browser = await chromium.launch({ executablePath: process.env.BROWSER_EXECUTABLE || undefined, headless: true });
  for (const phase of ["enable", "service", "config", "current"]) {
    const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
    const requests = [], errors = [];
    let held, loggedIn = false, serviceStarted = false, postStartConfigReads = 0, qrRequested = false;
    const reply = (route, data, status = 200) => route.fulfill({ status, contentType: "application/json", headers: {
      "Access-Control-Allow-Origin": ui.base, "Access-Control-Allow-Credentials": "true",
      "Access-Control-Allow-Headers": "content-type,x-agent-key,x-webd-csrf", "Access-Control-Allow-Methods": "GET,POST,OPTIONS",
    }, body: JSON.stringify({ ok: status === 200, data, error: status === 200 ? null : "fixture_unavailable" }) });
    page.on("pageerror", error => errors.push(error.message));
    await page.addInitScript(() => {
      localStorage.setItem("agent-runtime.monitor.currentPage", "services");
      localStorage.setItem("agent-runtime.monitor.pollingSeconds", "0");
      localStorage.setItem("agent-runtime.monitor.lang", "en");
      localStorage.setItem("agent-runtime.monitor.themeMode", "light");
    });
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
        worker_state: "idle", uptime_seconds: 1, task_timeout_seconds: 0, running_oldest_age_seconds: 0, wechatd_healthy: false });
      if (url.pathname === "/v1/wechat/config") {
        if (url.origin === origins[0] && request.method() === "POST" && phase === "enable" && !held) { held = route; return; }
        if (url.origin === origins[0] && request.method() === "GET" && serviceStarted && phase === "config") {
          postStartConfigReads++;
          if (postStartConfigReads === 2) { held = route; return; }
        }
        return reply(route, { ...config, enabled: request.method() === "POST" });
      }
      if (url.pathname === "/v1/services/wechatd/start") {
        if (phase === "service" && !held) { held = route; return; }
        serviceStarted = true; return reply(route, {});
      }
      if (url.pathname === "/v1/wechat/login-qr/start") { qrRequested = true; return reply(route, null, 503); }
      if (url.pathname === "/v1/wechat/login-status") return reply(route, { connected: false, qr_ready: false, status: "disconnected" });
      return reply(route, null, 503);
    });
    const login = async origin => {
      await page.getByLabel("Webd URL (optional)", { exact: true }).fill(origin);
      await page.getByLabel("Username", { exact: true }).fill("fixture-user");
      await page.getByLabel("Password", { exact: true }).fill("fixture-password");
      await page.getByRole("button", { name: "Enter Console", exact: true }).click();
      await page.getByRole("button", { name: "Log out", exact: true }).waitFor();
      await page.getByRole("button", { name: "Enable WeChat and start setup", exact: true }).waitFor();
    };
    try {
      await page.goto(ui.base);
      await page.getByLabel("Webd URL (optional)", { exact: true }).waitFor();
      page.setDefaultTimeout(10000);
      await login(origins[0]);
      await page.getByRole("button", { name: "Enable WeChat and start setup", exact: true }).click();
      if (phase === "current") {
        await waitFor(() => qrRequested);
        assert.equal(requests.filter(row => row.path === "/v1/services/wechatd/start").length, 1);
        assert.equal(requests.filter(row => row.path === "/v1/wechat/login-qr/start").length, 1);
      } else {
        await waitFor(() => held);
        await page.getByRole("button", { name: "Log out", exact: true }).click();
        await login(origins[1]);
        const before = requests.length;
        const completed = page.waitForResponse(response => response.request() === held.request());
        await reply(held, phase === "service" ? {} : { ...config, enabled: true });
        await (await completed).finished(); await page.waitForTimeout(1200);
        const stale = requests.slice(before).filter(row => row.origin === origins[0]);
        assert.deepEqual(stale, [], "a superseded setup continuation must not initiate requests to the former endpoint");
      }
      assert.deepEqual(errors, []);
      await page.screenshot({ path: `${output}/app-${phase}.png` });
      results.push({ phase, passed: true });
    } catch (error) { results.push({ phase, passed: false, error: String(error), errors, requests }); }
    finally { await page.close(); }
  }
  await writeFile(`${output}/report.json`, JSON.stringify({ results, actual_service_mutations: 0, real_provider_calls: 0 }, null, 2));
  console.log(JSON.stringify({ results, actual_service_mutations: 0, real_provider_calls: 0 }));
  assert.deepEqual(results.filter(row => !row.passed), []);
} finally { await browser?.close(); await ui.close(); }
