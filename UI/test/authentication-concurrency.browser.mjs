import assert from "node:assert/strict";
import { createServer } from "node:http";
import { once } from "node:events";
import { fileURLToPath } from "node:url";
import { isolatedUiServer } from "./helpers/isolated-ui-server.mjs";

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || "playwright");
const ui = await isolatedUiServer(fileURLToPath(new URL("../", import.meta.url)));
const fixtureKey = "fixture-concurrent-auth-key";
const backends = [];
const results = [];
let browser;

async function backend(mode) {
  const state = { logins: 0, logouts: 0, requests: [], held: null };
  const server = createServer(async (req, res) => {
    const cors = { "Access-Control-Allow-Origin": req.headers.origin || ui.base,
      "Access-Control-Allow-Credentials": "true", "Access-Control-Allow-Headers": "content-type,x-agent-key,x-webd-csrf",
      "Access-Control-Allow-Methods": "GET,POST,OPTIONS" };
    if (req.method === "OPTIONS") { res.writeHead(204, cors).end(); return; }
    for await (const _part of req) { /* Only owned fixture input is consumed. */ }
    state.requests.push({ path: req.url, key: req.headers["x-agent-key"] });
    res.writeHead(200, { ...cors, "Content-Type": "application/json" });
    let payload = { ok: false, error: "fixture_unavailable" };
    let hold = false;
    if (req.url === "/webd/login") {
      state.logins++;
      payload = { ok: true, data: { csrf_token: "ab".repeat(16) } };
      hold = ["enter", "endpoint-password"].includes(mode) && state.logins === 1;
    } else if (req.url === "/webd/logout") {
      state.logouts++;
      payload = { ok: true };
      hold = mode === "logout" && state.logouts === 1;
    } else if (["/v1/auth/me", "/v1/auth/ui-key/verify"].includes(req.url)) {
      payload = { ok: true, data: { user_id: 101, chat_id: 202, role: "user", user_key: fixtureKey } };
      hold = mode === "endpoint" && req.url === "/v1/auth/ui-key/verify";
    } else if (req.url === "/webd/session") {
      payload = { ok: true, data: { logged_in: true, csrf_token: "ab".repeat(16) } };
    }
    if (hold) {
      state.held = { res, payload };
      if (mode !== "logout") res.flushHeaders();
    } else res.end(JSON.stringify(payload));
  });
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  const fixture = { base: `http://127.0.0.1:${server.address().port}`, state,
    release() {
      if (state.held) { state.held.res.end(JSON.stringify(state.held.payload)); state.held = null; }
    },
    async close() {
      fixture.release();
      await new Promise((resolve, reject) => {
        server.close(error => error ? reject(error) : resolve()); server.closeAllConnections();
      });
    } };
  backends.push(fixture);
  return fixture;
}

async function waitFor(predicate) {
  const deadline = Date.now() + 5000;
  while (!predicate()) {
    if (Date.now() >= deadline) throw new Error("owned fixture did not receive the expected request");
    await new Promise(resolve => setTimeout(resolve, 10));
  }
}

async function pageFor(...origins) {
  const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
  await page.addInitScript(() => {
    localStorage.setItem("agent-runtime.monitor.currentPage", "ai_learning");
    localStorage.setItem("agent-runtime.monitor.pollingSeconds", "0");
  });
  const allowed = new Set([ui.base, ...origins]);
  await page.route("**/*", route => allowed.has(new URL(route.request().url()).origin)
    ? route.continue() : route.abort());
  await page.goto(ui.base);
  return page;
}

async function fillLogin(page, base) {
  await page.getByLabel("Webd 地址（可选）", { exact: true }).fill(base);
  await page.getByLabel("用户名", { exact: true }).fill("fixture-user");
  await page.getByLabel("密码", { exact: true }).fill("fixture-password");
}

