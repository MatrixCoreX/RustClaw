import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { assertDocumentResponse, recordBrowserFailure } from "../src/browser_diagnostics.mjs";

test("JSON navigation failures are not empty search results or selector drift", () => {
  for (const type of ["application/json; charset=UTF-8", "application/problem+json"]) {
    assert.throws(() => assertDocumentResponse({ headers: () => ({ "content-type": type }) }, "source_navigation"),
      { message: "unexpected_page_response", discovery_stage: "source_navigation" });
  }
  assert.doesNotThrow(() => assertDocumentResponse({ headers: () => ({ "content-type": "text/html" }) }));
  assert.doesNotThrow(() => assertDocumentResponse(null));
});

test("unresponsive browser diagnostics are bounded and retain the original failure", async t => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "media-discovery-diagnostics-"));
  t.after(() => fs.rm(root, { recursive: true, force: true }));
  const error = new Error("navigation_timeout");
  const diagnostic = await recordBrowserFailure({ evaluate: () => new Promise(() => {}) },
    { root, runId: "test", platform: "xiaohongshu", stage: "source_navigation", error, timeoutMs: 20 });
  assert.equal(diagnostic.document, null);
  assert.equal(error.message, "navigation_timeout");
  assert.equal(diagnostic.stage, "source_navigation");
  assert.deepEqual(JSON.parse(await fs.readFile(path.join(root, "diagnostics/test/xiaohongshu.json"), "utf8")), diagnostic);
});
