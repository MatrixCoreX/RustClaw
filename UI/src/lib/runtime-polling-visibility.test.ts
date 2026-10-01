import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

function source(relativePath: string): string {
  return readFileSync(new URL(relativePath, import.meta.url), "utf8");
}

test("high-frequency page polling pauses while the browser document is hidden", () => {
  for (const relativePath of [
    "../hooks/useLogsRuntime.ts",
    "../hooks/useSkillsRuntime.ts",
    "../hooks/useSystemRuntime.ts",
    "../hooks/useTaskRuntime.ts",
    "../hooks/useWechatRuntime.ts",
    "../hooks/useWhatsappWebRuntime.ts",
  ]) {
    const text = source(relativePath);
    assert.match(text, /document\.visibilityState/);
    assert.match(text, /addEventListener\("visibilitychange"/);
    assert.match(text, /removeEventListener\("visibilitychange"/);
  }
});

test("channel login polling uses a single visible polling lane", () => {
  const wechat = source("../hooks/useWechatRuntime.ts");
  assert.match(wechat, /statusRequestRef/);
  assert.match(wechat, /qrPollRequestRef/);
  assert.match(wechat, /if \(wechatSessionKey && !wechatLoginStatus\?\.connected\) return/);

  const whatsapp = source("../hooks/useWhatsappWebRuntime.ts");
  assert.match(whatsapp, /statusRequestRef/);
  assert.match(whatsapp, /if \(waLoginDialogOpen\) return/);
});

test("log discovery is server-paged instead of materializing the full directory", () => {
  const hook = source("../hooks/useLogsRuntime.ts");
  assert.match(hook, /\/v1\/logs\/files\?\$\{params\.toString\(\)\}/);
  assert.match(hook, /logFilesNextCursor/);
  assert.match(hook, /openPreviousLogFilesPage/);

  const page = source("../components/LogsPage.tsx");
  assert.match(page, /logFilesHasPrevious/);
  assert.match(page, /logFilesHasNext/);
});

test("log polling aborts page-scoped responses and coalesces duplicate reads", () => {
  const hook = source("../hooks/useLogsRuntime.ts");
  assert.match(hook, /logFilesRequestRef/);
  assert.match(hook, /logContentRequestRef/);
  assert.match(hook, /const controller = new AbortController\(\)/);
  assert.match(hook, /controller\.abort\(\)/);
  assert.match(hook, /signal\?\.aborted/);
});

test("NNI periodic reads pause while the browser document is hidden", () => {
  const text = source("../App.tsx");
  assert.match(text, /const refreshWhenVisible = \(\) => \{/);
  assert.match(text, /document\.visibilityState !== "visible"/);
  assert.match(text, /fetchNniHeartbeatRecords\(nniHeartbeatRecordsPage, true\)/);
  assert.match(text, /document\.addEventListener\("visibilitychange", refreshWhenVisible\)/);
});
