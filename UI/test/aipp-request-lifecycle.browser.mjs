import assert from "node:assert/strict";
import { mkdir } from "node:fs/promises";
import path from "node:path";
import os from "node:os";
import { fileURLToPath } from "node:url";
import { isolatedUiServer } from "./helpers/isolated-ui-server.mjs";
import { guardFixtureRequests } from "./helpers/fixture-network.mjs";

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || "playwright");
const output = process.env.UI_BROWSER_ARTIFACTS || path.join(os.tmpdir(), "aipp-request-lifecycle");
await mkdir(output, { recursive: true });
const server = await isolatedUiServer(fileURLToPath(new URL("../", import.meta.url)));
let browser;
try {
  browser = await chromium.launch({ executablePath: process.env.BROWSER_EXECUTABLE || undefined, headless: true });
  for (const width of [1440, 390]) for (const theme of ["dark", "light"]) {
    const context = await browser.newContext({ viewport: { width, height: 900 } });
    let held;
    try {
      const page = await context.newPage();
      await guardFixtureRequests(page, server.base);
      const errors = [];
      page.on("pageerror", error => errors.push(error.message));
      await page.addInitScript(() => {
        window.fixtureItemReads = [];
        const original = window.fetch;
        window.fetch = function(input, init) {
          const url = typeof input === "string" ? input : input instanceof Request ? input.url : String(input);
          if (url.includes("/v1/aipps/example_collection/items?")) {
            const signal = init?.signal;
            const request = { hasSignal: !!signal, aborted: !!signal?.aborted };
            window.fixtureItemReads.push(request);
            signal?.addEventListener("abort", () => { request.aborted = true; });
          }
          return original.call(this, input, init);
        };
      });
      const app = { skill_name: "example_collection", package_version: "1", renderer: "task_activity_v1", data_contract: "skill_task_activity_v1",
        icon: "download", default_locale: "en", titles: { en: "Fixture" }, descriptions: {}, installed: true,
        entrypoint: null, bridge_capabilities: [], task_channel_scope: null,
        account_controls: { schema_version: 1, status_capability: "fixture.account_status", logout_capability: "fixture.account_logout" } };
      const body = { schema_version: 1, page_item_count: 0, total_item_count: 0, sort_order: "newest", next_cursor_sequence: null, updated_at_ms: null, items: [] };
      const json = data => ({ json: { ok: true, data } });
      let itemReads = 0, accountStarts = 0;
      await page.route("**/v1/**", route => {
        const url = new URL(route.request().url());
        if (url.pathname === "/v1/aipps") return route.fulfill(json({ apps: [app] }));
        if (url.pathname.endsWith("/items")) {
          itemReads++;
          if (itemReads === 2) { held = route; return; }
          return route.fulfill(json(body));
        }
        if (url.pathname.endsWith("/account-control")) {
          assert.equal(JSON.parse(route.request().postData()).operation, "status");
          accountStarts++;
          return route.fulfill(json({ task_id: "fixture-account" }));
        }
        if (url.pathname === "/v1/tasks/fixture-account") return route.fulfill(json({ status: "succeeded", result_json: { extra: { accounts: [
          { schema_version: 1, platform: "weibo", status: "verified", display_name: "Fixture account", checked_at: 123, session_present: true },
        ] } } }));
        return route.abort();
      });
      await page.goto(`${server.base}/test/fixtures/aipp-collection.html?lang=en&theme=${theme}`);
      const accounts = page.getByRole("button", { name: "Platform accounts", exact: true });
      await accounts.waitFor();
      assert.equal(accountStarts, 0, "collapsed panels must not submit tasks");
      await accounts.click(); await page.getByText("Fixture account", { exact: true }).waitFor();
      assert.equal(accountStarts, 1);
      await accounts.click(); await accounts.click();
      assert.equal(accountStarts, 1, "reopening preserves the first result without a new task");
      await accounts.click();
      await page.evaluate(() => document.dispatchEvent(new Event("visibilitychange")));
      await page.waitForFunction(() => window.fixtureItemReads.length === 2);
      await page.getByRole("tab", { name: "Usage guide", exact: true }).click();
      await page.waitForFunction(() => window.fixtureItemReads[1].aborted);
      await page.getByRole("tab", { name: "Results", exact: true }).click();
      await page.waitForFunction(() => window.fixtureItemReads.length === 3);
      await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
      await page.evaluate(() => document.dispatchEvent(new Event("visibilitychange")));
      await page.waitForFunction(() => window.fixtureItemReads.length === 4);
      assert.equal(accountStarts, 1, "the new collapsed view must remain lazy");
      assert.ok((await page.evaluate(() => window.fixtureItemReads)).every(read => read.hasSignal));
      assert.deepEqual(errors, []);
      await page.screenshot({ path: path.join(output, `${width}-${theme}.png`) });
      console.log(`PASS ${width} ${theme}: lazy account check, stable collapse/reopen, cancelled old refresh, new view refresh before old response`);
    } finally {
      await held?.abort().catch(() => {});
      await context.close();
    }
  }
} finally { await browser?.close(); await server.close(); }
