import test from "node:test";
import assert from "node:assert/strict";

import { ClipboardWriteError, writeTextToClipboard } from "./clipboard.ts";

function fallbackEnvironment(operations: string[], copiedValues: string[]) {
  const textarea = {
    value: "",
    setAttribute: (name: string, value: string) => operations.push(`set:${name}=${value}`),
    style: {} as Record<string, string>,
    focus: () => operations.push("focus"),
    select: () => operations.push("select"),
  };

  return {
    textarea,
    environment: {
      document: {
        body: {
          appendChild: () => operations.push("append"),
          removeChild: () => operations.push("remove"),
        },
        createElement: () => textarea,
        execCommand: (command: string) => {
          operations.push(command);
          copiedValues.push(textarea.value);
          return true;
        },
      },
    },
  };
}

test("copies the exact task id without encoding or display formatting", async () => {
  const writes: string[] = [];
  const taskId = "0190c2d8-7c2a-7f45-a830-4da9f45c2d10";

  await writeTextToClipboard(taskId, {
    clipboard: {
      writeText: async (value) => {
        writes.push(value);
      },
    },
  });

  assert.deepEqual(writes, [taskId]);
});

test("falls back when the browser clipboard api rejects the write", async () => {
  const operations: string[] = [];
  const copiedValues: string[] = [];
  const { environment } = fallbackEnvironment(operations, copiedValues);

  await writeTextToClipboard("task-exact", {
    clipboard: { writeText: async () => Promise.reject(new Error("permission denied")) },
    ...environment,
  });

  assert.deepEqual(copiedValues, ["task-exact"]);
  assert.deepEqual(operations.slice(-4), ["focus", "select", "copy", "remove"]);
});

test("returns a stable machine error when no copy mechanism is available", async () => {
  await assert.rejects(
    writeTextToClipboard("task-exact", {}),
    (error: unknown) => error instanceof ClipboardWriteError && error.code === "clipboard_write_failed",
  );
});
