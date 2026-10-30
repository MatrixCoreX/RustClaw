import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { isolatedUiServer } from "./helpers/isolated-ui-server.mjs";
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || "playwright");
const output = process.env.UI_BROWSER_ARTIFACTS || "/tmp/wechat-login-ownership";
await mkdir(output, { recursive: true });
const ui = await isolatedUiServer(fileURLToPath(new URL("../", import.meta.url)));
const origins = ["https://device-a.example.test", "https://device-b.example.test"];
const results = [];
let browser;
const normal = label => ({ connected: false, qr_ready: false, account_label: label });
const qr = label => ({ session_key: label, qrcode_url: `https://qr.example.test/${label}` });
try {
  browser = await chromium.launch({ executablePath: process.env.BROWSER_EXECUTABLE || undefined, headless: true });
  for (const width of [1440, 390]) for (const theme of ["light", "dark"]) {
    for (const scenario of ["late-status", "late-start", "scope-clear", "duplicate-start", "status-fence", "role", "epoch", "logout", "old-connected-poll", "confirmed-refresh-failure", "ordinary-user"]) {
      const page = await browser.newPage({ viewport: { width, height: width === 390 ? 844 : 1000 } });
      const requests = [], held = [], errors = [], sessions = new Map();
      let hold = "", failStatus = false;
      const state = async () => JSON.parse(await page.locator("#state").innerText());
      const wait = async predicate => {
        const deadline = Date.now() + 5000;
        while (!(await predicate())) {
          if (Date.now() >= deadline) throw new Error("fixture_state_timeout");
          await page.waitForTimeout(10);
        }
      };
      const reply = (route, data, status = 200) => route.fulfill({ status, contentType: "application/json", headers: {
        "Access-Control-Allow-Origin": ui.base, "Access-Control-Allow-Headers": "content-type", "Access-Control-Allow-Methods": "GET,POST,OPTIONS",
      }, body: JSON.stringify({ ok: status === 200, data, error: status === 200 ? null : "fixture_unavailable" }) });
      const event = (name, detail) => page.evaluate(value => window.dispatchEvent(new CustomEvent(value.name, { detail: value.detail })), { name, detail });
      const scope = async detail => {
        const before = JSON.stringify((await state()).scope);
        await event("fixture-scope", detail);
        await wait(async () => JSON.stringify((await state()).scope) !== before);
      };
      const read = () => page.locator("[data-read]").click();
      const start = async () => {
        await page.locator('[data-start="normal"]').click();
        if (hold !== "start") await wait(async () => (await state()).status?.qr_ready === true);
      };
      const release = async (index, data) => {
        const route = held[index], response = page.waitForResponse(value => value.request() === route.request());
        await reply(route, data); await (await response).finished(); await page.waitForTimeout(75);
      };
      page.on("pageerror", error => errors.push(error.message));
      await page.route("**/*", async route => {
        const request = route.request(), url = new URL(request.url());
        if (url.origin === ui.base) return route.continue();
        if (!origins.includes(url.origin)) return route.abort();
        if (request.method() === "OPTIONS") return reply(route, null);
        requests.push({ origin: url.origin, path: url.pathname, method: request.method(), body: request.postDataJSON() });
        const kind = url.pathname.endsWith("/start") ? "start" : url.pathname.endsWith("/wait") ? "wait" : "status";
        if (kind === hold) { held.push(route); return; }
        if (kind === "start") { sessions.set(url.origin, url.hostname); return reply(route, qr(url.hostname)); }
        if (kind === "wait") return reply(route, { connected: false, qr_status: "wait" });
        if (failStatus) return reply(route, null, 503);
        const session = sessions.get(url.origin);
        return reply(route, { ...normal(url.hostname), ...(session ? { qr_ready: true, session_key: session, qrcode_url: qr(session).qrcode_url } : {}) });
      });
      try {
        await page.goto(`${ui.base}/test/fixtures/wechat-login-ownership.html?theme=${theme}`);
        await page.locator("#state").waitFor(); page.setDefaultTimeout(5000);
        if (scenario === "late-status") {
          hold = "status"; await read(); await wait(() => held.length === 1);
          await scope("device"); hold = ""; await read();
          await wait(async () => (await state()).status?.account_label === "device-b.example.test");
          await release(0, normal("obsolete-a"));
          assert.equal((await state()).status.account_label, "device-b.example.test");
        } else if (scenario === "late-start") {
          hold = "start"; await start(); await wait(() => held.length === 1);
          await scope("device"); hold = ""; await start();
          await release(0, qr("obsolete-a"));
          assert.equal((await state()).status.qrcode_url, qr("device-b.example.test").qrcode_url);
        } else if (scenario === "scope-clear") {
          await start(); await scope("device");
          assert.equal((await state()).status, null);
          assert.equal((await state()).preview, false);
        } else if (scenario === "duplicate-start") {
          hold = "start"; await start(); await wait(() => held.length === 1);
          await page.locator('[data-start="force"]').click();
          assert.equal(requests.filter(row => row.path.endsWith("/start")).length, 1);
          assert.deepEqual(requests[0].body, { force: false });
          await release(0, qr("current"));
          assert.equal((await state()).starting, false);
        } else if (scenario === "status-fence") {
          hold = "status"; await read(); await wait(() => held.length === 1);
          hold = ""; await start(); await release(0, normal("obsolete"));
          assert.equal((await state()).status.qrcode_url, qr("device-a.example.test").qrcode_url);
        } else if (["role", "epoch", "logout"].includes(scenario)) {
          hold = "start"; await start(); await wait(() => held.length === 1);
          await scope(scenario); await release(0, qr("obsolete"));
          assert.equal((await state()).status, null);
          assert.equal((await state()).starting, false);
        } else if (scenario === "ordinary-user") {
          await scope("role"); await start();
          assert.equal((await state()).status.qr_ready, true);
          assert.equal(requests.filter(row => row.path.endsWith("/start")).length, 1);
        } else {
          await start(); hold = "wait"; await event("fixture-visibility", "visible"); await wait(() => held.length === 1);
          if (scenario === "old-connected-poll") {
            await event("fixture-visibility", "hidden"); await scope("device");
            const before = requests.length;
            await release(0, { connected: true, current_user_bound: true, binding_status: "bound" });
            assert.equal(requests.length, before);
            assert.equal((await state()).status, null);
          } else {
            failStatus = true;
            await release(0, { connected: true, current_user_bound: true, provider_connected: true, binding_status: "bound" });
            await wait(() => requests.filter(row => row.path.endsWith("/login-status")).length >= 2);
            assert.equal((await state()).status.connected, true);
            assert.equal((await state()).status.current_user_bound, true);
            assert.equal((await state()).status.qrcode_url, null);
            assert.equal((await state()).error, null);
          }
        }
        assert.deepEqual(errors, []);
        results.push({ width, theme, scenario, passed: true });
      } catch (error) {
        results.push({ width, theme, scenario, passed: false, error: String(error), errors, requests });
        await page.screenshot({ path: `${output}/failed-${width}-${theme}-${scenario}.png` });
      } finally { await page.close(); }
    }
  }
  await writeFile(`${output}/report.json`, JSON.stringify({ results, real_provider_calls: 0, actual_logins: 0, actual_service_mutations: 0 }, null, 2));
  console.log(JSON.stringify({ cases: results.length, passed: results.filter(row => row.passed).length, failed: results.filter(row => !row.passed) }));
  assert.deepEqual(results.filter(row => !row.passed), []);
} finally { await browser?.close(); await ui.close(); }
