import assert from "node:assert/strict";
import { mkdir } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { createServer } from "vite";

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || "playwright");
const root = fileURLToPath(new URL("../", import.meta.url));
const output = process.env.UI_BROWSER_ARTIFACTS || "/tmp/model-credentials-browser-tests";
await mkdir(output, { recursive: true });
const vendors = ["openai", "google", "anthropic", "grok", "deepseek", "qwen", "minimax", "mimo", "custom"];
const server = await createServer({ root, server: { host: "127.0.0.1", port: 0, strictPort: true } });
let browser;
try {
  await server.listen();
  const base = `http://127.0.0.1:${server.httpServer.address().port}`;
  browser = await chromium.launch({ executablePath: process.env.BROWSER_EXECUTABLE || undefined, headless: true });
  for (const width of [1440, 390]) for (const theme of ["light", "dark"]) {
    const zh = theme === "light";
    const page = await browser.newPage({ viewport: { width, height: width === 390 ? 844 : 1100 } });
    const errors = [], requests = [];
    page.on("pageerror", error => errors.push(error.message));
    const config = {
      config_path: "configs/config.toml", selected_vendor: "minimax", selected_model: "fixture-model", restart_required: false,
      vendors: vendors.map(name => ({ name, default_model: "fixture-model", models: ["fixture-model"], base_url: "https://provider.example/v1", api_key_configured: false })),
      hosted_relay: { vendor: "custom", model: "relay-model", base_url: "https://relay.example/v1", api_format: "openai_compat", daily_request_limit: 1000 },
    };
    await page.route("**/v1/**", async route => {
      const path = new URL(route.request().url()).pathname;
      const body = route.request().postDataJSON();
      if (body) {
        requests.push({ path, body });
        if (path === "/v1/llm/config") {
          config.selected_vendor = body.selected_vendor;
          config.selected_model = body.selected_model;
          const vendor = config.vendors.find(v => v.name === body.selected_vendor);
          if (body.vendor_api_key) { vendor.api_key_configured = true; vendor.api_key_source = "environment_file"; }
        }
      }
      const data = path === "/v1/llm/test" ? { success: true, vendor: "minimax", model: "fixture-model", response_text: "ok" }
        : path === "/v1/llm/config" ? config : { entries: [] };
      await route.fulfill({ contentType: "application/json", body: JSON.stringify({ ok: true, data }) });
    });
    await page.goto(`${base}/test/fixtures/model-credentials.html?lang=${zh ? "zh" : "en"}&theme=${theme}`);
    const mode = page.getByLabel(zh ? "使用方式" : "Connection mode");
    const key = page.getByLabel("API Key", { exact: true });
    await key.waitFor();
    assert.equal(await mode.inputValue(), "minimax", "missing environment must not force relay mode");
    for (const vendor of vendors) {
      await mode.selectOption(vendor);
      await key.waitFor();
      assert.equal(await key.getAttribute("type"), "password");
      assert.equal(await key.inputValue(), "");
      await key.fill("fixture-browser-key");
      await page.getByRole("button", { name: zh ? "测试连接" : "Test Connection", exact: true }).click();
      await page.waitForFunction(() => document.body.innerText.includes("ok"));
      assert.equal(requests.at(-1).body.vendor_api_key, "fixture-browser-key");
      await page.getByRole("button", { name: zh ? "保存模型设置" : "Save LLM Settings", exact: true }).click();
      await page.waitForFunction(() => document.querySelector('input[type="password"]').value === "");
      assert.equal(requests.at(-1).path, "/v1/llm/config");
      assert.equal(requests.at(-1).body.vendor_api_key, "fixture-browser-key");
      await page.waitForFunction(text => document.querySelector('input[type="password"]').placeholder.includes(text), zh ? "留空" : "Leave blank");
      assert.ok((await key.getAttribute("placeholder")).includes(zh ? "留空" : "Leave blank"));
    }
    await mode.selectOption("minimax");
    await key.scrollIntoViewIfNeeded();
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth + 1), false);
    await page.screenshot({ path: `${output}/${width}-${theme}.png` });
    await page.reload(); await key.waitFor();
    assert.equal(await key.inputValue(), "");
    assert.equal(await page.evaluate(() => JSON.stringify(localStorage).includes("fixture-browser-key") || JSON.stringify(sessionStorage).includes("fixture-browser-key")), false);
    await key.fill("must-not-follow-to-relay");
    const relay = await mode.locator("option").first().getAttribute("value");
    await mode.selectOption(relay);
    assert.equal(await key.count(), 0);
    await mode.selectOption("minimax");
    assert.equal(await key.inputValue(), "");
    assert.deepEqual(errors, []);
    console.log(`PASS ${width} ${theme}: nine vendors, key entry/test/save, refreshed state, relay isolation, no browser persistence or horizontal overflow`);
    await page.close();
  }
} finally { await browser?.close(); await server.close(); }
