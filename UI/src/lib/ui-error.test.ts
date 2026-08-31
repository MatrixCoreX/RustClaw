import assert from "node:assert/strict";
import test from "node:test";

import { formatUiError } from "./ui-error";

const zh = (value: string) => value;
const en = (_zh: string, value: string) => value;

test("ordinary UI errors do not expose unknown machine tokens", () => {
  for (const token of [
    "future_error_code",
    "workspace.update.failed",
    "HTTP:UPSTREAM_FAILED",
  ]) {
    assert.equal(formatUiError(new Error(token), zh, "操作未完成。", "Operation failed."), "操作未完成。");
    assert.equal(formatUiError(token, en, "操作未完成。", "Operation failed."), "Operation failed.");
  }
});

test("ordinary UI errors preserve already presented human diagnostics", () => {
  assert.equal(
    formatUiError(new Error("The server closed the connection."), en, "操作未完成。", "Operation failed."),
    "The server closed the connection.",
  );
});
