import assert from "node:assert/strict";
import test from "node:test";

import {
  defaultClawdBaseUrl,
  defaultWebdBaseUrl,
  preferredClawdBaseUrl,
  preferredWebdBaseUrl,
  type BrowserLocation,
} from "./service-origins";

function location(href: string): BrowserLocation {
  const url = new URL(href);
  return {
    href: url.href,
    hostname: url.hostname,
    port: url.port,
    protocol: url.protocol,
  };
}

test("domain deployments use the current origin without backend ports", () => {
  const deployed = location("https://rustclaw.example.com/login");
  assert.equal(defaultClawdBaseUrl(deployed), "https://rustclaw.example.com");
  assert.equal(defaultWebdBaseUrl(deployed), "https://rustclaw.example.com");
});

test("direct clawd UI keeps clawd origin and points webd login to 8788", () => {
  const local = location("http://127.0.0.1:8787/");
  assert.equal(defaultClawdBaseUrl(local), "http://127.0.0.1:8787");
  assert.equal(defaultWebdBaseUrl(local), "http://127.0.0.1:8788");
});

test("local frontend development ports resolve both backend services", () => {
  const local = location("http://localhost:3000/");
  assert.equal(defaultClawdBaseUrl(local), "http://localhost:8787");
  assert.equal(defaultWebdBaseUrl(local), "http://localhost:8788");
});

test("standard HTTP ports do not gain explicit backend ports", () => {
  const deployed = location("http://agent.example.com/");
  assert.equal(defaultClawdBaseUrl(deployed), "http://agent.example.com");
  assert.equal(defaultWebdBaseUrl(deployed), "http://agent.example.com");
});

test("legacy generated domain ports migrate to the current reverse-proxy origin", () => {
  const deployed = location("https://rustclaw.example.com/");
  assert.equal(
    preferredClawdBaseUrl("https://rustclaw.example.com:8787", deployed),
    "https://rustclaw.example.com",
  );
  assert.equal(
    preferredWebdBaseUrl("https://rustclaw.example.com:8788", deployed),
    "https://rustclaw.example.com",
  );
});

test("manually configured service addresses are preserved", () => {
  const deployed = location("https://rustclaw.example.com/");
  assert.equal(
    preferredWebdBaseUrl("https://gateway.example.net", deployed),
    "https://gateway.example.net",
  );
});
