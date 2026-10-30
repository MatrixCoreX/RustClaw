import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { isolatedUiServer } from "./helpers/isolated-ui-server.mjs";
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || "playwright");
const output = process.env.UI_BROWSER_ARTIFACTS || "/tmp/channel-bind-ownership";
await mkdir(output, { recursive: true });
const ui = await isolatedUiServer(fileURLToPath(new URL("../", import.meta.url)));
const origins = ["https://device-a.example.test", "https://device-b.example.test"];
const results = [];
let browser;
try {
  browser = await chromium.launch({ executablePath: process.env.BROWSER_EXECUTABLE || undefined, headless: true });
  for (const platform of ["feishu", "lark"]) for (const width of [1440, 390]) for (const theme of ["light", "dark"]) {
    for (const scenario of ["current-reset", "cancel-reset", "duplicate-reset", "begin-conflict", "reset-conflict", "reset-failure", "secondary-failure", "late-begin", "late-poll", "late-reset", "stale-confirm-device", "stale-confirm-epoch", "stale-confirm-role", "stale-confirm-logout", "stale-confirm-platform", "ordinary-user", "stale-callbacks"]) {
      const page = await browser.newPage({ viewport: { width, height: width === 390 ? 844 : 1000 } });
      const requests = [], held = [], errors = [];
      let hold = "", failRefresh = false, failReset = false, id = 0;
      const session = (channel = platform, sessionId = 1, status = "pending") => ({ session_id: sessionId, channel, status,
        bind_token: "fixture-only", created_at: "1793361600", updated_at: "1793361600", expires_at: "1793362200",
        entry_url: `https://bind.example.test/${channel}/${sessionId}`, poll_interval_seconds: 1 });
      const state = async () => JSON.parse(await page.locator("#state").innerText());
      const wait = async predicate => {
        const deadline = Date.now() + 5000;
        while (!(await predicate())) { if (Date.now() >= deadline) throw new Error("fixture_state_timeout"); await page.waitForTimeout(10); }
      };
      const reply = (route, data, status = 200) => route.fulfill({ status, contentType: "application/json", headers: {
        "Access-Control-Allow-Origin": ui.base, "Access-Control-Allow-Headers": "content-type", "Access-Control-Allow-Methods": "GET,POST,OPTIONS",
      }, body: JSON.stringify({ ok: status === 200, data, error: status === 200 ? null : "fixture_unavailable" }) });
      const scope = async detail => {
        const before = JSON.stringify((await state()).scope);
        await page.evaluate(value => window.dispatchEvent(new CustomEvent("fixture-scope", { detail: value })), detail);
        await wait(async () => JSON.stringify((await state()).scope) !== before);
      };
      const begin = () => page.locator("[data-begin]").click();
      const reset = () => page.locator("[data-reset]").click();
      const confirm = () => page.getByRole("alertdialog").getByRole("button", { name: "Reset", exact: true }).click();
      const release = async (index, data, status = 200) => {
        const route = held[index], response = page.waitForResponse(value => value.request() === route.request());
        await reply(route, data, status); await (await response).finished(); await page.waitForTimeout(50);
      };
      const mutations = () => requests.filter(row => row.method === "POST");
      const refreshes = () => requests.filter(row => row.path.startsWith("/fixture/"));
      page.on("pageerror", error => errors.push(error.message));
      await page.route("**/*", async route => {
        const request = route.request(), url = new URL(request.url());
        if (url.origin === ui.base) return route.continue();
        if (!origins.includes(url.origin)) return route.abort();
        if (request.method() === "OPTIONS") return reply(route, null);
        const kind = url.pathname.endsWith("/start") ? "begin" : url.pathname.endsWith("/reset") ? "reset" : url.pathname.startsWith("/fixture/") ? "refresh" : "poll";
        requests.push({ origin: url.origin, path: url.pathname, method: request.method(), body: request.postDataJSON() });
        if (kind === hold) { held.push(route); return; }
        if (kind === "refresh") return reply(route, {}, failRefresh ? 503 : 200);
        if (kind === "reset") return reply(route, {}, failReset ? 503 : 200);
        return reply(route, session(platform, kind === "begin" ? ++id : id));
      });
      try {
        await page.goto(`${ui.base}/test/fixtures/channel-bind-ownership.html?platform=${platform}&theme=${theme}`);
        await page.locator("#state").waitFor(); page.setDefaultTimeout(5000);
        if (scenario === "ordinary-user") {
          await scope("role"); await begin(); await reset();
          assert.equal(requests.length, 0); assert.equal(await page.getByRole("alertdialog").count(), 0);
        } else if (scenario === "stale-callbacks") {
          await scope("device"); await scope("return");
          await page.locator("[data-old-begin]").click(); await page.locator("[data-old-reset]").click();
          assert.equal(requests.length, 0); assert.equal(await page.getByRole("alertdialog").count(), 0);
        } else if (scenario === "late-begin") {
          hold = "begin"; await begin(); await wait(() => held.length === 1);
          await scope("device"); await release(0, session());
          assert.equal((await state()).session, null); assert.equal((await state()).qr, null);
        } else if (scenario === "begin-conflict") {
          hold = "begin"; await begin(); await wait(() => held.length === 1); await begin(); await reset();
          assert.equal(mutations().length, 1); assert.equal(await page.getByRole("alertdialog").count(), 0);
          await release(0, session()); await wait(async () => (await state()).qr);
        } else {
          await begin(); await wait(async () => (await state()).qr);
          assert.equal(await page.getByAltText("Binding QR").evaluate(node => node.naturalWidth), 288);
          if (scenario === "late-poll") {
            hold = "poll"; await wait(() => held.length === 1); await scope("device"); await release(0, session(platform, 1, "bound"));
            assert.equal((await state()).session, null); assert.deepEqual(refreshes(), []);
          } else {
            await reset(); await page.getByRole("alertdialog").waitFor();
            if (scenario === "duplicate-reset") {
              await page.locator("[data-reset]").evaluate(node => node.click());
              assert.equal(await page.getByRole("alertdialog").count(), 1);
            }
            if (scenario.startsWith("stale-confirm-")) {
              await scope(scenario.slice("stale-confirm-".length)); await confirm(); await page.waitForTimeout(75);
              assert.equal(mutations().length, 1); assert.deepEqual(refreshes(), []);
              assert.equal((await state()).session, null); assert.equal((await state()).resetting, false);
            } else if (scenario === "cancel-reset") {
              await page.getByRole("alertdialog").getByRole("button", { name: "Cancel", exact: true }).filter({ hasText: "Cancel" }).click();
              assert.equal(mutations().length, 1); assert.equal((await state()).session.session_id, 1);
            } else {
              if (scenario === "reset-conflict") {
                await page.locator("[data-begin]").evaluate(node => node.click()); assert.equal(mutations().length, 1);
              }
              failRefresh = scenario === "secondary-failure"; failReset = scenario === "reset-failure";
              if (["late-reset", "duplicate-reset", "reset-conflict"].includes(scenario)) hold = "reset";
              await confirm();
              if (hold === "reset") {
                await wait(() => held.length === 1);
                if (scenario === "late-reset") await scope("device");
                else if (scenario === "reset-conflict") await begin();
                await release(0, {});
              }
              await wait(async () => !(await state()).resetting);
              const resets = mutations().filter(row => row.path === `/v1/admin/${platform}/reset`);
              assert.equal(resets.length, 1); assert.equal(resets[0].body, null);
              if (scenario === "late-reset") { assert.deepEqual(refreshes(), []); assert.equal((await state()).error, null); }
              else if (scenario === "reset-failure") {
                assert.equal((await state()).session.session_id, 1); assert.ok((await state()).error);
                const count = requests.length; await wait(() => requests.length > count);
                assert.equal(mutations().length, 2); assert.deepEqual(refreshes(), []);
              } else {
                assert.equal((await state()).session, null); assert.equal((await state()).qr, null); assert.equal((await state()).error, null);
                assert.deepEqual(refreshes().map(row => row.path), [`/fixture/${platform}/config`, `/fixture/${platform}/health`]);
                if (scenario === "duplicate-reset") { assert.equal(await page.getByRole("alertdialog").count(), 0); assert.equal(mutations().length, 2); }
              }
            }
          }
        }
        const starts = mutations().filter(row => row.path.endsWith("/start"));
        for (const request of starts) assert.deepEqual(request.body, {});
        assert.deepEqual(errors, []);
        if (scenario === "current-reset") await page.screenshot({ path: `${output}/${platform}-${width}-${theme}.png` });
        results.push({ platform, width, theme, scenario, passed: true });
      } catch (error) {
        results.push({ platform, width, theme, scenario, passed: false, error: String(error), errors, requests });
        await page.screenshot({ path: `${output}/failed-${platform}-${width}-${theme}-${scenario}.png` });
      } finally { await page.close(); }
    }
  }
  await writeFile(`${output}/report.json`, JSON.stringify({ results, real_provider_calls: 0, actual_bindings: 0, actual_resets: 0 }, null, 2));
  console.log(JSON.stringify({ cases: results.length, passed: results.filter(row => row.passed).length, failed: results.filter(row => !row.passed) }));
  assert.deepEqual(results.filter(row => !row.passed), []);
} finally { await browser?.close(); await ui.close(); }
