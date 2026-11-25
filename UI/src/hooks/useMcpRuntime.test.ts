import assert from "node:assert/strict";
import test from "node:test";
import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";

import type { McpConfigResponse } from "../types/api";
import { useMcpRuntime } from "./useMcpRuntime";

test("adds new MCP server forms above saved servers and prior drafts", async (context) => {
  const config: McpConfigResponse = {
    config_path: "configs/config.toml",
    enabled: true,
    restart_required: false,
    servers: [{
      server_id: "existing_tools",
      enabled: true,
      trusted: true,
      transport: "stdio",
      command: "tool-server",
      args: [],
      env_refs: {},
      oauth_scopes: [],
      allowed_tools: ["lookup"],
      has_static_env: false,
      has_advanced_policy: false,
    }],
  };
  let runtime!: ReturnType<typeof useMcpRuntime>;
  function Probe() {
    runtime = useMcpRuntime({
      apiFetch: async (path) => Response.json({
        ok: true,
        data: path.endsWith("/config") ? config : { servers: [], tools: [] },
      }),
      t: (zh) => zh,
    });
    return null;
  }

  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  let renderer!: ReactTestRenderer;
  await act(async () => { renderer = create(createElement(Probe)); });
  context.after(() => act(async () => { renderer.unmount(); }));

  await act(async () => { await runtime.refreshMcp(); });
  await act(async () => { runtime.addMcpServer(); });
  await act(async () => { runtime.updateMcpServer(0, { command: "draft-command" }); });
  await act(async () => { runtime.addMcpServer(); });

  assert.deepEqual(
    runtime.mcpDraft.servers.map(({ serverId, command }) => [serverId, command]),
    [["server_2", ""], ["server_1", "draft-command"], ["existing_tools", "tool-server"]],
  );
});
