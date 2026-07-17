import test from "node:test";
import assert from "node:assert/strict";

import type { McpConfigResponse } from "../types/api";
import {
  buildMcpConfigDraft,
  buildMcpConfigUpdatePayload,
  formatMcpEnvRefs,
  hasUnsavedMcpChanges,
  mcpErrorLabel,
  newMcpServerDraft,
  parseMcpEnvRefs,
} from "./mcp-config.ts";

function configFixture(): McpConfigResponse {
  return {
    config_path: "configs/config.toml",
    enabled: true,
    restart_required: false,
    servers: [
      {
        server_id: "fixture",
        enabled: true,
        trusted: true,
        transport: "stdio",
        command: "/usr/bin/fixture",
        args: ["--stdio"],
        env_refs: { CHILD_TOKEN: "RUSTCLAW_MCP_TOKEN" },
        oauth_scopes: [],
        allowed_tools: ["lookup"],
        has_static_env: true,
        has_advanced_policy: true,
      },
    ],
  };
}

test("builds an MCP draft and saves only secret reference names", () => {
  const config = configFixture();
  const draft = buildMcpConfigDraft(config);
  const payload = buildMcpConfigUpdatePayload(draft);
  assert.equal(payload.servers[0].env_refs.CHILD_TOKEN, "RUSTCLAW_MCP_TOKEN");
  assert.equal(JSON.stringify(payload).includes("secret_value"), false);
  assert.equal(hasUnsavedMcpChanges(config, draft), false);
});

test("normalizes MCP line fields and detects draft changes", () => {
  const config = configFixture();
  const draft = buildMcpConfigDraft(config);
  draft.servers[0].allowedToolsText = " lookup \nlookup\nsearch ";
  const payload = buildMcpConfigUpdatePayload(draft);
  assert.deepEqual(payload.servers[0].allowed_tools, ["lookup", "search"]);
  assert.equal(hasUnsavedMcpChanges(config, draft), true);
});

test("parses environment mappings and rejects incomplete lines", () => {
  assert.deepEqual(parseMcpEnvRefs("B=HOST_B\nA=HOST_A"), { B: "HOST_B", A: "HOST_A" });
  assert.equal(formatMcpEnvRefs({ B: "HOST_B", A: "HOST_A" }), "A=HOST_A\nB=HOST_B");
  assert.throws(() => parseMcpEnvRefs("TOKEN"), /mcp_stdio_env_ref_line_invalid/);
});

test("creates a disabled, untrusted server with a unique machine id", () => {
  const existing = buildMcpConfigDraft(configFixture()).servers;
  existing.push({ ...newMcpServerDraft(existing), serverId: "server_1" });
  const created = newMcpServerDraft(existing);
  assert.equal(created.serverId, "server_2");
  assert.equal(created.enabled, false);
  assert.equal(created.trusted, false);
});

test("maps machine errors to beginner-facing keyed UI copy", () => {
  const en = (_zh: string, text: string) => text;
  assert.equal(mcpErrorLabel("mcp_server_untrusted", en), "Confirm trust before enabling this server.");
  assert.match(mcpErrorLabel("mcp_unknown", en), /mcp_unknown/);
});
