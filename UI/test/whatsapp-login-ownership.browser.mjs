import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { isolatedUiServer } from "./helpers/isolated-ui-server.mjs";
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || "playwright");
const output = process.env.UI_BROWSER_ARTIFACTS || "/tmp/whatsapp-login-ownership";
await mkdir(output, { recursive: true });
const ui = await isolatedUiServer(fileURLToPath(new URL("../", import.meta.url)));
const origins = ["https://device-a.example.test", "https://device-b.example.test"];
const results = [];
let browser;
const snapshot = label => ({ connected: false, qr_ready: true, qr_data_url: label, phase: "qr_ready" });
try {
  browser = await chromium.launch({ executablePath: process.env.BROWSER_EXECUTABLE || undefined, headless: true });
  for (const width of [1440, 390]) for (const theme of ["light", "dark"]) {
    for (const scenario of ["late-status", "scope-clear", "late-logout", "settle-delay", "duplicate-logout", "read-fence", "stale-setter", "ordinary-user", "secondary-failure", "foreground-promotion", "role", "logout"]) {
      const page = await browser.newPage({ viewport: { width, height: width === 390 ? 844 : 1000 } });
      const requests = [], held = [], errors = [];
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
      }, body: JSON.stringify({ ok: status === 200, data, error: status === 200 ? null : "whatsapp_web.login_status_unavailable" }) });
      const scope = async detail => {
        const before = JSON.stringify((await state()).scope);
        await page.evaluate(value => window.dispatchEvent(new CustomEvent("fixture-scope", { detail: value })), detail);
        await wait(async () => JSON.stringify((await state()).scope) !== before);
      };
      const read = () => page.locator("[data-read]").click();
      const logout = () => page.locator("[data-logout]").click();
      const release = async (index, data, status = 200) => {
        const route = held[index], response = page.waitForResponse(value => value.request() === route.request());
        await reply(route, data, status); await (await response).finished(); await page.waitForTimeout(75);
      };
      page.on("pageerror", error => errors.push(error.message));
      await page.route("**/*", async route => {
        const request = route.request(), url = new URL(request.url());
        if (url.origin === ui.base) return route.continue();
        if (!origins.includes(url.origin)) return route.abort();
        if (request.method() === "OPTIONS") return reply(route, null);
        const kind = request.method() === "POST" ? "logout" : "status";
        requests.push({ origin: url.origin, path: url.pathname, method: request.method(), body: request.postDataJSON() });
        if (kind === hold) { held.push(route); return; }
        return reply(route, kind === "logout" ? {} : failStatus ? null : snapshot(url.hostname), kind === "status" && failStatus ? 503 : 200);
      });
      try {
        await page.goto(`${ui.base}/test/fixtures/whatsapp-login-ownership.html?theme=${theme}`);
        await page.locator("#state").waitFor(); page.setDefaultTimeout(5000);
        if (scenario === "late-status") {
          hold = "status"; await read(); await wait(() => held.length === 1);
          await scope("device"); hold = ""; await read();
          await wait(async () => (await state()).status?.qr_data_url === "device-b.example.test");
          await release(0, snapshot("obsolete"));
          assert.equal((await state()).status.qr_data_url, "device-b.example.test");
        } else if (scenario === "scope-clear") {
          await page.locator("[data-open]").click(); await wait(async () => (await state()).reachable);
          await scope("device");
          assert.equal((await state()).status, null); assert.equal((await state()).panel, false); assert.equal((await state()).reachable, false);
        } else if (["late-logout", "role", "logout"].includes(scenario)) {
          hold = "logout"; await logout(); await wait(() => held.length === 1);
          await scope(scenario === "late-logout" ? "device" : scenario);
          await release(0, {}); await page.waitForTimeout(1300);
          assert.equal(requests.length, 1); assert.deepEqual((await state()).notices, []); assert.equal((await state()).loggingOut, false);
        } else if (scenario === "settle-delay") {
          await logout(); await wait(() => requests.length === 1);
          await scope("device"); await page.waitForTimeout(1300);
          assert.equal(requests.length, 1); assert.deepEqual((await state()).notices, []);
        } else if (["duplicate-logout", "ordinary-user", "secondary-failure"].includes(scenario)) {
          if (scenario === "ordinary-user") await scope("role");
          hold = "logout"; await logout(); await wait(() => held.length === 1);
          if (scenario === "duplicate-logout") await logout();
          assert.equal(requests.length, 1); assert.equal(requests[0].body, null);
          failStatus = scenario === "secondary-failure"; hold = "";
          await release(0, {}); await wait(async () => !(await state()).loggingOut);
          assert.equal(requests.filter(row => row.method === "POST").length, 1);
          assert.equal((await state()).notices[0].notice.tone, "success");
          if (failStatus) assert.ok((await state()).error);
        } else if (scenario === "read-fence") {
          hold = "status"; await read(); await wait(() => held.length === 1);
          hold = ""; await logout(); await wait(async () => !(await state()).loggingOut);
          await release(0, snapshot("obsolete"));
          assert.equal((await state()).status.qr_data_url, "device-a.example.test");
        } else if (scenario === "stale-setter") {
          await scope("device"); await page.locator("[data-old-open]").click();
          assert.equal((await state()).panel, false); assert.equal(requests.length, 0);
          await page.locator("[data-open]").click(); await wait(async () => (await state()).reachable);
          await page.locator("[data-old-close]").click(); assert.equal((await state()).panel, true);
          await page.locator("[data-toggle]").click(); assert.equal((await state()).panel, false);
        } else {
          hold = "status"; await page.locator("[data-silent]").click(); await wait(() => held.length === 1); await read();
          assert.equal((await state()).loading, true); assert.equal(requests.length, 1);
          await release(0, null, 503); assert.ok((await state()).error); assert.equal((await state()).loading, false);
        }
        assert.deepEqual(errors, []);
        results.push({ width, theme, scenario, passed: true });
      } catch (error) {
        results.push({ width, theme, scenario, passed: false, error: String(error), errors, requests });
        await page.screenshot({ path: `${output}/failed-${width}-${theme}-${scenario}.png` });
      } finally { await page.close(); }
    }
  }
  await writeFile(`${output}/report.json`, JSON.stringify({ results, real_provider_calls: 0, actual_signouts: 0 }, null, 2));
  console.log(JSON.stringify({ cases: results.length, passed: results.filter(row => row.passed).length, failed: results.filter(row => !row.passed) }));
  assert.deepEqual(results.filter(row => !row.passed), []);
} finally { await browser?.close(); await ui.close(); }
