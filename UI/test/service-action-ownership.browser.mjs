import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { isolatedUiServer } from "./helpers/isolated-ui-server.mjs";

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || "playwright");
const ui = await isolatedUiServer(fileURLToPath(new URL("../", import.meta.url)));
const output = process.env.UI_BROWSER_ARTIFACTS || join(tmpdir(), `service-action-ownership-${Date.now()}`);
await mkdir(output, { recursive: true });
const results = [], services = ["telegramd", "whatsappd", "whatsapp_webd", "wechatd", "feishud", "larkd"];
let browser;
async function waitFor(predicate) {
  const deadline = Date.now() + 5000;
  while (!predicate()) {
    if (Date.now() >= deadline) throw new Error("fixture_request_not_received");
    await new Promise(resolve => setTimeout(resolve, 10));
  }
}
async function fixture(width, theme, refresh = "") {
  const page = await browser.newPage({ viewport: { width, height: width === 390 ? 844 : 1000 } });
  const pending = [], errors = [];
  page.on("pageerror", error => errors.push(error.message));
  await page.route("**/*", route => {
    const request = route.request(), url = new URL(request.url());
    if (url.origin === ui.base) return route.continue();
    if (["https://device-a.example.test", "https://device-b.example.test"].includes(url.origin) && url.pathname.startsWith("/v1/services/")) {
      pending.push({ route, origin: url.origin, path: url.pathname, method: request.method(), body: request.postData() });
      return;
    }
    return route.abort();
  });
  await page.goto(`${ui.base}/test/fixtures/service-action-ownership.html?theme=${theme}&lang=${theme === "light" ? "en" : "zh"}&refresh=${refresh}`);
  await page.locator("[data-action]").first().waitFor();
  page.setDefaultTimeout(5000);
  const click = (service = "wechatd", action = "start") => page.locator(`[data-action="${service}:${action}"]`).click();
  const text = id => page.getByTestId(id).textContent();
  const expectText = (id, value) => page.waitForFunction(({ id, value }) =>
    document.querySelector(`[data-testid="${id}"]`)?.textContent === value, { id, value });
  const switchScope = async scope => {
    const old = await text("scope");
    await page.evaluate(scope => window.dispatchEvent(new CustomEvent("fixture-scope", { detail: scope })), scope);
    await page.waitForFunction(old => document.querySelector('[data-testid="scope"]')?.textContent !== old, old);
  };
  const respond = async (index, status = 200) => {
    const { route } = pending[index];
    const completed = page.waitForResponse(response => response.request() === route.request());
    await route.fulfill({ status, contentType: "application/json", headers: { "Access-Control-Allow-Origin": ui.base },
      body: JSON.stringify({ ok: status === 200, data: { error_code: status === 200 ? undefined : "service_start_not_running" }, error: null }) });
    await (await completed).finished();
    await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  };
  const confirm = () => page.getByRole("alertdialog").locator('button[type="submit"]').click();
  return { page, pending, errors, click, text, expectText, switchScope, respond, confirm };
}
try {
  browser = await chromium.launch({ executablePath: process.env.BROWSER_EXECUTABLE || undefined, headless: true });
  for (const width of [1440, 390]) for (const theme of ["light", "dark"]) {
    const run = async (name, check, refresh = "") => {
      const f = await fixture(width, theme, refresh);
      try {
        await check(f);
        assert.deepEqual(f.errors, []);
        assert.equal(await f.page.evaluate(() => document.documentElement.scrollWidth > innerWidth + 1), false);
        results.push({ width, theme, case: name, passed: true });
      } catch (error) { results.push({ width, theme, case: name, passed: false, error: String(error) }); }
      finally { await f.page.close(); }
    };
    for (const scope of ["device", "epoch", "logout", "role", "return"]) await run(`reset_confirmation_${scope}`, async f => {
      await f.click("wechatd", "reset");
      await f.page.getByRole("alertdialog").waitFor();
      if (scope === "return") { await f.switchScope("device"); await f.switchScope("return"); }
      else await f.switchScope(scope);
      await f.confirm(); await f.page.getByRole("alertdialog").waitFor({ state: "hidden" });
      await f.page.waitForTimeout(60);
      assert.equal(f.pending.length, 0); await f.expectText("results", "[false]");
    });
    for (const service of services) await run(`${service}_current_reset`, async f => {
      await f.page.clock.install(); await f.click(service, "reset");
      await f.page.getByRole("alertdialog").waitFor();
      assert.equal(f.pending.length, 0);
      const box = await f.page.getByRole("alertdialog").boundingBox();
      assert.ok(box && box.x >= 0 && box.y >= 0 && box.x + box.width <= width);
      if (service === "wechatd") await f.page.screenshot({ path: join(output, `reset-${width}-${theme}.png`) });
      await f.confirm(); await waitFor(() => f.pending.length === 1);
      assert.equal(f.pending[0].path, `/v1/services/${service}/reset`);
      assert.equal(f.pending[0].method, "POST"); assert.equal(f.pending[0].body, null);
      await f.respond(0); await f.page.clock.fastForward(850);
      await f.expectText("results", "[true]"); await f.expectText("tone", "success");
      assert.equal(JSON.parse(await f.text("reads")).length, 2);
    });
    for (const action of ["start", "stop", "restart"]) await run(`current_${action}`, async f => {
      await f.page.clock.install(); await f.click("wechatd", action); await waitFor(() => f.pending.length === 1);
      assert.equal(f.pending[0].path, `/v1/services/wechatd/${action}`);
      await f.respond(0); await f.page.clock.fastForward(850); await f.expectText("results", "[true]");
      await f.expectText("tone", "success");
    });
    await run("declined_reset_has_no_dispatch", async f => {
      await f.click("wechatd", "reset"); await f.page.getByRole("alertdialog").waitFor();
      await f.page.keyboard.press("Escape"); await f.expectText("results", "[false]");
      assert.equal(f.pending.length, 0);
    });
    await run("duplicate_reset_is_not_queued", async f => {
      await f.page.clock.install();
      await f.page.evaluate(() => { const button = document.querySelector('[data-action="wechatd:reset"]'); button.click(); button.click(); });
      await f.confirm(); await f.page.getByRole("alertdialog").waitFor({ state: "hidden" });
      await waitFor(() => f.pending.length === 1); await f.respond(0);
      await f.page.clock.fastForward(850); await f.expectText("results", "[false,true]");
      assert.equal(f.pending.length, 1);
    });
    await run("old_ack_does_not_clear_new_busy_state", async f => {
      await f.page.clock.install(); await f.click(); await waitFor(() => f.pending.length === 1);
      await f.switchScope("device"); await f.click(); await waitFor(() => f.pending.length === 2);
      assert.equal(f.pending[1].origin, "https://device-b.example.test");
      await f.respond(0, 503); await f.expectText("busy", '{"wechatd":true}'); await f.expectText("notice", "none");
      await f.respond(1); await f.page.clock.fastForward(850); await f.expectText("results", "[false,true]");
      assert.deepEqual(JSON.parse(await f.text("reads")), ["https://device-b.example.test:health", "https://device-b.example.test:config"]);
    });
    await run("scope_change_during_recovery_suppresses_reads", async f => {
      await f.page.clock.install(); await f.click(); await waitFor(() => f.pending.length === 1); await f.respond(0);
      await f.switchScope("epoch"); await f.page.clock.fastForward(850);
      await f.expectText("results", "[false]"); await f.expectText("notice", "none"); await f.expectText("reads", "[]");
    });
    for (const refresh of ["throw", "reject"]) await run(`accepted_action_survives_refresh_${refresh}`, async f => {
      await f.page.clock.install(); await f.click(); await waitFor(() => f.pending.length === 1); await f.respond(0);
      await f.page.clock.fastForward(850); await f.expectText("results", "[true]"); await f.expectText("tone", "success");
      assert.equal(f.pending.length, 1); assert.equal(JSON.parse(await f.text("reads")).length, 2);
    }, refresh);
    await run("parallel_services_keep_latest_notice", async f => {
      await f.page.clock.install(); await f.click(); await f.click("telegramd"); await waitFor(() => f.pending.length === 2);
      await f.respond(1); const latest = await f.text("notice");
      await f.respond(0, 503); assert.equal(await f.text("notice"), latest);
      await f.page.clock.fastForward(850); await f.expectText("tone", "success");
    });
    await run("current_structured_failure_remains_visible", async f => {
      await f.click(); await waitFor(() => f.pending.length === 1); await f.respond(0, 409);
      await f.expectText("results", "[false]"); await f.expectText("tone", "error");
      assert.notEqual(await f.text("notice"), "none"); assert.equal(f.pending.length, 1);
    });
    console.log(JSON.stringify({ width, theme, cases: results.filter(row => row.width === width && row.theme === theme) }));
  }
  await writeFile(join(output, "service-action-ownership.json"), JSON.stringify({ results, real_provider_calls: 0, actual_service_mutations: 0 }, null, 2));
  assert.equal(results.length, 88);
  assert.deepEqual(results.filter(result => !result.passed), []);
  console.log(JSON.stringify({ status: "passed", cases: results.length, real_provider_calls: 0, actual_service_mutations: 0, output }));
} finally { await browser?.close(); await ui.close(); }
