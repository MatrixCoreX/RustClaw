import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { isolatedUiServer } from "./helpers/isolated-ui-server.mjs";
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || "playwright");
const output = process.env.UI_BROWSER_ARTIFACTS || "/tmp/channel-bind-app";
await mkdir(output, { recursive: true });
const ui = await isolatedUiServer(fileURLToPath(new URL("../", import.meta.url)));
const origins = ["https://device-a.example.test", "https://device-b.example.test"];
const results = [];
let browser;
try {
  browser = await chromium.launch({ executablePath: process.env.BROWSER_EXECUTABLE || undefined, headless: true });
  for (const theme of ["light", "dark"]) for (const platform of ["feishu", "lark"]) for (const phase of ["begin", "poll", "reset", "ordinary", "parallel", "transport-error", "timing-error", "session-error", "session-error-mobile"]) {
    if (phase === "parallel" && platform === "lark") continue;
    const page = await browser.newPage({ viewport: { width: phase === "session-error-mobile" ? 390 : 1440, height: 1000 } });
    const requests = [], errors = [], held = [];
    let loggedIn = false, watch = true;
    const title = platform === "lark" ? "Lark" : "Feishu";
    const cardFor = name => page.locator(".channel-setup-card").filter({ has: page.getByRole("heading", { name, exact: true }) });
    const card = cardFor(title);
    const session = (channel, origin) => ({ session_id: origin === origins[0] ? 1 : 2, channel, status: "pending", bind_token: "fixture-only",
      created_at: "1793361600", updated_at: "1793361600", expires_at: "1793362200",
      entry_url: `https://bind.example.test/${channel}/${origin === origins[0] ? 1 : 2}`, poll_interval_seconds: 1 });
    const wait = async predicate => {
      const deadline = Date.now() + 10000;
      while (!(await predicate())) { if (Date.now() >= deadline) throw new Error("fixture_state_timeout"); await page.waitForTimeout(10); }
    };
    const reply = (route, data, status = 200, error = "fixture_unavailable") => route.fulfill({ status, contentType: "application/json", headers: {
      "Access-Control-Allow-Origin": ui.base, "Access-Control-Allow-Credentials": "true",
      "Access-Control-Allow-Headers": "content-type,x-agent-key,x-webd-csrf", "Access-Control-Allow-Methods": "GET,POST,OPTIONS",
    }, body: JSON.stringify({ ok: status === 200, data, error: status === 200 ? null : error }) });
    await page.addInitScript(value => {
      localStorage.setItem("agent-runtime.monitor.currentPage", "services");
      localStorage.setItem("agent-runtime.monitor.pollingSeconds", "0");
      localStorage.setItem("agent-runtime.monitor.lang", "en");
      localStorage.setItem("agent-runtime.monitor.themeMode", value);
    }, theme);
    page.on("pageerror", error => errors.push(error.message));
    await page.route("**/*", async route => {
      const request = route.request(), url = new URL(request.url());
      if (url.origin === ui.base && !url.pathname.startsWith("/v1/") && !url.pathname.startsWith("/webd/")) return route.continue();
      if (!origins.includes(url.origin)) return route.abort();
      if (request.method() === "OPTIONS") return reply(route, null);
      requests.push({ origin: url.origin, path: url.pathname, method: request.method() });
      if (url.pathname === "/webd/login") { loggedIn = true; return reply(route, { csrf_token: "ab".repeat(16) }); }
      if (url.pathname === "/webd/logout") { loggedIn = false; return reply(route, {}); }
      if (url.pathname === "/webd/session") return reply(route, { logged_in: loggedIn, csrf_token: "ab".repeat(16) });
      if (url.pathname === "/v1/auth/me") return reply(route, { user_id: 101, chat_id: 301,
        role: phase === "ordinary" ? "user" : "admin", user_key: "fixture-user-key" });
      if (url.pathname === "/v1/health") return reply(route, { version: "fixture", queue_length: 0, running_length: 0,
        worker_state: "idle", uptime_seconds: 1, task_timeout_seconds: 0, running_oldest_age_seconds: 0,
        wechatd_healthy: false, feishud_healthy: false, larkd_healthy: false });
      const bind = url.pathname.match(/^\/v1\/admin\/channel-binds\/(feishu|lark)\/(start|[0-9]+)$/);
      if (bind) {
        const [, channel, action] = bind;
        if ((phase === "transport-error" || phase === "timing-error") && action !== "start") {
          return phase === "timing-error"
            ? reply(route, null, 409, "channel_registration_poll_interval_invalid")
            : reply(route, null, 502, "channel_bind_registration_response_invalid");
        }
        if ((phase === "session-error" || phase === "session-error-mobile") && action !== "start") {
          return reply(route, { ...session(channel, url.origin), status: "failed", error_text: "channel_bind_registration_start_failed" });
        }
        if (watch && url.origin === origins[0] && ((phase === "begin" && action === "start") || (phase === "poll" && action !== "start") || phase === "parallel")) {
          held.push(route); return;
        }
        return reply(route, session(channel, url.origin));
      }
      if (url.pathname === `/v1/admin/${platform}/reset` && watch && phase === "reset") { held.push(route); return; }
      return reply(route, null, 503);
    });
    const login = async origin => {
      await page.getByLabel("Webd URL (optional)", { exact: true }).fill(origin);
      await page.getByLabel("Username", { exact: true }).fill("fixture-user");
      await page.getByLabel("Password", { exact: true }).fill("fixture-password");
      await page.getByRole("button", { name: "Enter Console", exact: true }).click();
      if (phase !== "session-error-mobile") await page.getByRole("button", { name: "Log out", exact: true }).waitFor();
      await card.getByRole("button", { name: `Start ${title} setup`, exact: true }).waitFor();
    };
    const release = async (index, data) => {
      const route = held[index], response = page.waitForResponse(value => value.request() === route.request());
      await reply(route, data); await (await response).finished(); await page.waitForTimeout(50);
    };
    try {
      await page.goto(ui.base); await page.getByLabel("Webd URL (optional)", { exact: true }).waitFor(); page.setDefaultTimeout(10000);
      await login(origins[0]);
      if (phase === "ordinary") {
        assert.equal(await card.getByRole("button", { name: `Start ${title} setup`, exact: true }).isDisabled(), true);
        assert.equal(await card.getByRole("button", { name: `Reset ${title}`, exact: true }).isDisabled(), true);
        assert.deepEqual(requests.filter(row => row.path.startsWith("/v1/admin/channel-binds/")), []);
      } else {
        await card.getByRole("button", { name: `Start ${title} setup`, exact: true }).click();
        if (phase === "session-error" || phase === "session-error-mobile") {
          await card.getByText("The configuration was saved, but the channel did not start. Review and save the channel configuration, then start the service, or reset setup and scan again.", { exact: true }).waitFor();
          const polls = () => requests.filter(row => row.path.startsWith(`/v1/admin/channel-binds/${platform}/`) && row.method === "GET");
          assert.equal(polls().length, 1);
          await page.waitForTimeout(1500);
          assert.equal(polls().length, 1);
          assert.equal(await card.getByText("channel_bind_registration_start_failed", { exact: true }).count(), 0);
        } else if (phase === "transport-error" || phase === "timing-error") {
          await card.getByText("The channel could not complete this setup request. Check your connection and start a new scan later.", { exact: true }).waitFor();
          const polls = () => requests.filter(row => row.path.startsWith(`/v1/admin/channel-binds/${platform}/`) && row.method === "GET");
          assert.equal(polls().length, 1);
          await page.waitForTimeout(1500);
          assert.equal(polls().length, 1);
          assert.equal(await card.getByText("channel_bind_registration_response_invalid", { exact: true }).count(), 0);
        } else if (phase === "parallel") {
          await wait(() => held.length === 1);
          await cardFor("Lark").getByRole("button", { name: "Start Lark setup", exact: true }).click();
          await wait(() => held.length === 2); watch = false;
          await release(1, session("lark", origins[0])); await page.getByAltText("Lark QR", { exact: true }).waitFor();
          assert.equal(await page.getByAltText("Feishu QR", { exact: true }).count(), 0);
          await release(0, session("feishu", origins[0])); await page.getByAltText("Feishu QR", { exact: true }).waitFor();
          for (const name of ["Feishu", "Lark"]) assert.equal(await page.getByAltText(`${name} QR`, { exact: true }).evaluate(node => node.naturalWidth), 288);
        } else {
          if (phase === "reset") {
            await page.getByAltText(`${title} QR`, { exact: true }).waitFor();
            await card.getByRole("button", { name: `Reset ${title}`, exact: true }).click();
            await page.getByRole("alertdialog").getByRole("button", { name: "Reset", exact: true }).click();
          }
          await wait(() => held.length === 1);
          await page.getByRole("button", { name: "Log out", exact: true }).click();
          await login(origins[1]); assert.equal(await page.getByAltText(`${title} QR`, { exact: true }).count(), 0);
          const before = requests.length; watch = false;
          await release(0, phase === "reset" ? {} : { ...session(platform, origins[0]), status: phase === "poll" ? "bound" : "pending" });
          await page.waitForTimeout(1100);
          assert.deepEqual(requests.slice(before).filter(row => row.origin === origins[0]), []);
          assert.equal(await page.getByAltText(`${title} QR`, { exact: true }).count(), 0);
          await card.getByRole("button", { name: `Start ${title} setup`, exact: true }).click();
          await page.getByAltText(`${title} QR`, { exact: true }).waitFor();
          assert.equal(await page.getByAltText(`${title} QR`, { exact: true }).evaluate(node => node.naturalWidth), 288);
        }
      }
      assert.deepEqual(errors, []);
      await page.screenshot({ path: `${output}/app-${platform}-${theme}-${phase}.png` });
      results.push({ platform, theme, phase, passed: true });
    } catch (error) { results.push({ platform, theme, phase, passed: false, error: String(error), errors, requests }); }
    finally { await page.close(); }
  }
  await writeFile(`${output}/report.json`, JSON.stringify({ results, real_provider_calls: 0, actual_bindings: 0, actual_resets: 0 }, null, 2));
  console.log(JSON.stringify({ cases: results.length, passed: results.filter(row => row.passed).length, failed: results.filter(row => !row.passed) }));
  assert.deepEqual(results.filter(row => !row.passed), []);
} finally { await browser?.close(); await ui.close(); }
