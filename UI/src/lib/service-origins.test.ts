import assert from "node:assert/strict";
import test from "node:test";

import {
  defaultBrowserApiBaseUrl,
  defaultWebdBaseUrl,
  preferredBrowserApiBaseUrl,
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
  const deployed = location("https://agent-runtime.example.com/login");
  assert.equal(defaultBrowserApiBaseUrl(deployed), "https://agent-runtime.example.com");
  assert.equal(defaultWebdBaseUrl(deployed), "https://agent-runtime.example.com");
});

test("legacy clawd UI addresses resolve all browser traffic through webd", () => {
  const local = location("http://127.0.0.1:8787/");
  assert.equal(defaultBrowserApiBaseUrl(local), "http://127.0.0.1:8788");
  assert.equal(defaultWebdBaseUrl(local), "http://127.0.0.1:8788");
  assert.equal(
    preferredBrowserApiBaseUrl("http://127.0.0.1:8787", location("http://127.0.0.1:8788/")),
    "http://127.0.0.1:8788",
  );
});

test("local frontend development ports resolve both backend services", () => {
  const local = location("http://localhost:3000/");
  assert.equal(defaultBrowserApiBaseUrl(local), "http://localhost:8788");
  assert.equal(defaultWebdBaseUrl(local), "http://localhost:8788");
  assert.equal(
    preferredBrowserApiBaseUrl("http://localhost:3000", local),
    "http://localhost:8788",
  );
  assert.equal(
    preferredWebdBaseUrl("http://localhost:3000", local),
    "http://localhost:8788",
  );
});

test("standard HTTP ports do not gain explicit backend ports", () => {
  const deployed = location("http://agent.example.com/");
  assert.equal(defaultBrowserApiBaseUrl(deployed), "http://agent.example.com");
  assert.equal(defaultWebdBaseUrl(deployed), "http://agent.example.com");
});

test("legacy generated domain ports migrate to the current reverse-proxy origin", () => {
  const deployed = location("https://agent-runtime.example.com/");
  assert.equal(
    preferredBrowserApiBaseUrl("https://agent-runtime.example.com:8787", deployed),
    "https://agent-runtime.example.com",
  );
  assert.equal(
    preferredWebdBaseUrl("https://agent-runtime.example.com:8788", deployed),
    "https://agent-runtime.example.com",
  );
});

test("manually configured service addresses are preserved", () => {
  const deployed = location("https://agent-runtime.example.com/");
  assert.equal(
    preferredWebdBaseUrl("https://gateway.example.net", deployed),
    "https://gateway.example.net",
  );
});
