import assert from "node:assert/strict";
import { createServer } from "node:http";
import { once } from "node:events";
import { fileURLToPath } from "node:url";
import { isolatedUiServer } from "./helpers/isolated-ui-server.mjs";

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || "playwright");
const ui = await isolatedUiServer(fileURLToPath(new URL("../", import.meta.url)));
const csrf = "ab".repeat(16);
const identity = { user_id: 101, chat_id: 202, role: "user", user_key: "fixture-recovery-key" };
const cases = [
  { name: "session_server_failure", path: "/webd/session", status: 503 },
  { name: "profile_server_failure", path: "/v1/auth/me", status: 500 },
  { name: "misleading_session_expiry", path: "/webd/session", status: 503,
    body: { ok: true, error: "webd_session_invalid", data: { logged_in: false } } },
  { name: "misleading_profile_expiry", path: "/v1/auth/me", status: 503,
    body: { ok: false, error: "auth_key_invalid" } },
  { name: "session_invalid_json", path: "/webd/session", invalidJson: true },
  { name: "profile_invalid_json", path: "/v1/auth/me", invalidJson: true },
  { name: "session_transport_failure", path: "/webd/session", disconnect: true },
  { name: "profile_transport_failure", path: "/v1/auth/me", disconnect: true },
  { name: "post_login_profile_failure", path: "/v1/auth/me", status: 503, viaLogin: true },
  { name: "post_login_invalid_json", path: "/v1/auth/me", invalidJson: true, viaLogin: true },
  { name: "post_login_transport_failure", path: "/v1/auth/me", disconnect: true, viaLogin: true },
  { name: "missing_csrf", path: "/webd/session", body: { ok: true, data: { logged_in: true } } },
  { name: "confirmed_signed_out", path: "/webd/session", expires: true,
    body: { ok: true, data: { logged_in: false } } },
  { name: "confirmed_session_expiry", path: "/webd/session", status: 401, expires: true,
    body: { ok: false, error: "webd_session_invalid" } },
  { name: "confirmed_profile_expiry", path: "/v1/auth/me", status: 401, expires: true,
    body: { ok: false, error: "auth_key_invalid" } },
  { name: "post_login_confirmed_expiry", path: "/v1/auth/me", status: 401, expires: true, viaLogin: true,
    body: { ok: false, error: "auth_key_invalid" } },
];
let current;
let browser;
const results = [];
const backend = createServer(async (req, res) => {
  const cors = { "Access-Control-Allow-Origin": ui.base, "Access-Control-Allow-Credentials": "true",
    "Access-Control-Allow-Headers": "content-type,x-agent-key,x-agent-csrf-token",
    "Access-Control-Allow-Methods": "GET,POST,OPTIONS" };
  if (req.method === "OPTIONS") { res.writeHead(204, cors).end(); return; }
  for await (const _part of req) { /* Consume only private fixture requests. */ }
  current.requests.push(req.url);
  if (current.fault && req.url === current.test.path) {
    if (current.test.disconnect) { req.socket.destroy(); return; }
    res.writeHead(current.test.status || 200, { ...cors, "Content-Type": "application/json" });
    res.end(current.test.invalidJson ? "{" : JSON.stringify(current.test.body || { ok: false, error: "auth_lookup_failed" }));
    return;
  }
  let body = { ok: false, error: "fixture_unavailable" };
  if (req.url === "/webd/session") body = { ok: true, data: { logged_in: true, csrf_token: csrf } };
  if (req.url === "/webd/login") body = { ok: true, data: { csrf_token: csrf } };
  if (req.url === "/v1/auth/me") body = { ok: true, data: identity };
  res.writeHead(200, { ...cors, "Content-Type": "application/json" });
  res.end(JSON.stringify(body));
});

try {
  backend.listen(0, "127.0.0.1");
  await once(backend, "listening");
  const base = `http://127.0.0.1:${backend.address().port}`;
  browser = await chromium.launch({ executablePath: process.env.BROWSER_EXECUTABLE || undefined, headless: true });
  for (const test of cases) {
    current = { test, fault: true, requests: [] };
    const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
    await page.addInitScript(({ base, viaLogin }) => {
      if (localStorage.getItem("fixture-recovery-initialized")) return;
      localStorage.setItem("fixture-recovery-initialized", "1");
      localStorage.setItem("agent-runtime.monitor.baseUrl", base);
      localStorage.setItem("agent-runtime.monitor.webdBaseUrl", base);
      if (!viaLogin) localStorage.setItem("agent-runtime.monitor.authMode", "webd");
      localStorage.setItem("agent-runtime.monitor.currentPage", "ai_learning");
      localStorage.setItem("agent-runtime.monitor.pollingSeconds", "0");
    }, { base, viaLogin: Boolean(test.viaLogin) });
    const allowed = new Set([ui.base, base]);
    await page.route("**/*", route => allowed.has(new URL(route.request().url()).origin)
      ? route.continue() : route.abort());
    await page.goto(ui.base);
    const signIn = page.getByRole("button", { name: "进入控制台", exact: true });
    await signIn.waitFor();
    if (test.viaLogin) {
      await page.getByLabel("用户名", { exact: true }).fill("fixture-user");
      await page.getByLabel("密码", { exact: true }).fill("fixture-password");
      await signIn.click();
    }
    await page.locator('p[class~="border-red-500/30"]').waitFor();
    const preference = await page.evaluate(() => localStorage.getItem("agent-runtime.monitor.authMode"));
    assert.equal(preference, test.expires ? null : "webd", test.name);
    await page.waitForTimeout(150);
    const authRequests = () => current.requests.filter(path => ["/webd/session", "/v1/auth/me"].includes(path)).length;
    const requestCount = authRequests();
    await page.waitForTimeout(300);
    assert.equal(authRequests(), requestCount, "a failed restore must not loop automatically");
    assert.equal(await page.getByText("角色: user", { exact: true }).count(), 0,
      "an unavailable or expired identity must not grant console access");
    if (!test.expires) {
      current.fault = false;
      await page.reload();
      await page.getByText("角色: user", { exact: true }).waitFor();
    }
    assert.equal(current.requests.filter(path => path === "/webd/login").length, test.viaLogin ? 1 : 0,
      "recovery must reuse the existing cookie session without an extra password login");
    results.push({ name: test.name, preferenceRetained: preference === "webd", recovered: !test.expires, pass: true });
    await page.close();
  }
  console.log(JSON.stringify({ results, llmRequests: 0 }, null, 2));
} finally {
  await browser?.close();
  await ui.close();
  await new Promise((resolve, reject) => {
    backend.close(error => error ? reject(error) : resolve());
    backend.closeAllConnections();
  });
}
