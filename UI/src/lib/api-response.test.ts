import assert from "node:assert/strict";
import test from "node:test";

import { ApiResponseFormatError, readJsonApiResponse } from "./api-response";

test("reads a JSON API response", async () => {
  const response = new Response('{"ok":true,"data":{"running":true}}', {
    headers: { "content-type": "application/json" },
  });

  assert.deepEqual(await readJsonApiResponse(response), {
    ok: true,
    data: { running: true },
  });
});

test("classifies an HTML fallback separately from malformed JSON", async () => {
  const response = new Response("<!doctype html><html></html>", {
    headers: { "content-type": "text/html" },
  });

  await assert.rejects(
    () => readJsonApiResponse(response),
    (error: unknown) =>
      error instanceof ApiResponseFormatError && error.kind === "html_response",
  );
});

test("classifies a malformed JSON API response", async () => {
  const response = new Response("not-json", {
    headers: { "content-type": "application/json" },
  });

  await assert.rejects(
    () => readJsonApiResponse(response),
    (error: unknown) =>
      error instanceof ApiResponseFormatError && error.kind === "invalid_json",
  );
});
