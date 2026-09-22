import assert from "node:assert/strict";
import { mkdir } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { createServer } from "vite";
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || "playwright");
const root = fileURLToPath(new URL("../", import.meta.url));
const output = process.env.UI_BROWSER_ARTIFACTS || "/tmp/aipp-uninstall-tests";
await mkdir(output, { recursive: true });
const server = await createServer({ root, server: { host: "127.0.0.1", port: 0, strictPort: true } });
const app = (name) => ({
  skill_name: name, package_version: "1", renderer: "collection_feed_v1", data_contract: "media_collection_v1",
  icon: "download", default_locale: "en", titles: { en: name, zh: name }, descriptions: {},
  installed: true, entrypoint: null, bridge_capabilities: [], task_channel_scope: null,
});
let browser;
try {
  await server.listen();
  const base = `http://127.0.0.1:${server.httpServer.address().port}`;
  browser = await chromium.launch({ executablePath: process.env.BROWSER_EXECUTABLE || undefined, headless: true });
  for (const lang of ["zh", "en"]) {
    const context = await browser.newContext({ viewport: { width: lang === "zh" ? 1440 : 390, height: 900 } });
    const page = await context.newPage();
    const errors = [], mutations = [];
    page.on("pageerror", error => errors.push(error.message));
    let apps = [app("example_one"), app("example_two")];
    let rejectRemoval = true, failCatalog = false, releaseJob;
    let jobDone = new Promise(resolve => { releaseJob = resolve; });
    let currentSkill = "example_one";
    const reply = (route, data) => route.fulfill({ json: { ok: true, data } });
    await page.route("**/v1/**", async route => {
      const request = route.request(), pathname = new URL(request.url()).pathname;
      if (request.method() !== "GET") mutations.push({ path: pathname, method: request.method(), body: request.postDataJSON() });
      if (pathname === "/v1/aipps") {
        if (failCatalog) return route.fulfill({ status: 503, json: { ok: false } });
        return reply(route, { schema_version: 1, apps });
      }
      if (pathname === "/v1/skills/store/remove") {
        if (rejectRemoval) return route.fulfill({ status: 409, json: { ok: false, error: "skill_store_operation_busy" } });
        currentSkill = request.postDataJSON().skill_name;
        return reply(route, { operation: { operation_id: "remove-1", action: "remove", skill_name: currentSkill, status: "queued" } });
      }
      if (pathname === "/v1/skills/store/operations/remove-1") {
        await jobDone;
        apps = apps.filter(item => item.skill_name !== currentSkill);
        return reply(route, { operation: { operation_id: "remove-1", action: "remove", skill_name: currentSkill, status: "success", result: { installed: false } } });
      }
      if (pathname.endsWith("/items")) return reply(route, { items: [], platform_states: {}, next_cursor_sequence: null });
      return route.fulfill({ status: 404, json: { ok: false } });
    });
    await page.goto(`${base}/test/fixtures/aipp-uninstall.html?lang=${lang}&theme=${lang === "zh" ? "light" : "dark"}`);
    const openName = (name) => `${lang === "zh" ? "打开" : "Open"}：${name}`;
    const uninstall = lang === "zh" ? "卸载应用和技能" : "Uninstall app and skill";
    await page.getByRole("button", { name: openName("example_one"), exact: true }).click();
    await page.getByRole("button", { name: uninstall, exact: true }).click();
    let dialog = page.getByRole("alertdialog");
    await dialog.waitFor();
    assert.match(await dialog.innerText(), lang === "zh" ? /对应技能 example_one/ : /its skill example_one/);
    assert.match(await dialog.innerText(), lang === "zh" ? /配置和已采集数据会保留/ : /Configuration and collected data will be kept/);
    await page.screenshot({ path: `${output}/confirmation-${lang}.png`, fullPage: true });
    await dialog.getByRole("button", { name: lang === "zh" ? "取消" : "Cancel", exact: true }).last().click();
    assert.equal(mutations.length, 0);
    await page.getByRole("button", { name: uninstall, exact: true }).click();
    await page.getByRole("alertdialog").getByRole("button", { name: uninstall, exact: true }).click();
    await page.getByText(lang === "zh" ? "另一个技能正在安装或删除，请等待完成后重试。" : "Another skill is being installed or removed. Wait for it to finish, then try again.", { exact: true }).waitFor();
    assert.ok(await page.getByRole("button", { name: uninstall, exact: true }).isEnabled());
    rejectRemoval = false;
    await page.getByRole("button", { name: uninstall, exact: true }).click();
    await page.getByRole("alertdialog").getByRole("button", { name: uninstall, exact: true }).click();
    await page.waitForRequest(request => request.url().includes("/operations/remove-1"));
    assert.ok(await page.getByRole("button", { name: uninstall, exact: true }).isDisabled());
    failCatalog = true;
    releaseJob();
    await page.getByRole("button", { name: openName("example_two"), exact: true }).waitFor();
    assert.equal(await page.getByTestId("aipp-launcher-icon").count(), 1);
    assert.equal(await page.getByRole("button", { name: openName("example_one"), exact: true }).count(), 0);
    assert.equal(await page.evaluate(() => document.documentElement.dataset.skillsChanged), "true");
    await page.reload();
    await page.getByRole("button", { name: openName("example_two"), exact: true }).waitFor();
    assert.equal(await page.getByTestId("aipp-launcher-icon").count(), 1);
    const cache = await page.evaluate(() => Object.entries(sessionStorage).find(([key]) => key.endsWith("monitor.aipp.catalog.v1"))?.[1]);
    assert.deepEqual(JSON.parse(cache).apps.map(item => item.skill_name), ["example_two"]);
    failCatalog = false;
    await page.reload();
    await page.getByRole("button", { name: openName("example_two"), exact: true }).click();
    await page.getByRole("button", { name: uninstall, exact: true }).click();
    await page.getByRole("alertdialog").getByRole("button", { name: uninstall, exact: true }).click();
    await page.getByText(lang === "zh" ? "尚未安装应用。" : "No apps installed.", { exact: true }).waitFor();
    assert.equal(await page.getByTestId("aipp-launcher-icon").count(), 0);
    await page.reload();
    await page.getByText(lang === "zh" ? "尚未安装应用。" : "No apps installed.", { exact: true }).waitFor();
    await page.getByRole("button", { name: lang === "zh" ? "打开 Skill Store" : "Open Skill Store", exact: true }).click();
    assert.equal(await page.evaluate(() => document.documentElement.dataset.storeOpened), "true");
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
    assert.ok(mutations.every(call => call.path === "/v1/skills/store/remove" && call.method === "POST" && call.body.preserve_data === true && call.body.preserve_config === true));
    assert.deepEqual(errors, []);
    await page.screenshot({ path: `${output}/empty-${lang}.png`, fullPage: true });
    console.log(`PASS ${lang}: warning, cancel, failure, queued job, successful uninstall, cache, reload, last icon removal, store navigation`);
    await context.close();
  }
} finally { await browser?.close(); await server.close(); }
