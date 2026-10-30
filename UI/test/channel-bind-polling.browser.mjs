import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { isolatedUiServer } from "./helpers/isolated-ui-server.mjs";

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || "playwright");
const output = process.env.UI_BROWSER_ARTIFACTS || "/tmp/channel-bind-polling";
await mkdir(output, { recursive: true });
const ui = await isolatedUiServer(fileURLToPath(new URL("../", import.meta.url)));
const results = [];
let browser;
try {
  browser = await chromium.launch({ executablePath: process.env.BROWSER_EXECUTABLE || undefined, headless: true });
  for (const platform of ["feishu", "lark"]) for (const width of [1440, 390]) for (const theme of ["light", "dark"]) {
    for (const scenario of ["overflow", "extreme", "text", "normal", "refreshed-overflow", "scope", "terminal"]) {
      const page = await browser.newPage({ viewport: { width, height: width === 390 ? 844 : 1000 } });
      const requests = [], errors = [];
      const interval = scenario === "overflow" ? 2_147_483.648 : scenario === "text" ? "9"
        : ["normal", "refreshed-overflow"].includes(scenario) ? 1 : 1e300;
      const session = (status, pollInterval) => ({ session_id: 1, channel: platform, status,
        bind_token: "fixture-only", created_at: "1793361600", updated_at: "1793361600", expires_at: "1793362200",
        entry_url: `https://bind.example.test/${platform}/1`, poll_interval_seconds: pollInterval });
      const state = async () => JSON.parse(await page.locator("#state").innerText());
      const wait = async predicate => {
        const deadline = Date.now() + 8000;
        while (!(await predicate())) {
          if (Date.now() >= deadline) throw new Error("fixture_state_timeout");
          await page.waitForTimeout(10);
        }
      };
      const delays = () => page.evaluate(() => window.fixturePollDelays);
      const polls = () => requests.filter(row => row.path.startsWith("/v1/admin/channel-binds/") && row.method === "GET");
      page.on("pageerror", error => errors.push(error.message));
      await page.addInitScript(() => {
        window.fixturePollDelays = [];
        const setTimer = window.setTimeout.bind(window);
        window.setTimeout = (handler, delay, ...args) => {
          if (typeof handler === "function" && handler.constructor.name === "AsyncFunction") window.fixturePollDelays.push(delay);
          return setTimer(handler, delay, ...args);
        };
      });
      await page.route("**/*", async route => {
        const request = route.request(), url = new URL(request.url());
        if (url.origin === ui.base) return route.continue();
        if (url.origin !== "https://device-a.example.test" && url.origin !== "https://device-b.example.test") return route.abort();
        const reply = data => route.fulfill({ status: 200, contentType: "application/json", headers: {
          "Access-Control-Allow-Origin": ui.base, "Access-Control-Allow-Headers": "content-type", "Access-Control-Allow-Methods": "GET,POST,OPTIONS",
        }, body: JSON.stringify({ ok: true, data, error: null }) });
        if (request.method() === "OPTIONS") return reply(null);
        requests.push({ path: url.pathname, method: request.method(), body: request.postDataJSON() });
        if (url.pathname.endsWith("/start")) return reply(session(scenario === "terminal" ? "bound" : "pending", interval));
        if (url.pathname.startsWith("/fixture/")) return reply({});
        if (scenario === "refreshed-overflow" && polls().length === 1) return reply(session("pending", 2_147_483.648));
        return reply(session("bound", interval));
      });
      try {
        await page.goto(`${ui.base}/test/fixtures/channel-bind-ownership.html?platform=${platform}&theme=${theme}`);
        await page.locator("#state").waitFor();
        await page.locator("[data-begin]").click();
        await wait(async () => (await state()).session !== null);
        if (scenario === "terminal") {
          assert.equal((await state()).session.status, "bound");
          assert.deepEqual(await delays(), []);
          await page.waitForTimeout(150);
          assert.equal(polls().length, 0);
        } else {
          await wait(async () => (await delays()).length > 0);
          assert.deepEqual(await delays(), [scenario === "normal" || scenario === "refreshed-overflow" ? 1000 : 5000]);
          if (scenario === "scope") {
            await page.evaluate(() => window.dispatchEvent(new CustomEvent("fixture-scope", { detail: "device" })));
            await wait(async () => (await state()).session === null);
            await page.waitForTimeout(150);
            assert.equal(polls().length, 0);
          } else if (scenario === "normal") {
            await wait(async () => (await state()).session?.status === "bound");
            assert.equal(polls().length, 1);
            assert.deepEqual(await delays(), [1000]);
          } else if (scenario === "refreshed-overflow") {
            await wait(async () => (await delays()).length === 2);
            assert.deepEqual(await delays(), [1000, 5000]);
            await page.waitForTimeout(150);
            assert.equal(polls().length, 1);
            if (width === 1440 && theme === "light") {
              await wait(async () => (await state()).session?.status === "bound");
              assert.equal(polls().length, 2);
            }
          } else {
            await page.waitForTimeout(150);
            assert.equal(polls().length, 0);
            if (scenario === "overflow" && width === 1440 && theme === "light") {
              await wait(async () => (await state()).session?.status === "bound");
              assert.equal(polls().length, 1);
            }
          }
        }
        const starts = requests.filter(row => row.method === "POST");
        assert.equal(starts.length, 1);
        assert.deepEqual(starts[0].body, {});
        assert.deepEqual(errors, []);
        if (scenario === "overflow") await page.screenshot({ path: `${output}/${platform}-${width}-${theme}.png` });
        results.push({ platform, width, theme, scenario, passed: true, delays: await delays(), polls: polls().length });
      } catch (error) {
        results.push({ platform, width, theme, scenario, passed: false, error: String(error), errors, requests, delays: await delays() });
        await page.screenshot({ path: `${output}/failed-${platform}-${width}-${theme}-${scenario}.png` });
      } finally { await page.close(); }
    }
  }
  await writeFile(`${output}/report.json`, JSON.stringify({ results, real_provider_calls: 0, actual_bindings: 0, actual_resets: 0 }, null, 2));
  console.log(JSON.stringify({ cases: results.length, passed: results.filter(row => row.passed).length, failed: results.filter(row => !row.passed) }));
  assert.deepEqual(results.filter(row => !row.passed), []);
} finally { await browser?.close(); await ui.close(); }
