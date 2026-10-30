import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { isolatedUiServer } from "./helpers/isolated-ui-server.mjs";

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || "playwright");
const ui = await isolatedUiServer(fileURLToPath(new URL("../", import.meta.url)));
const output = process.env.UI_BROWSER_ARTIFACTS || join(tmpdir(), `system-action-ownership-${Date.now()}`);
await mkdir(output, { recursive: true });
const results = [];
let browser;
async function waitFor(predicate) {
  const deadline = Date.now() + 5000;
  while (!predicate()) {
    if (Date.now() >= deadline) throw new Error("fixture_request_not_received");
    await new Promise(resolve => setTimeout(resolve, 10));
  }
}
async function fixture(width, theme, lang) {
  const page = await browser.newPage({ viewport: { width, height: width === 390 ? 844 : 1000 }, acceptDownloads: true });
  const pending = [], errors = [], downloads = [];
  page.on("pageerror", error => errors.push(error.message));
  page.on("download", download => downloads.push(download.suggestedFilename()));
  await page.route("**/*", async route => {
    const request = route.request(), url = new URL(request.url());
    if (url.origin === ui.base) return route.continue();
    if (["https://device-a.example.test", "https://device-b.example.test"].includes(url.origin) && url.pathname.startsWith("/v1/")) {
      if (request.method() === "OPTIONS") return route.fulfill({ status: 204, headers: {
        "Access-Control-Allow-Origin": ui.base, "Access-Control-Allow-Methods": "GET, POST, OPTIONS",
        "Access-Control-Allow-Headers": "Content-Type",
      } });
      pending.push({ route, origin: url.origin, path: url.pathname, method: request.method(), body: request.postDataJSON() });
      return;
    }
    return route.abort();
  });
  await page.goto(`${ui.base}/test/fixtures/system-action-ownership.html?theme=${theme}&lang=${lang}`);
  const click = name => page.getByRole("button", { name, exact: true }).click();
  const text = id => page.getByTestId(id).textContent();
  const expectText = (id, value) => page.waitForFunction(({ id, value }) =>
    document.querySelector(`[data-testid="${id}"]`)?.textContent === value, { id, value });
  const switchScope = async scope => {
    const old = await text("scope");
    await page.evaluate(scope => window.dispatchEvent(new CustomEvent("fixture-scope", { detail: scope })), scope);
    await page.waitForFunction(old => document.querySelector('[data-testid="scope"]')?.textContent !== old, old);
  };
  const respond = async (index, data, status = 200) => {
    const { route } = pending[index];
    const completed = page.waitForResponse(response => response.request() === route.request());
    await route.fulfill({ status, contentType: "application/json", headers: { "Access-Control-Allow-Origin": ui.base },
      body: JSON.stringify({ ok: status === 200, data, error: status === 200 ? null : "fixture_unavailable" }) });
    await (await completed).finished();
    await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  };
  const confirmed = async (button, choice = false) => {
    await click(button);
    const dialog = page.getByRole("alertdialog");
    await dialog.waitFor();
    if (choice) await dialog.getByRole("button", { name: /Include nginx|包含 nginx/ }).click();
    else await dialog.locator('button[type="submit"]').click();
  };
  return { page, pending, errors, downloads, click, text, expectText, switchScope, respond, confirmed };
}
const workspace = marker => ({ status: "running", mode: "local_https_enable", current_version: marker });
const webd = marker => ({ platform: marker, enabled: true, externally_accessible: true, restart_scheduled: false });
try {
  browser = await chromium.launch({ executablePath: process.env.BROWSER_EXECUTABLE || undefined, headless: true });
  for (const width of [1440, 390]) for (const theme of ["light", "dark"]) {
    const lang = theme === "light" ? "en" : "zh";
    const run = async (name, check) => {
      const f = await fixture(width, theme, lang);
      try {
        await check(f);
        assert.deepEqual(f.errors, []);
        assert.equal(await f.page.evaluate(() => document.documentElement.scrollWidth > innerWidth + 1), false);
        results.push({ width, theme, lang, case: name, passed: true });
      } catch (error) {
        results.push({ width, theme, lang, case: name, passed: false, error: String(error) });
      } finally { await f.page.close(); }
    };
    for (const button of ["Expose port", "Build all", "Prompt"]) await run(`dialog_theme_${button}`, async f => {
      await f.click(button);
      const dialog = f.page.getByRole("alertdialog");
      await dialog.waitFor();
      const colors = await dialog.evaluate(element => {
        const resolve = token => {
          const probe = document.createElement("span");
          probe.style.color = `var(${token})`; document.body.append(probe);
          const color = getComputedStyle(probe).color; probe.remove(); return color;
        };
        return { title: getComputedStyle(element.querySelector("h2")).color,
          message: getComputedStyle(element.querySelector("p")).color,
          expectedTitle: resolve("--theme-text-strong"), expectedMessage: resolve("--theme-text-body"),
          expectedDanger: resolve("--theme-danger-text"),
          label: element.querySelector("label") ? getComputedStyle(element.querySelector("label")).color : null,
          submit: element.querySelector('button[type="submit"]') ? getComputedStyle(element.querySelector('button[type="submit"]')).color : null,
          choices: [...element.querySelectorAll(".theme-choice-btn")].map(choice =>
            [...choice.querySelectorAll("span")].map(span => getComputedStyle(span).color)) };
      });
      assert.equal(colors.title, colors.expectedTitle);
      assert.equal(colors.message, colors.expectedMessage);
      if (colors.label) assert.equal(colors.label, colors.expectedMessage);
      if (button === "Expose port") assert.equal(colors.submit, colors.expectedDanger);
      for (const [label, description] of colors.choices) {
        assert.equal(label, colors.expectedTitle); assert.equal(description, colors.expectedMessage);
      }
      const box = await dialog.boundingBox();
      assert.ok(box && box.x >= 0 && box.y >= 0 && box.x + box.width <= width
        && box.y + box.height <= (width === 390 ? 844 : 1000));
      await f.page.screenshot({ path: join(output, `dialog-${button.replaceAll(" ", "-")}-${width}-${theme}.png`) });
      await f.page.keyboard.press("Escape");
      await dialog.waitFor({ state: "hidden" });
      assert.equal(f.pending.length, 0);
    });
    for (const [button, choice] of [["Expose port", false], ["Build all", true], ["Stop update", false]]) {
      for (const scope of ["device", "epoch", "logout", "role"]) await run(`${button}_confirmation_${scope}`, async f => {
        await f.click(button);
        const dialog = f.page.getByRole("alertdialog");
        await dialog.waitFor();
        await f.switchScope(scope);
        if (choice) await dialog.getByRole("button", { name: /Include nginx|包含 nginx/ }).click();
        else await dialog.locator('button[type="submit"]').click();
        await dialog.waitFor({ state: "hidden" });
        await f.page.waitForTimeout(60);
        assert.equal(f.pending.length, 0);
      });
    }
    await run("current_write_supersedes_reads_and_preserves_request", async f => {
      await f.click("Read webd"); await waitFor(() => f.pending.length === 1);
      await f.confirmed("Expose port"); await waitFor(() => f.pending.length === 2);
      assert.deepEqual(f.pending[1].body, { externally_accessible: true });
      assert.equal(f.pending[1].method, "POST");
      await f.respond(1, webd("accepted")); await f.respond(0, webd("obsolete"));
      await f.expectText("webd-platform", "accepted");
      await f.expectText("webd-updating", "false");
      assert.notEqual(await f.text("webd-message"), "none");
      await f.page.screenshot({ path: join(output, `action-ownership-${width}-${theme}.png`) });
    });
    await run("duplicate_confirmations_are_not_queued_or_dispatched", async f => {
      await f.page.evaluate(() => {
        const button = [...document.querySelectorAll("main button")].find(button => button.textContent === "Expose port");
        button.click(); button.click();
      });
      const dialog = f.page.getByRole("alertdialog");
      await dialog.locator('button[type="submit"]').click();
      await waitFor(() => f.pending.length === 1);
      await dialog.waitFor({ state: "hidden" });
      await f.respond(0, webd("accepted"));
      await f.expectText("webd-updating", "false");
      assert.equal(f.pending.length, 1);
    });
    await run("delayed_exposure_refresh_belongs_to_its_original_scope", async f => {
      await f.page.clock.install();
      await f.confirmed("Expose port"); await waitFor(() => f.pending.length === 1);
      await f.respond(0, { ...webd("accepted"), restart_scheduled: true });
      await f.switchScope("epoch"); await f.page.clock.fastForward(5100);
      assert.equal(f.pending.length, 1);
      await f.expectText("webd-message", "none");
      await f.confirmed("Expose port"); await waitFor(() => f.pending.length === 2);
      await f.respond(1, { ...webd("current"), restart_scheduled: true });
      await f.page.clock.fastForward(5100); await waitFor(() => f.pending.length === 3);
      assert.equal(f.pending[2].method, "GET");
      await f.respond(2, webd("refreshed")); await f.expectText("webd-platform", "refreshed");
    });
    await run("accepted_workspace_ack_does_not_leak_after_epoch_change", async f => {
      await f.confirmed("Build UI"); await waitFor(() => f.pending.length === 1);
      await f.switchScope("epoch");
      await f.confirmed("Build UI"); await waitFor(() => f.pending.length === 2);
      await f.respond(0, workspace("obsolete"));
      await f.expectText("workspace-version", "empty"); await f.expectText("workspace-loading", "true");
      await f.respond(1, workspace("current"));
      await f.expectText("workspace-version", "current"); await f.expectText("workspace-loading", "false");
    });
    await run("current_workspace_409_and_cancel_feedback", async f => {
      await f.confirmed("Build UI"); await waitFor(() => f.pending.length === 1);
      await f.respond(0, workspace("already-running"), 409);
      await f.expectText("workspace-version", "already-running");
      await f.confirmed("Stop update"); await waitFor(() => f.pending.length === 2);
      assert.equal(f.pending[1].path, "/v1/admin/workspace-update/cancel");
      await f.respond(1, { ...workspace("stopping"), status: "canceling" });
      await f.expectText("workspace-version", "stopping"); await f.expectText("workspace-canceling", "false");
      assert.notEqual(await f.text("workspace-error"), "none");
    });
    await run("stale_diagnostic_download_is_suppressed", async f => {
      await f.click("Export diagnostics"); await waitFor(() => f.pending.length === 1);
      await f.switchScope("logout");
      await f.respond(0, { generated_at_ts: 42, size_limit_bytes: 10000, schema_version: 1 });
      await f.page.waitForTimeout(60);
      assert.deepEqual(f.downloads, []); await f.expectText("diagnostic-error", "none");
    });
    await run("current_diagnostic_export_still_downloads", async f => {
      await f.click("Export diagnostics"); await waitFor(() => f.pending.length === 1);
      await f.respond(0, { generated_at_ts: 42, size_limit_bytes: 10000, schema_version: 1 });
      await waitFor(() => f.downloads.length === 1);
      assert.deepEqual(f.downloads, ["system-diagnostics-42.json"]);
    });
    await run("restart_recovery_stops_observing_after_scope_change", async f => {
      await f.page.clock.install();
      await f.click("Restart system"); await waitFor(() => f.pending.length === 1);
      await f.respond(0, {}); await f.switchScope("device");
      await f.page.clock.fastForward(2500);
      assert.equal(f.pending.length, 1);
      await f.expectText("restart-message", "none"); await f.expectText("restart-busy", "false");
      await f.expectText("config-reads", "0");
    });
    await run("current_restart_still_refreshes_health_and_configuration", async f => {
      await f.page.clock.install();
      await f.click("Restart system"); await waitFor(() => f.pending.length === 1);
      await f.respond(0, {}); await f.page.clock.fastForward(1900);
      await waitFor(() => f.pending.length === 2);
      assert.equal(f.pending[1].path, "/v1/health");
      await f.respond(1, { version: "fixture-healthy" });
      await f.expectText("health", "fixture-healthy"); await f.expectText("config-reads", "4");
      await f.expectText("restart-busy", "false");
    });
    console.log(JSON.stringify({ width, theme, cases: results.filter(row => row.width === width && row.theme === theme) }));
  }
  await writeFile(join(output, "system-action-ownership.json"), JSON.stringify({ results, real_provider_calls: 0 }, null, 2));
  assert.equal(results.length, 96);
  assert.deepEqual(results.filter(result => !result.passed), []);
  console.log(JSON.stringify({ status: "passed", cases: results.length, real_provider_calls: 0, output }));
} finally { await browser?.close(); await ui.close(); }
