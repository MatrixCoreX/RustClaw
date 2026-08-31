import assert from "node:assert/strict";
import test from "node:test";

import {
  buildRuntimeRequestHeaders,
  isUnsafeHttpMethod,
  normalizeWebdCsrfToken,
  runtimeRequestCredentials,
  WEBD_CSRF_HEADER,
} from "./webd-csrf";

const TOKEN = "ab".repeat(16);

test("webd CSRF tokens use a fixed canonical envelope", () => {
  assert.equal(normalizeWebdCsrfToken(TOKEN), TOKEN);
  assert.equal(normalizeWebdCsrfToken(TOKEN.toUpperCase()), null);
  assert.equal(normalizeWebdCsrfToken("01"), null);
  assert.equal(normalizeWebdCsrfToken(null), null);
});

test("unsafe Cookie-session requests carry the in-memory CSRF token", () => {
  const headers = buildRuntimeRequestHeaders({
    initialHeaders: { "Content-Type": "application/json" },
    directAuthHeaders: { "x-agent-key": "rk-must-not-be-forwarded" },
    withAuth: true,
    authMode: "webd",
    method: "POST",
    csrfToken: TOKEN,
  });
  assert.equal(headers.get(WEBD_CSRF_HEADER), TOKEN);
  assert.equal(headers.get("x-agent-key"), null);
  assert.equal(headers.get("content-type"), "application/json");
});

test("safe Cookie reads and direct-key mutations keep separate auth contracts", () => {
  assert.equal(isUnsafeHttpMethod("GET"), false);
  assert.equal(isUnsafeHttpMethod("DELETE"), true);

  const safeHeaders = buildRuntimeRequestHeaders({
    directAuthHeaders: {},
    withAuth: true,
    authMode: "webd",
    method: "GET",
    csrfToken: TOKEN,
  });
  assert.equal(safeHeaders.get(WEBD_CSRF_HEADER), null);

  const directHeaders = buildRuntimeRequestHeaders({
    directAuthHeaders: { "x-agent-key": "rk-direct" },
    withAuth: true,
    authMode: "key",
    method: "POST",
    csrfToken: "",
  });
  assert.equal(directHeaders.get("x-agent-key"), "rk-direct");
  assert.equal(directHeaders.get(WEBD_CSRF_HEADER), null);
});

test("only authenticated Cookie mode sends browser credentials", () => {
  assert.equal(runtimeRequestCredentials(true, "webd"), "include");
  assert.equal(runtimeRequestCredentials(true, "key"), "omit");
  assert.equal(runtimeRequestCredentials(false, "webd"), "omit");
  assert.equal(runtimeRequestCredentials(false, null, "same-origin"), "same-origin");
});
