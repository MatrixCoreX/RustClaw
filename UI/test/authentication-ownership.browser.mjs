import assert from "node:assert/strict";
import { createServer as createHttpServer } from "node:http";
import { once } from "node:events";
import { mkdir } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { isolatedUiServer } from "./helpers/isolated-ui-server.mjs";

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || "playwright");
const root = fileURLToPath(new URL("../", import.meta.url));
const output = process.env.UI_BROWSER_ARTIFACTS || "/tmp/authentication-ownership-browser-tests";
await mkdir(output, { recursive: true });
const vite = await isolatedUiServer(root);
let browser;
let heldProfile;
let loginCount = 0;
let profileCount = 0;
let heldProfileNumber = 1;
const identity = role => ({ user_id: role === "admin" ? 101 : 202,
  chat_id: role === "admin" ? 301 : 402, role, user_key: `fixture-${role}-key` });
const backend = createHttpServer(async (req, res) => {
  const cors = { "Access-Control-Allow-Origin": req.headers.origin || "null",
    "Access-Control-Allow-Credentials": "true", "Access-Control-Allow-Headers": "content-type,x-webd-csrf",
    "Access-Control-Allow-Methods": "GET,POST,OPTIONS" };
  if (req.method === "OPTIONS") { res.writeHead(204, cors).end(); return; }
  for await (const _part of req) { /* Consume only owned fixture requests. */ }
  res.writeHead(200, { ...cors, "Content-Type": "application/json",
    "X-Fixture-Profile-Number": String(req.url === "/v1/auth/me" ? profileCount + 1 : 0) });
  let payload = { ok: false, error: "fixture_unavailable" };
  if (req.url === "/webd/login") {
    loginCount++;
    payload = { ok: true, data: { csrf_token: "ab".repeat(16) } };
  } else if (req.url === "/webd/session") {
    payload = { ok: true, data: { logged_in: true, csrf_token: "ab".repeat(16) } };
  } else if (req.url === "/webd/logout") {
    payload = { ok: true };
  } else if (req.url === "/v1/auth/me") {
    profileCount++;
    if (profileCount === heldProfileNumber) {
      heldProfile = res;
      res.flushHeaders();
      return;
    }
    payload = { ok: true, data: identity(loginCount === 1 ? "admin" : "user") };
  }
  res.end(JSON.stringify(payload));
});

try {
  backend.listen(0, "127.0.0.1");
  await once(backend, "listening");
  const backendBase = `http://127.0.0.1:${backend.address().port}`;
  const uiBase = vite.base;
  browser = await chromium.launch({ executablePath: process.env.BROWSER_EXECUTABLE || undefined, headless: true });
  for (const delayedProfile of [1, 2]) {
    loginCount = 0;
    profileCount = 0;
    heldProfileNumber = delayedProfile;
    const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
    await page.addInitScript(() => {
      localStorage.setItem("agent-runtime.monitor.currentPage", "ai_learning");
      localStorage.setItem("agent-runtime.monitor.pollingSeconds", "0");
    });
    const allowed = new Set([uiBase, backendBase]);
    await page.route("**/*", route => allowed.has(new URL(route.request().url()).origin)
      ? route.continue() : route.abort());
    await page.goto(uiBase);
    await page.getByLabel("Webd 地址（可选）", { exact: true }).fill(backendBase);
    await page.getByLabel("用户名", { exact: true }).fill("fixture-old-user");
    await page.getByLabel("密码", { exact: true }).fill("fixture-old-password");
    const firstProfile = page.waitForResponse(response => new URL(response.url()).pathname === "/v1/auth/me"
      && response.headers()["x-fixture-profile-number"] === String(delayedProfile));
    await page.getByRole("button", { name: "进入控制台", exact: true }).click();
    const oldResponse = await firstProfile;
    await page.getByRole("button", { name: "退出", exact: true }).click();
    await page.getByLabel("用户名", { exact: true }).fill("fixture-new-user");
    await page.getByLabel("密码", { exact: true }).fill("fixture-new-password");
    await page.getByRole("button", { name: "进入控制台", exact: true }).click();
    await page.getByText("角色: user", { exact: true }).waitFor();
    await page.evaluate(() => {
      window.fixtureIdentityRoles = [];
      const record = () => {
        for (const element of document.querySelectorAll("p")) {
          if (/^角色:/.test(element.textContent.trim())) window.fixtureIdentityRoles.push(element.textContent.trim());
        }
      };
      new MutationObserver(record).observe(document.body, { childList: true, subtree: true, characterData: true });
      record();
    });
    heldProfile.end(JSON.stringify({ ok: true, data: identity("admin") }));
    heldProfile = null;
    await oldResponse.finished();
    await page.waitForTimeout(300);
    const roles = await page.evaluate(() => window.fixtureIdentityRoles);
    console.log(JSON.stringify({ delayedProfile, loginCount, profileCount, roles, llmRequests: 0 }));
    await page.screenshot({ path: `${output}/after-old-profile-${delayedProfile}-body.png` });
    assert.equal(roles.some(role => role.includes("admin")), false,
      "an old response body must never replace the new authenticated identity");
    await page.getByText("角色: user", { exact: true }).waitFor();
    await page.close();
  }
} finally {
  heldProfile?.end(JSON.stringify({ ok: true, data: identity("admin") }));
  await browser?.close();
  await vite.close();
  await new Promise((resolve, reject) => {
    backend.close(error => error ? reject(error) : resolve());
    backend.closeAllConnections();
  });
}
