import assert from "node:assert/strict";
import { mkdir } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { isolatedUiServer } from "./helpers/isolated-ui-server.mjs";

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || "playwright");
const ui = await isolatedUiServer(fileURLToPath(new URL("../", import.meta.url)));
const output = process.env.UI_BROWSER_ARTIFACTS || join(tmpdir(), `system-host-ownership-${Date.now()}`);
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

async function fixture(width, theme) {
  const page = await browser.newPage({ viewport: { width, height: width === 390 ? 844 : 1000 } });
  const pending = [], errors = [];
  page.on("pageerror", error => errors.push(error.message));
  await page.route("**/*", async route => {
    const url = new URL(route.request().url());
    if (url.origin === ui.base) return route.continue();
    if (["https://device-a.example.test", "https://device-b.example.test"].includes(url.origin)
      && ["/v1/system/host-summary", "/v1/system/dependencies"].includes(url.pathname)) {
      pending.push({ route, origin: url.origin, path: url.pathname });
      return;
    }
    return route.abort();
  });
  await page.goto(`${ui.base}/test/fixtures/system-host-ownership.html?theme=${theme}`);
  const click = name => page.getByRole("button", { name, exact: true }).click();
  const expectText = async (id, text) => {
    await page.waitForFunction(({ id, text }) => document.querySelector(`[data-testid="${id}"]`)?.textContent === text,
      { id, text });
  };
  const start = async name => {
    const index = pending.length;
    await click(name);
    await waitFor(() => pending.length === index + 1);
    return index;
  };
  const respond = async (index, marker, status = 200) => {
    const { route, path } = pending[index];
    const response = page.waitForResponse(value => value.request() === route.request());
    const data = path.endsWith("host-summary") ? { schema_version: 4, architecture: marker }
      : { schema_version: 1, platform: marker, dependencies: [], operations: [], summary: {} };
    await route.fulfill({ status, contentType: "application/json",
      headers: { "Access-Control-Allow-Origin": ui.base },
      body: JSON.stringify({ ok: status === 200, data: status === 200 ? data : null, error: null }) });
    await (await response).finished();
    await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  };
  return { page, pending, errors, click, start, respond, expectText };
}

try {
  browser = await chromium.launch({ executablePath: process.env.BROWSER_EXECUTABLE || undefined, headless: true });
  for (const width of [1440, 390]) for (const theme of ["light", "dark"]) {
    const f = await fixture(width, theme);
    try {
      const old = await f.start("Summary"), latest = await f.start("Summary");
      await f.respond(latest, "current");
      await f.respond(old, "obsolete");
      await f.expectText("summary", "current");
      const failed = await f.start("Summary"), current = await f.start("Summary");
      await f.respond(failed, null, 403);
      await f.expectText("summary-loading", "true");
      await f.expectText("summary-error", "none");
      await f.respond(current, "current");
      await f.expectText("summary-loading", "false");
      results.push({ width, theme, case: "latest_summary_and_obsolete_error", passed: true });

      const manual = await f.start("Dependencies");
      await f.click("Poll dependencies");
      await f.page.evaluate(() => new Promise(resolve => setTimeout(resolve, 75)));
      assert.equal(f.pending.length, manual + 1);
      await f.expectText("dependencies-loading", "true");
      await f.respond(manual, "current");
      await f.expectText("dependencies", "current");
      const poll = await f.start("Poll dependencies");
      await f.respond(poll, "polled");
      await f.expectText("dependencies", "polled");
      await f.expectText("dependencies-loading", "false");
      results.push({ width, theme, case: "manual_refresh_owns_silent_poll", passed: true });

      const oldEndpoint = await f.start("Summary");
      await f.click("Switch device");
      await f.expectText("summary", "empty");
      await f.expectText("dependencies", "empty");
      await f.respond(oldEndpoint, "obsolete-device");
      await f.expectText("summary", "empty");
      const newEndpoint = await f.start("Summary");
      assert.equal(f.pending[newEndpoint].origin, "https://device-b.example.test");
      await f.respond(newEndpoint, "new-device");
      await f.expectText("summary", "new-device");
      results.push({ width, theme, case: "device_transition", passed: true });

      const privileged = await f.start("Dependencies");
      await f.click("Use ordinary role");
      await f.expectText("summary", "empty");
      await f.respond(privileged, "obsolete-admin");
      await f.expectText("dependencies", "empty");
      const ordinary = await f.start("Dependencies");
      await f.respond(ordinary, "ordinary");
      await f.expectText("dependencies", "ordinary");
      results.push({ width, theme, case: "role_transition", passed: true });

      const signedIn = await f.start("Summary");
      await f.click("Sign out");
      await f.respond(signedIn, "obsolete-authenticated");
      await f.expectText("summary", "empty");
      await f.expectText("dependencies", "empty");
      await f.expectText("summary-loading", "false");
      const count = f.pending.length;
      await f.click("Summary"); await f.click("Dependencies");
      await f.page.evaluate(() => new Promise(resolve => setTimeout(resolve, 75)));
      assert.equal(f.pending.length, count);
      results.push({ width, theme, case: "authentication_loss", passed: true });
      assert.deepEqual(f.errors, []);
      assert.equal(await f.page.evaluate(() => document.documentElement.scrollWidth > innerWidth + 1), false);
      await f.page.screenshot({ path: join(output, `host-ownership-${width}-${theme}.png`) });
      console.log(JSON.stringify({ width, theme, cases: 5, real_provider_calls: 0, passed: true }));
    } finally { await f.page.close(); }
  }
  assert.equal(results.length, 20);
  console.log(JSON.stringify({ status: "passed", cases: results, real_provider_calls: 0, output }));
} finally { await browser?.close(); await ui.close(); }
