import assert from "node:assert/strict";
import { createServer as createHttpServer } from "node:http";
import { once } from "node:events";
import { fileURLToPath } from "node:url";
import { isolatedUiServer } from "./helpers/isolated-ui-server.mjs";

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || "playwright");
const root = fileURLToPath(new URL("../", import.meta.url));
const vite = await isolatedUiServer(root);
const password = "fixture-browser-password";
const key = "fixture-browser-key";
const servers = [];
const probes = [];
let browser;
let uiOrigin;

async function server(status, redirectBase) {
  const requests = [];
  const http = createHttpServer(async (req, res) => {
    const cors = { "Access-Control-Allow-Origin": req.headers.origin || uiOrigin,
      "Access-Control-Allow-Credentials": "true",
      "Access-Control-Allow-Headers": "content-type,x-agent-key,x-webd-csrf",
      "Access-Control-Allow-Methods": "GET,POST,OPTIONS" };
    if (req.method === "OPTIONS") { res.writeHead(204, cors).end(); return; }
    let body = "";
    for await (const part of req) body += part;
    requests.push({ path: req.url, body, key: req.headers["x-agent-key"] });
    const original = req.url === "/webd/login" || req.url === "/v1/auth/ui-key/verify";
    const code = original ? status : 200;
    const headers = { ...cors, "Content-Type": "application/json" };
    if (code >= 300 && code < 400) headers.Location = `${redirectBase || base}/destination`;
    let payload = { ok: false, error_code: "invalid_credentials" };
    if (status === 200) {
      if (req.url === "/webd/login") payload = { ok: true, data: { csrf_token: "ab".repeat(16) } };
      else if (["/v1/auth/me", "/v1/auth/ui-key/verify"].includes(req.url)) {
        payload = { ok: true, data: { user_id: 101, chat_id: 202, role: "user", user_key: key } };
      } else if (req.url === "/webd/session") {
        payload = { ok: true, data: { logged_in: true, csrf_token: "ab".repeat(16) } };
      }
    }
    res.writeHead(code, headers).end(JSON.stringify(payload));
  });
  http.listen(0, "127.0.0.1");
  await once(http, "listening");
  const base = `http://127.0.0.1:${http.address().port}`;
  servers.push(http);
  return { base, requests };
}

try {
  uiOrigin = vite.base;
  browser = await chromium.launch({ executablePath: process.env.BROWSER_EXECUTABLE || undefined, headless: true });
  for (const mode of ["password", "key"]) {
    for (const status of [200, 301, 302, 303, 307, 308]) {
      for (const crossOrigin of status === 200 ? [false] : [false, true]) {
        const target = await server(401);
        const origin = await server(status, crossOrigin ? target.base : undefined);
        const context = await browser.newContext();
        await context.addInitScript(() => {
          localStorage.setItem("agent-runtime.monitor.currentPage", "ai_learning");
          localStorage.setItem("agent-runtime.monitor.pollingSeconds", "0");
        });
        const allowed = new Set([uiOrigin, origin.base, target.base]);
        await context.route("**/*", route => allowed.has(new URL(route.request().url()).origin)
          ? route.continue() : route.abort());
        const page = await context.newPage();
        await page.goto(uiOrigin);
        await page.getByRole("button", { name: "进入控制台", exact: true }).waitFor();
        if (mode === "password") {
          await page.getByLabel("Webd 地址（可选）", { exact: true }).fill(origin.base);
          await page.getByLabel("用户名", { exact: true }).fill("fixture-browser-user");
          await page.getByLabel("密码", { exact: true }).fill(password);
        } else {
          await page.getByRole("button", { name: "使用 Key 登录", exact: true }).click();
          await page.getByPlaceholder("http://127.0.0.1:8788", { exact: true }).fill(origin.base);
          await page.getByPlaceholder("输入已经生成好的 user_key", { exact: true }).fill(key);
        }
        await page.getByRole("button", { name: "进入控制台", exact: true }).click();
        if (status === 200) {
          await page.getByRole("button", { name: "退出", exact: true }).waitFor();
          if (mode === "password") await page.getByText("角色: user", { exact: true }).waitFor();
          assert.equal(await page.evaluate(value => JSON.stringify(localStorage).includes(value)
            || JSON.stringify(sessionStorage).includes(value), key), false);
          assert.equal(target.requests.length, 0);
        } else {
          await page.waitForFunction(() => {
            const button = [...document.querySelectorAll("button")].find(node => node.textContent.trim() === "进入控制台");
            return button && !button.disabled;
          });
          const forwarded = [...origin.requests.slice(1), ...target.requests];
          const probe = { mode, status, crossOrigin, originRequests: origin.requests.length,
            targetRequests: target.requests.length,
            forwardedPassword: forwarded.some(req => req.body.includes(password)),
            forwardedKey: forwarded.some(req => req.key === key || req.body.includes(key)) };
          probes.push(probe);
          console.log(JSON.stringify(probe));
        }
        await context.close();
      }
    }
  }
  assert.equal(probes.length, 20);
  assert.ok(probes.every(probe => probe.originRequests === 1 && probe.targetRequests === 0),
    "credentialed UI requests must not follow redirects");
  console.log("PASS: 20 redirect probes and both direct sign-in modes; LLM requests=0");
} finally {
  await browser?.close();
  await vite.close();
  for (const server of servers) {
    await new Promise((resolve, reject) => {
      server.close(error => error ? reject(error) : resolve());
      server.closeAllConnections();
    });
  }
}
