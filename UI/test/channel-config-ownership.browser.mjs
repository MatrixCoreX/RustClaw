import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { isolatedUiServer } from "./helpers/isolated-ui-server.mjs";
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || "playwright");
const output = process.env.UI_BROWSER_ARTIFACTS || "/tmp/channel-config-ownership";
await mkdir(output, { recursive: true });
const ui = await isolatedUiServer(fileURLToPath(new URL("../", import.meta.url)));
const origins = ["https://device-a.example.test", "https://device-b.example.test"];
const channels = ["wechat", "feishu", "lark", "telegram"];
const results = [];
let browser;
const config = (channel, label, name = "primary") => channel === "telegram"
  ? { config_path: label, restart_required: true, agents: [], bots: [{ name, bot_token: "", bot_token_configured: true,
    bot_token_masked: label, agent_id: "main", allowlist: [], access_mode: "public", allowed_telegram_usernames: [], is_primary: true }] }
  : { config_path: label, enabled: false, listen: "127.0.0.1:8792", clawd_base_url: "http://127.0.0.1:8787",
    api_base_url: "https://example.test", wechat_uin_base64: "", request_timeout_seconds: 30, longpoll_timeout_ms: 35000,
    text_chunk_chars: 1200, restart_required: true, mode: "websocket", app_id: "", app_secret: "",
    verification_token_configured: false, encrypt_key_configured: false, bind_ready: false, current_key_bound: false };
try {
  browser = await chromium.launch({ executablePath: process.env.BROWSER_EXECUTABLE || undefined, headless: true });
  for (const width of [1440, 390]) for (const theme of ["light", "dark"]) {
    for (const scenario of [...channels.map(channel => `late-read:${channel}`), "scope-clear", "late-write", "dirty-refresh", "dirty-save", "duplicate-save", "role", "epoch", "logout"]) {
      const page = await browser.newPage({ viewport: { width, height: width === 390 ? 844 : 1000 } });
      const requests = [], held = [], errors = [];
      let hold = false, serial = 0;
      const state = async () => JSON.parse(await page.locator("#state").innerText());
      const wait = async predicate => {
        const deadline = Date.now() + 5000;
        while (!(await predicate())) {
          if (Date.now() >= deadline) throw new Error("fixture_state_timeout");
          await page.waitForTimeout(10);
        }
      };
      const reply = (route, data) => route.fulfill({ status: 200, contentType: "application/json", headers: {
        "Access-Control-Allow-Origin": ui.base, "Access-Control-Allow-Headers": "content-type", "Access-Control-Allow-Methods": "GET,POST,OPTIONS",
      }, body: JSON.stringify({ ok: true, data, error: null }) });
      const scope = async detail => {
        const before = JSON.stringify((await state()).scope);
        await page.evaluate(value => window.dispatchEvent(new CustomEvent("fixture-scope", { detail: value })), detail);
        await wait(async () => JSON.stringify((await state()).scope) !== before);
      };
      const read = channel => page.locator(`[data-read="${channel}"]`).click();
      const load = async channel => { await read(channel); await wait(async () => !(await state())[channel].loading); };
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
        if (hold) { held.push(route); return; }
        const channel = url.pathname.split("/")[2];
        return reply(route, config(channel, `${url.hostname}:${++serial}`));
      });
      try {
        await page.goto(`${ui.base}/test/fixtures/channel-config-ownership.html?theme=${theme}`);
        await page.locator("#state").waitFor(); page.setDefaultTimeout(5000);
        if (scenario.startsWith("late-read:")) {
          const channel = scenario.split(":")[1];
          hold = true; await read(channel); await wait(() => held.length === 1);
          await scope("device"); hold = false; await load(channel);
          const fresh = (await state())[channel].data.config_path;
          await release(0, config(channel, "obsolete-a"));
          assert.equal((await state())[channel].data.config_path, fresh);
        } else if (scenario === "scope-clear") {
          for (const channel of channels) await load(channel);
          await page.getByLabel("Draft token", { exact: true }).fill("fixture-only");
          await scope("device");
          for (const channel of channels) assert.equal((await state())[channel].data, null);
          assert.equal((await state()).telegram.draftToken, "");
        } else if (scenario === "late-write") {
          await load("wechat"); hold = true; await page.locator('[data-write="wechat"]').click(); await wait(() => held.length === 1);
          await scope("device"); hold = false; await load("wechat");
          const fresh = (await state()).wechat.data.config_path;
          await release(0, { ...config("wechat", "obsolete-a"), enabled: true });
          assert.equal((await state()).wechat.data.config_path, fresh);
          assert.equal((await state()).wechat.saving, false);
        } else if (scenario === "dirty-refresh") {
          await load("telegram"); await page.getByLabel("Draft name", { exact: true }).fill("keep-draft");
          await load("telegram");
          assert.equal((await state()).telegram.draftName, "keep-draft");
          assert.equal((await state()).telegram.dirty, true);
        } else if (scenario === "dirty-save" || scenario === "duplicate-save") {
          await load("telegram"); await page.getByLabel("Draft name", { exact: true }).fill("submitted"); hold = true;
          await page.locator('[data-write="telegram"]').click(); await wait(() => held.length === 1);
          if (scenario === "dirty-save") await page.getByLabel("Draft name", { exact: true }).fill("newer-draft");
          else {
            await page.locator('[data-write="telegram"]').click();
            assert.equal(requests.filter(row => row.method === "POST").length, 1);
          }
          await release(0, config("telegram", "accepted", "submitted"));
          assert.equal((await state()).telegram.saving, false);
          assert.equal((await state()).telegram.draftName, scenario === "dirty-save" ? "newer-draft" : "submitted");
          assert.equal((await state()).telegram.dirty, scenario === "dirty-save");
          if (scenario === "dirty-save") assert.equal((await state()).telegram.message, null);
        } else {
          await load("telegram"); hold = true; await read("telegram"); await wait(() => held.length === 1);
          await scope(scenario); await release(0, config("telegram", "obsolete"));
          assert.equal((await state()).telegram.data, null);
          assert.equal((await state()).telegram.loading, false);
        }
        assert.deepEqual(errors, []);
        results.push({ width, theme, scenario, passed: true });
      } catch (error) {
        results.push({ width, theme, scenario, passed: false, error: String(error), errors, requests });
        await page.screenshot({ path: `${output}/failed-${width}-${theme}-${scenario.replaceAll(":", "-")}.png` });
      } finally { await page.close(); }
    }
  }
  await writeFile(`${output}/report.json`, JSON.stringify({ results, real_provider_calls: 0, actual_config_mutations: 0 }, null, 2));
  console.log(JSON.stringify({ cases: results.length, passed: results.filter(row => row.passed).length, failed: results.filter(row => !row.passed) }));
  assert.deepEqual(results.filter(row => !row.passed), []);
} finally { await browser?.close(); await ui.close(); }
