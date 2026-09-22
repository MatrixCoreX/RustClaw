import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdir, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";
import path from "node:path";
import { fileURLToPath } from "node:url";

const origin = process.env.UI_LIVE_TEST_ORIGIN;
const databasePath = process.env.UI_LIVE_TEST_DB;
if (!origin || !databasePath || !["127.0.0.1", "localhost"].includes(new URL(origin).hostname)) {
  throw new Error("Provide an explicit loopback UI_LIVE_TEST_ORIGIN and UI_LIVE_TEST_DB for this live, read-only task test.");
}
const require = createRequire(import.meta.url);
const Database = require("better-sqlite3");
const database = new Database(databasePath, { readonly: true, fileMustExist: true });
const credential = database.prepare("SELECT user_key FROM auth_keys WHERE role = 'admin' AND enabled = 1 ORDER BY created_at LIMIT 1").get();
database.close();
assert.ok(credential?.user_key, "An existing administrator credential is required; the test never creates or resets one.");

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || "playwright");
const output = process.env.UI_BROWSER_ARTIFACTS || "/tmp/agent-chat-progress-tests";
await mkdir(output, { recursive: true });
const root = fileURLToPath(new URL("../../", import.meta.url));
const traceState = path.join(output, "llm-trace-state.json");
const printTrace = (args) => {
  if (process.env.PRINT_LLM_TRACE === "0") return "";
  const trace = spawnSync("python3", [path.join(root, "scripts/nl_tests/print_llm_raw_trace.py"),
    "--log", path.join(root, "logs/model_io.log"), "--state-file", traceState,
    "--max-field-chars", "1200", ...args], { encoding: "utf8" });
  assert.equal(trace.status, 0, trace.stderr);
  if (trace.stdout) process.stdout.write(trace.stdout);
  return trace.stdout;
};
printTrace(["--init-state"]);
const browser = await chromium.launch({ executablePath: process.env.BROWSER_EXECUTABLE || undefined, headless: true });
const errors = [];
const prompt = "请使用工具读取当前工作区 README.md 的前 5 行，只回复第一行标题。不要修改文件，也不要执行任何安装或部署操作。";
let taskId;
try {
  const context = await browser.newContext({ viewport: { width: 1440, height: 1000 } });
  const page = await context.newPage();
  page.setDefaultTimeout(30_000);
  page.on("pageerror", error => errors.push(error.message));
  await page.goto(origin);
  await page.evaluate(() => {
    localStorage.setItem("agent-runtime.monitor.lang", "zh");
    localStorage.setItem("agent-runtime.monitor.themeMode", "light");
    localStorage.setItem("agent-runtime.monitor.baseUrl", location.origin);
  });
  await page.reload();
  await page.getByRole("button", { name: "使用 Key 登录", exact: true }).click();
  await page.getByPlaceholder("输入已经生成好的 user_key").fill(credential.user_key);
  await page.getByRole("button", { name: "进入控制台", exact: true }).click();
  await page.getByRole("button", { name: "Agent", exact: true }).first().click();
  await page.getByRole("button", { name: "新建任务", exact: true }).click();
  const chat = page.locator("#agent-chat-window");
  const composer = page.getByTestId("chat-composer");
  await composer.locator("textarea").fill(prompt);
  console.log(JSON.stringify({ run_id: path.basename(output), suite: "chat-progress-live", ordinal: 1,
    case_id: "chat-progress-live", nl: prompt, provider: process.env.UI_LIVE_TEST_PROVIDER,
    model: process.env.UI_LIVE_TEST_MODEL, dry_run: false, skip: false }));
  const submitted = page.waitForResponse(response => new URL(response.url()).pathname === "/v1/tasks" && response.request().method() === "POST");
  await composer.locator("button.chat-send-btn").click();
  const submission = await (await submitted).json();
  assert.equal(submission.ok, true);
  taskId = submission.data.task_id;
  console.log(JSON.stringify({ case_id: "chat-progress-live", task_id: taskId, nl: prompt, dry_run: false }));
  const indicator = page.getByTestId("chat-working-indicator");
  await indicator.waitFor();
  assert.equal(await page.getByTestId("chat-message-list").getByTestId("chat-working-indicator").count(), 0);
  const assertPlacement = async () => {
    const position = await page.evaluate(() => {
      const progress = document.querySelector('[data-testid="chat-working-indicator"]');
      const composer = document.querySelector('[data-testid="chat-composer"]');
      const messages = document.querySelector('[data-testid="chat-message-list"]');
      const p = progress.getBoundingClientRect(), c = composer.getBoundingClientRect();
      return { outside: !messages.contains(progress), above: p.bottom <= c.top + 1,
        fullWidth: Math.abs(p.width - c.width) < 2, pageFits: document.documentElement.scrollWidth <= innerWidth };
    });
    assert.deepEqual(position, { outside: true, above: true, fullWidth: true, pageFits: true });
  };
  await assertPlacement();
  await page.screenshot({ path: path.join(output, "desktop-light.png"), fullPage: true });
  await page.getByRole("button", { name: "占满浏览器窗口", exact: true }).click();
  await assertPlacement();
  await page.screenshot({ path: path.join(output, "desktop-maximized.png"), fullPage: true });
  await page.keyboard.press("Escape");
  await page.setViewportSize({ width: 390, height: 844 });
  await indicator.scrollIntoViewIfNeeded();
  await assertPlacement();
  await page.evaluate(() => { document.documentElement.dataset.theme = "dark"; });
  await page.screenshot({ path: path.join(output, "mobile-dark.png"), fullPage: true });
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.evaluate(() => { document.documentElement.dataset.theme = "light"; });
  const observed = new Set();
  const capturedCalls = new Set();
  let result;
  const deadline = Date.now() + 300_000;
  while (Date.now() < deadline) {
    if (await indicator.count()) {
      const text = await indicator.innerText();
      observed.add(text);
      const call = text.match(/LLM (\d+)/)?.[1];
      if (call && !capturedCalls.has(call)) {
        await assertPlacement();
        await page.screenshot({ path: path.join(output, `llm-${call}.png`), fullPage: true });
        capturedCalls.add(call);
      }
    }
    const response = await context.request.get(`${origin}/v1/tasks/${taskId}`, { headers: { "x-agent-key": credential.user_key } });
    const body = await response.json();
    assert.equal(body.ok, true, `Task status lookup failed: HTTP ${response.status()}`);
    result = body.data;
    if (["succeeded", "failed", "canceled", "timeout"].includes(result.status)) break;
    await page.waitForTimeout(1000);
  }
  await writeFile(path.join(output, "result.json"), JSON.stringify({ task_id: taskId, result, observed: [...observed], errors }, null, 2));
  assert.equal(result?.status, "succeeded", `Read-only task did not succeed: ${result?.status}`);
  await indicator.waitFor({ state: "detached", timeout: 30_000 });
  assert.ok(await chat.getByTestId("chat-message-list").innerText());
  assert.ok([...observed].some(text => /LLM [2-9]/.test(text)), "Observe a subsequent model request, not just submission.");
  await page.screenshot({ path: path.join(output, "completed.png"), fullPage: true });
  await page.getByRole("button", { name: "新建任务", exact: true }).click();
  assert.equal(await indicator.count(), 0);
  assert.deepEqual(errors, []);
  console.log(JSON.stringify({ status: "PASS", task_id: taskId, observed: [...observed], screenshots: output }));
} catch (error) {
  console.error(JSON.stringify({ status: "FAIL", task_id: taskId, message: error.message, errors, artifacts: output }));
  throw error;
} finally {
  await browser.close();
  if (taskId) await writeFile(path.join(output, "llm-trace.txt"), printTrace(["--task-id", taskId]));
}
