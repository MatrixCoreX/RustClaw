import assert from "node:assert/strict";
import test from "node:test";

import { taskReportReturnTab } from "../components/TasksPage.tsx";

test("task reports return to the list tab that opened them", () => {
  assert.equal(taskReportReturnTab("active"), "active");
  assert.equal(taskReportReturnTab("history"), "history");
  assert.equal(taskReportReturnTab("manual"), "manual");
});

test("nested report navigation keeps a stable list fallback", () => {
  assert.equal(taskReportReturnTab("report"), "active");
});
