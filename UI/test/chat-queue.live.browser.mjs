import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdir, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";
import path from "node:path";
import { fileURLToPath } from "node:url";

const origin = process.env.UI_LIVE_TEST_ORIGIN;
assert.ok(origin && ["127.0.0.1", "localhost"].includes(new URL(origin).hostname));
assert.ok(process.env.UI_LIVE_TEST_DB && process.env.UI_LIVE_TEST_PROVIDER && process.env.UI_LIVE_TEST_MODEL);
const require = createRequire(import.meta.url);
const Database = require("better-sqlite3");
const db = new Database(process.env.UI_LIVE_TEST_DB, { readonly: true, fileMustExist: true });
const credential = db.prepare("SELECT user_key FROM auth_keys WHERE role = 'admin' AND enabled = 1 ORDER BY created_at LIMIT 1").get();
db.close();
assert.ok(credential?.user_key);
const root = fileURLToPath(new URL("../../", import.meta.url));
const output = process.env.UI_BROWSER_ARTIFACTS || "/tmp/agent-chat-queue-tests";
await mkdir(output, { recursive: true });
const cases = [
  "请使用工具读取当前工作区 README.md 的第一行，只回复这一行原文。不要修改文件。",
  "继续，把你上一条回复的标题原样返回，并在前面加上 QUEUED:。不要调用工具或修改文件。",
];
function trace(index, args) {
  if (process.env.PRINT_LLM_TRACE === "0") return "";
  const result = spawnSync("python3", [path.join(root, "scripts/nl_tests/print_llm_raw_trace.py"),
    "--log", path.join(root, "logs/model_io.log"), "--state-file", path.join(output, `trace-${index}.json`),
    "--max-field-chars", "1200", ...args], { encoding: "utf8" });
  assert.equal(result.status, 0, result.stderr);
  process.stdout.write(result.stdout);
  return result.stdout;
}
for (let i = 0; i < cases.length; i++) {
  trace(i, ["--init-state"]);
  console.log(JSON.stringify({ run_id: path.basename(output), suite: "chat-queue-live", ordinal: i + 1,
    case_id: `queue-${i + 1}`, nl: cases[i], provider: process.env.UI_LIVE_TEST_PROVIDER,
    model: process.env.UI_LIVE_TEST_MODEL, dry_run: false, skip: false }));
}
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || "playwright");
const browser = await chromium.launch({ executablePath: process.env.BROWSER_EXECUTABLE, headless: true });
const taskIds = [], posts = [], results = [], errors = [];
try {
  const context = await browser.newContext({ viewport: { width: 1440, height: 1000 } });
  const page = await context.newPage();
  page.setDefaultTimeout(30_000);
  page.on("pageerror", error => errors.push(error.message));
  const terminal = new Set();
  let submittedBeforePredecessorFinished = false;
  page.on("request", request => {
    if (new URL(request.url()).pathname === "/v1/tasks" && request.method() === "POST") {
      const payload = request.postDataJSON().payload;
      if (posts.length && !terminal.has(taskIds[0])) submittedBeforePredecessorFinished = true;
      posts.push(payload);
    }
  });
  page.on("response", async response => {
    const pathname = new URL(response.url()).pathname;
    if (pathname === "/v1/tasks" && response.request().method() === "POST") {
      const body = await response.json();
      if (body.data?.task_id) {
        taskIds.push(body.data.task_id);
        console.log(JSON.stringify({ case_id: `queue-${taskIds.length}`, task_id: body.data.task_id }));
      }
    } else if (/^\/v1\/tasks\/[^/]+$/.test(pathname) && response.request().method() === "GET") {
      const body = await response.json();
      if (["succeeded", "failed", "canceled", "timeout"].includes(body.data?.status)) terminal.add(body.data.task_id);
    }
  });
  await page.goto(origin);
  await page.evaluate(() => {
    localStorage.setItem("agent-runtime.monitor.lang", "zh");
    localStorage.setItem("agent-runtime.monitor.themeMode", "light");
    localStorage.setItem("agent-runtime.monitor.baseUrl", location.origin);
  });
  await page.reload();
  await page.getByRole("button", { name: "使用 Key 登录", exact: true }).click();
  await page.getByPlaceholder("输入已经生成好的 user_key").fill(credential.user_key);
  const history = page.waitForResponse(response => response.url().includes("/v1/tasks/conversation-history"));
  await page.getByRole("button", { name: "进入控制台", exact: true }).click();
  await (await history).finished();
  await page.getByRole("button", { name: "Agent", exact: true }).first().click();
  await page.getByRole("button", { name: "新建任务", exact: true }).click();
  const chat = page.locator("#agent-chat-window");
  const composer = page.getByTestId("chat-composer");
  await chat.getByRole("checkbox").check();
  const send = composer.locator("button.chat-send-btn");
  await composer.locator("textarea").fill(cases[0]);
  const firstSubmit = page.waitForResponse(response => new URL(response.url()).pathname === "/v1/tasks" && response.request().method() === "POST");
  await send.click();
  await (await firstSubmit).finished();
  await page.getByTestId("chat-working-indicator").waitFor();
  await composer.locator("textarea").fill(cases[1]);
  assert.equal(await send.isEnabled(), true);
  assert.equal(await send.innerText(), "排队");
  await composer.locator("textarea").press("Enter");
  const queue = page.getByTestId("chat-message-queue");
  await queue.waitFor();
  assert.equal(posts.length, 1);
  assert.ok((await queue.innerText()).includes(cases[1]));
  await composer.locator("textarea").fill("此条仅用于检查移除队列，不应发给模型。");
  await send.click();
  assert.equal(await queue.locator("li").count(), 2);
  await queue.getByRole("button", { name: "移除待发送消息" }).nth(1).click();
  assert.equal(await queue.locator("li").count(), 1);
  await page.screenshot({ path: path.join(output, "queued-desktop.png"), fullPage: true });
  await page.getByRole("button", { name: "占满浏览器窗口", exact: true }).click();
  const q = await queue.boundingBox(), c = await composer.boundingBox();
  assert.ok(q && c && q.y + q.height <= c.y);
  await page.screenshot({ path: path.join(output, "queued-maximized.png"), fullPage: true });
  await page.keyboard.press("Escape");
  await page.setViewportSize({ width: 390, height: 844 });
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
  await page.screenshot({ path: path.join(output, "queued-mobile.png"), fullPage: true });
  await page.setViewportSize({ width: 1440, height: 1000 });
  const deadline = Date.now() + 300_000;
  while (Date.now() < deadline && results.length < 2) {
    for (const id of taskIds.slice(results.length)) {
      const response = await context.request.get(`${origin}/v1/tasks/${id}`, { headers: { "x-agent-key": credential.user_key } });
      const body = await response.json();
      assert.equal(body.ok, true);
      if (!["succeeded", "failed", "canceled", "timeout"].includes(body.data.status)) break;
      results.push(body.data);
    }
    if (results.length < 2) await page.waitForTimeout(1000);
  }
  assert.equal(results.length, 2);
  assert.ok(results.every(result => result.status === "succeeded"));
  await page.getByTestId("chat-working-indicator").waitFor({ state: "detached" });
  assert.equal(await queue.count(), 0);
  assert.equal(posts.length, 2);
  assert.equal(posts[0].conversation_id, posts[1].conversation_id);
  assert.equal(submittedBeforePredecessorFinished, false);
  assert.match(results[1].result_json.text, /QUEUED:.*Agent Runtime/s);
  assert.deepEqual(errors, []);
  await page.screenshot({ path: path.join(output, "completed.png"), fullPage: true });
  console.log(JSON.stringify({ status: "PASS", task_ids: taskIds, requests: posts.length, sequential: true, artifacts: output }));
} finally {
  await writeFile(path.join(output, "result.json"), JSON.stringify({ taskIds, posts, results, errors }, null, 2));
  await browser.close();
  for (const [index, taskId] of taskIds.entries()) {
    await writeFile(path.join(output, `llm-${index + 1}.txt`), trace(index, ["--task-id", taskId]));
  }
}
