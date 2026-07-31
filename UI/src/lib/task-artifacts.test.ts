import assert from "node:assert/strict";
import test from "node:test";

import {
  artifactPreviewKind,
  extractTaskArtifacts,
  normalizeTaskArtifacts,
  safeArtifactUrl,
} from "./task-artifacts";
import type { TaskArtifact, TaskQueryResponse } from "../types/api";

function artifact(overrides: Partial<TaskArtifact> = {}): TaskArtifact {
  return {
    schema_version: 2,
    id: "artifact-1",
    artifact_ref: "artifact:task/task-1/artifact-1",
    filename: "report.pdf",
    kind: "pdf",
    mime_type: "application/pdf",
    size_bytes: 42,
    sha256: "a".repeat(64),
    download_url: "/v1/tasks/task-1/artifacts/artifact-1/content",
    preview_url: "/v1/tasks/task-1/artifacts/artifact-1/content?disposition=inline",
    ...overrides,
  };
}

test("extracts validated task artifacts without interpreting assistant text", () => {
  const result: TaskQueryResponse = {
    task_id: "task-1",
    status: "succeeded",
    result_json: { text: "download words are not parsed", artifacts: [artifact()] },
    error_text: null,
  };

  assert.deepEqual(extractTaskArtifacts(result), [artifact()]);
});

test("rejects external, traversal, malformed, and duplicate artifact records", () => {
  assert.deepEqual(
    normalizeTaskArtifacts([
      artifact(),
      artifact(),
      artifact({ id: "../escape" }),
      artifact({ id: "external", download_url: "https://example.com/report.pdf" }),
      artifact({ id: "broken", sha256: "bad" }),
    ]),
    [artifact()],
  );
  assert.equal(safeArtifactUrl("//example.com/v1/tasks/a/artifacts/b/content"), false);
});

test("normalizes legacy artifacts and rejects mismatched canonical references", () => {
  const legacy = { ...artifact(), schema_version: 1 as const };
  delete (legacy as Partial<TaskArtifact>).artifact_ref;
  assert.deepEqual(normalizeTaskArtifacts([legacy]), [artifact()]);
  assert.deepEqual(
    normalizeTaskArtifacts([artifact({ artifact_ref: "artifact:task/other/artifact-1" })]),
    [],
  );
  assert.deepEqual(
    normalizeTaskArtifacts([
      artifact({ download_url: "/v1/tasks/task-1/artifacts/%ZZ/content" }),
    ]),
    [],
  );
});

test("allows only server-approved inline preview media", () => {
  assert.equal(artifactPreviewKind(artifact()), "pdf");
  assert.equal(
    artifactPreviewKind(artifact({ mime_type: "image/png", kind: "image" })),
    "image",
  );
  assert.equal(
    artifactPreviewKind(artifact({ mime_type: "image/svg+xml", kind: "image" })),
    "none",
  );
  assert.equal(artifactPreviewKind(artifact({ preview_url: null })), "none");
});
