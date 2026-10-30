import assert from "node:assert/strict";
import { mkdir } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { isolatedUiServer } from "./helpers/isolated-ui-server.mjs";

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || "playwright");
const ui = await isolatedUiServer(fileURLToPath(new URL("../", import.meta.url)));
const output = process.env.UI_BROWSER_ARTIFACTS || join(tmpdir(), `system-admin-ownership-${Date.now()}`);
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
  let navigations = 0;
  page.on("pageerror", error => errors.push(error.message));
  page.on("framenavigated", frame => { if (frame === page.mainFrame()) navigations++; });
  await page.route("**/*", async route => {
    const url = new URL(route.request().url());
    if (url.origin === ui.base) return route.continue();
    if (["https://device-a.example.test", "https://device-b.example.test"].includes(url.origin)
      && (url.pathname.startsWith("/v1/admin/") || ["/v1/pi-app/status", "/v1/system/host-summary", "/v1/system/dependencies"].includes(url.pathname))) {
      pending.push({ route, origin: url.origin, path: url.pathname }); return;
    }
    return route.abort();
  });
  await page.goto(`${ui.base}/test/fixtures/system-admin-ownership.html?theme=${theme}`);
  const click = name => page.getByRole("button", { name, exact: true }).click();
  const expectText = (id, text) => page.waitForFunction(({ id, text }) =>
    document.querySelector(`[data-testid="${id}"]`)?.textContent === text, { id, text });
  const start = async (name, path) => {
    const before = pending.length;
    await click(name);
    await waitFor(() => pending.slice(before).some(request => request.path === path));
    return pending.findIndex((request, index) => index >= before && request.path === path);
  };
  const respond = async (index, marker, status = 200, buildStatus = "idle") => {
    const { route, path } = pending[index];
    let data = { supported: true, platform: marker, available: true, model: marker,
      installed: true, running: true, configured: true, enabled: true, port: 8788,
      externally_accessible: false, listen: "127.0.0.1:8788", restart_scheduled: false };
    if (path === "/v1/admin/workspace-update") data = { status: buildStatus, mode: "build_ui", step: "fixture",
      current_version: marker, stdout_tail: "", stderr_tail: "" };
    const completed = page.waitForResponse(response => response.request() === route.request());
    await route.fulfill({ status, contentType: "application/json", headers: { "Access-Control-Allow-Origin": ui.base },
      body: JSON.stringify({ ok: status === 200, data: status === 200 ? data : null, error: null }) });
    await (await completed).finished();
    await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  };
  return { page, pending, errors, get navigations() { return navigations; }, click, expectText, start, respond };
}
const workspacePath = "/v1/admin/workspace-update";
try {
  browser = await chromium.launch({ executablePath: process.env.BROWSER_EXECUTABLE || undefined, headless: true });
  for (const width of [1440, 390]) for (const theme of ["light", "dark"]) {
    const f = await fixture(width, theme);
    try {
      const old = await f.start("Workspace", workspacePath), latest = await f.start("Workspace", workspacePath);
      await f.respond(latest, "current"); await f.respond(old, "obsolete");
      await f.expectText("workspace-version", "current");
      results.push({ width, theme, case: "workspace_latest_read", passed: true });
      for (const [button, path, field] of [["Nginx", "/v1/admin/nginx", "nginx"], ["Webd", "/v1/admin/webd-exposure", "webd"]]) {
        const previous = await f.start(button, path), current = await f.start(button, path);
        await f.respond(current, "current"); await f.respond(previous, null, 503);
        await f.expectText(`${field}-platform`, "current"); await f.expectText(`${field}-error`, "none");
        results.push({ width, theme, case: `${field}_obsolete_failure`, passed: true });
      }
      await f.respond(await f.start("Workspace", workspacePath), "old-build", 200, "running");
      await f.click("Switch device");
      await f.expectText("workspace-version", "empty");
      const idle = await f.start("Workspace", workspacePath);
      assert.equal(f.pending[idle].origin, "https://device-b.example.test");
      await f.respond(idle, "new-device");
      await f.page.waitForTimeout(750);
      assert.equal(f.navigations, 1);
      await f.expectText("workspace-version", "new-device");
      results.push({ width, theme, case: "old_build_history_does_not_reload_new_device", passed: true });
      const firstDashboard = f.pending.length;
      const hardwareOld = await f.start("Open dashboard", "/v1/pi-app/status");
      await waitFor(() => f.pending.length >= firstDashboard + 6);
      await f.click("Open chat");
      const secondDashboard = f.pending.length;
      const hardwareCurrent = await f.start("Open dashboard", "/v1/pi-app/status");
      await waitFor(() => f.pending.length >= secondDashboard + 6);
      await f.respond(hardwareCurrent, "current-hardware"); await f.respond(hardwareOld, "obsolete-hardware");
      await f.expectText("hardware-model", "current-hardware");
      results.push({ width, theme, case: "hardware_actual_dashboard_effect", passed: true });
      await f.click("Ordinary role");
      for (const field of ["workspace-version", "nginx-platform", "webd-platform", "hardware-model"]) await f.expectText(field, "empty");
      const adminReads = () => f.pending.filter(request => request.path.startsWith("/v1/admin/")
        || request.path === "/v1/pi-app/status").length;
      const count = adminReads();
      await f.click("Workspace"); await f.click("Nginx"); await f.click("Webd"); await f.click("Sign out");
      await f.page.waitForTimeout(75);
      assert.equal(adminReads(), count);
      assert.deepEqual(f.errors, []);
      assert.equal(await f.page.evaluate(() => document.documentElement.scrollWidth > innerWidth + 1), false);
      await f.page.screenshot({ path: join(output, `admin-ownership-${width}-${theme}.png`) });
      results.push({ width, theme, case: "access_transition", passed: true });
    } finally { await f.page.close(); }
    const reload = await fixture(width, theme);
    try {
      await reload.respond(await reload.start("Workspace", workspacePath), "building", 200, "running");
      await reload.respond(await reload.start("Workspace", workspacePath), "done", 200, "succeeded");
      await reload.click("Switch device");
      await reload.page.waitForTimeout(750);
      assert.equal(reload.navigations, 1);
      await reload.expectText("workspace-version", "empty");
      results.push({ width, theme, case: "scheduled_reload_is_canceled_on_scope_change", passed: true });
      await reload.respond(await reload.start("Workspace", workspacePath), "building", 200, "running");
      const navigation = reload.page.waitForEvent("framenavigated", { predicate: frame => frame === reload.page.mainFrame() });
      await reload.respond(await reload.start("Workspace", workspacePath), "done", 200, "succeeded");
      await navigation;
      await reload.page.getByRole("button", { name: "Workspace", exact: true }).waitFor();
      assert.equal(reload.navigations, 2);
      assert.deepEqual(reload.errors, []);
      results.push({ width, theme, case: "current_build_still_reloads_once", passed: true });
    } finally { await reload.page.close(); }
    console.log(JSON.stringify({ width, theme, cases: 8, real_provider_calls: 0, passed: true }));
  }
  assert.equal(results.length, 32);
  console.log(JSON.stringify({ status: "passed", cases: results, real_provider_calls: 0, output }));
} finally { await browser?.close(); await ui.close(); }
