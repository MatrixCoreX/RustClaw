import assert from "node:assert/strict";
import { mkdir } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { createServer } from "vite";

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || "playwright");
const root = fileURLToPath(new URL("../", import.meta.url));
const output = process.env.UI_BROWSER_ARTIFACTS || path.join(os.tmpdir(), "nni-navigation-tests");
const preference = "agent-runtime.monitor.nniNavigationVisible";
await mkdir(output, { recursive: true });
const server = await createServer({ root, server: { host: "127.0.0.1", port: 0, strictPort: true } });
let browser;
try {
  await server.listen();
  const origin = `http://127.0.0.1:${server.httpServer.address().port}`;
  browser = await chromium.launch({ executablePath: process.env.BROWSER_EXECUTABLE || undefined, headless: true });
  for (const width of [1440, 390]) for (const lang of ["zh", "en"]) {
    for (const saved of [null, "true", "false"]) {
      const context = await browser.newContext({ viewport: { width, height: 1000 } });
      const page = await context.newPage();
      const errors = [];
      const mutations = [];
      page.on("pageerror", error => errors.push(error.message));
      await context.route("**/*", async route => {
        const request = route.request();
        const url = new URL(request.url());
        if (url.origin !== origin) return route.abort();
        if (!url.pathname.startsWith("/v1/") && !url.pathname.startsWith("/webd/")) return route.continue();
        if (request.method() !== "GET") mutations.push(`${request.method()} ${url.pathname}`);
        if (url.pathname === "/webd/session") {
          return route.fulfill({ json: { ok: true, data: { logged_in: true, csrf_token: "ab".repeat(16) } } });
        }
        if (url.pathname === "/v1/auth/me") {
          return route.fulfill({ json: { ok: true, data: { user_id: 1, chat_id: 1, role: "admin", key_id: 1 } } });
        }
        return route.fulfill({ status: 503, json: { ok: false, error: "fixture_read_unavailable" } });
      });
      // Set up only this isolated browser profile, never a real device's preferences.
      await page.goto(origin);
      await page.evaluate(({ lang, saved, preference }) => {
        localStorage.clear();
        localStorage.setItem("agent-runtime.monitor.authMode", "webd");
        localStorage.setItem("agent-runtime.monitor.baseUrl", location.origin);
        localStorage.setItem("agent-runtime.monitor.webdBaseUrl", location.origin);
        localStorage.setItem("agent-runtime.monitor.lang", lang);
        localStorage.setItem("agent-runtime.monitor.themeMode", lang === "zh" ? "light" : "dark");
        localStorage.setItem("agent-runtime.monitor.dashboardSection", "nni_navigation");
        if (saved !== null) localStorage.setItem(preference, saved);
      }, { lang, saved, preference });
      await page.reload();
      const controls = page.getByRole("group", { name: lang === "zh" ? "NNI 导航启用设置" : "NNI navigation enablement" });
      const enable = controls.getByRole("button", { name: lang === "zh" ? "启用" : "Enable", exact: true });
      const disable = controls.getByRole("button", { name: lang === "zh" ? "关闭" : "Disable", exact: true });
      const assertState = async expected => {
        await controls.waitFor();
        await page.waitForFunction(({ preference, expected }) => localStorage.getItem(preference) === String(expected), { preference, expected });
        assert.equal(await enable.getAttribute("aria-pressed"), String(expected));
        assert.equal(await disable.getAttribute("aria-pressed"), String(!expected));
        const nav = width < 1024 ? page.locator("header").first() : page.locator("aside");
        if (width < 1024) await nav.getByRole("button", { name: lang === "zh" ? "导航" : "Nav", exact: true }).click();
        for (const name of ["NNI", "BANCOR", lang === "zh" ? "资产" : "Assets"]) {
          assert.equal(await nav.getByRole("button", { name, exact: true }).count(), expected ? 1 : 0, name);
        }
        if (width < 1024) await nav.getByRole("button", { name: lang === "zh" ? "导航" : "Nav", exact: true }).click();
      };
      await assertState(saved === "true");
      await page.screenshot({ path: path.join(output, `${width}-${lang}-${saved ?? "fresh"}.png`), fullPage: true });
      if (saved === null) {
        await enable.click();
        const dialog = page.getByRole("alertdialog");
        await dialog.waitFor();
        await dialog.getByRole("button", { name: lang === "zh" ? "取消" : "Cancel", exact: true }).last().click();
        await assertState(false);
        await enable.click();
        await dialog.getByRole("button", { name: lang === "zh" ? "确认并启用" : "Confirm and Enable", exact: true }).click();
        await assertState(true);
        await page.reload();
        await assertState(true);
        await disable.click();
        await assertState(false);
        await page.reload();
        await assertState(false);
      } else {
        await page.reload();
        await assertState(saved === "true");
      }
      assert.deepEqual(errors, []);
      assert.deepEqual(mutations, [], "Navigation preferences must not mutate backend state");
      assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
      console.log(`PASS ${width} ${lang} saved=${saved}: default/restore, refresh, navigation, no backend mutation`);
      await context.close();
    }
  }
} finally {
  if (browser) await browser.close();
  await server.close();
}