try {
  browser = await chromium.launch({ executablePath: process.env.BROWSER_EXECUTABLE || undefined, headless: true });
  {
    const origin = await backend("endpoint"), target = await backend("target");
    const page = await pageFor(origin.base, target.base);
    await page.getByRole("button", { name: "使用 Key 登录", exact: true }).click();
    const endpoint = page.getByPlaceholder("http://127.0.0.1:8788", { exact: true });
    await endpoint.fill(origin.base);
    await page.getByPlaceholder("输入已经生成好的 user_key", { exact: true }).fill(fixtureKey);
    const response = page.waitForResponse(res => new URL(res.url()).pathname === "/v1/auth/ui-key/verify");
    await page.getByRole("button", { name: "进入控制台", exact: true }).click();
    const pending = await response;
    await endpoint.fill(target.base);
    origin.release();
    await pending.finished();
    await page.waitForTimeout(300);
    const leakedRequests = target.state.requests.filter(req => req.key === fixtureKey).length;
    const staysSignedOut = await page.getByRole("button", { name: "进入控制台", exact: true }).count() === 1;
    let canSignInAgain = false;
    if (staysSignedOut) {
      await page.getByPlaceholder("输入已经生成好的 user_key", { exact: true }).fill("fixture-new-endpoint-key");
      await page.getByRole("button", { name: "进入控制台", exact: true }).click();
      await page.getByRole("button", { name: "退出", exact: true }).waitFor();
      canSignInAgain = true;
    }
    results.push({ name: "endpoint_change_during_key_verification", leakedRequests, staysSignedOut,
      canSignInAgain, pass: leakedRequests === 0 && staysSignedOut && canSignInAgain });
    await page.close();
  }
  {
    const origin = await backend("endpoint-password"), target = await backend("target");
    const page = await pageFor(origin.base, target.base);
    await fillLogin(page, origin.base);
    const response = page.waitForResponse(res => new URL(res.url()).pathname === "/webd/login");
    await page.getByRole("button", { name: "进入控制台", exact: true }).click();
    const pending = await response;
    await page.getByLabel("Webd 地址（可选）", { exact: true }).fill(target.base);
    origin.release();
    await pending.finished();
    await page.waitForTimeout(300);
    const staysSignedOut = await page.getByRole("button", { name: "进入控制台", exact: true }).count() === 1;
    let canSignInAgain = false;
    if (staysSignedOut) {
      await fillLogin(page, target.base);
      await page.getByRole("button", { name: "进入控制台", exact: true }).click();
      await page.getByText("角色: user", { exact: true }).waitFor();
      canSignInAgain = true;
    }
    results.push({ name: "endpoint_change_during_password_login", staysSignedOut, canSignInAgain,
      pass: staysSignedOut && canSignInAgain });
    await page.close();
  }
  {
    const origin = await backend("enter");
    const page = await pageFor(origin.base);
    await fillLogin(page, origin.base);
    const response = page.waitForResponse(res => new URL(res.url()).pathname === "/webd/login");
    await page.getByLabel("密码", { exact: true }).press("Enter");
    const pending = await response;
    await page.keyboard.press("Enter");
    await page.waitForTimeout(150);
    const loginRequests = origin.state.logins;
    origin.release();
    await pending.finished();
    await page.getByRole("button", { name: "退出", exact: true }).waitFor();
    results.push({ name: "repeated_enter_during_login", loginRequests, pass: loginRequests === 1 });
    await page.close();
  }
  {
    const origin = await backend("logout");
    const page = await pageFor(origin.base);
    await fillLogin(page, origin.base);
    await page.getByRole("button", { name: "进入控制台", exact: true }).click();
    await page.getByRole("button", { name: "退出", exact: true }).click();
    await waitFor(() => origin.state.held !== null);
    await page.getByRole("button", { name: "退出", exact: true }).click();
    await page.waitForTimeout(150);
    const logoutRequests = origin.state.logouts;
    if (logoutRequests === 1) origin.release();
    await fillLogin(page, origin.base);
    await page.getByRole("button", { name: "进入控制台", exact: true }).click();
    await page.getByRole("button", { name: "退出", exact: true }).waitFor();
    origin.release();
    await page.waitForTimeout(300);
    const staysSignedIn = await page.getByRole("button", { name: "退出", exact: true }).count() === 1;
    results.push({ name: "overlapping_logout_and_new_login", logoutRequests, staysSignedIn,
      pass: logoutRequests === 1 && staysSignedIn });
    await page.close();
  }
  console.log(JSON.stringify({ results, llmRequests: 0 }, null, 2));
  assert.ok(results.every(result => result.pass), "authentication attempts must retain their request ownership");
} finally {
  for (const fixture of backends) fixture.release();
  await browser?.close();
  await ui.close();
  for (const fixture of backends) await fixture.close();
}
