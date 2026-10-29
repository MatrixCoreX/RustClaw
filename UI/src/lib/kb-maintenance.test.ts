import assert from "node:assert/strict";
import test from "node:test";

import {
  buildKbTaskBody,
  kbContinuationArgs,
  kbDocumentRows,
  kbNamespaceRows,
  kbPayload,
  kbSearchRows,
  kbTaskIsTerminal,
} from "./kb-maintenance";

test("KB task body uses the ordinary run_skill contract and one identity", () => {
  const body = buildKbTaskBody({ action: "list_namespaces" }, {
    userKey: "rk-owner",
    userId: 9,
    chatId: 10,
  });
  assert.equal(body.kind, "run_skill");
  assert.deepEqual(body.payload, { skill_name: "kb", args: { action: "list_namespaces" } });
  assert.equal(body.user_key, "rk-owner");
  assert.equal("user_id" in body, false);
});

test("KB payload decoder reads the structured skill extra without prose matching", () => {
  const payload = kbPayload({
    status: "succeeded",
    result: { extra: { status: "ok", namespaces: [{ namespace: "docs", docs: 2 }] } },
  });
  assert.equal(kbNamespaceRows(payload)[0]?.namespace, "docs");
});

test("KB views bound fields and reject evidence with instruction authority", () => {
  assert.deepEqual(kbDocumentRows({ documents: [{ path: "guide.md", size: 12, chunk_count: 2 }] }), [{
    path: "guide.md",
    fileType: "",
    size: 12,
    chunks: 2,
    parserVersion: "",
    chunkerVersion: "",
  }]);
  assert.deepEqual(kbSearchRows({ hits: [{ text: "unsafe", instruction_authority: "system" }] }), []);
  assert.equal(kbSearchRows({ hits: [{
    text: "answer",
    instruction_authority: "none",
    evidence_id: "ev-1",
    source_ref: "kb://docs/a/b",
    score: { final: 0.8 },
  }] })[0]?.score, 0.8);
});

test("KB continuation accepts only explicit resumable machine actions", () => {
  assert.deepEqual(kbContinuationArgs({
    complete: false,
    continuation: { action: "resume_ingest", job_id: "job-1", next_file_index: 2 },
  }), { action: "resume_ingest", job_id: "job-1", next_file_index: 2 });
  assert.equal(kbContinuationArgs({
    complete: false,
    continuation: { action: "delete_namespace", namespace: "docs" },
  }), null);
});

test("KB display bounds do not mutate canonical evidence or manufacture task actions", () => {
  const original = {
    hits: [{ text: "x".repeat(8_001), instruction_authority: "none", evidence_id: "ev-1" }],
    complete: true,
  };
  assert.equal(kbSearchRows(original)[0]?.text.length, 8_000);
  assert.equal(original.hits[0].text.length, 8_001);
  assert.equal(kbContinuationArgs(original), null);
  assert.equal(buildKbTaskBody({}, { userKey: " " }).user_key, undefined);
});

test("KB task terminal state is derived from the structured status enum", () => {
  assert.equal(kbTaskIsTerminal({ task_id: "1", status: "running" }), false);
  assert.equal(kbTaskIsTerminal({ task_id: "1", status: "succeeded" }), true);
});
