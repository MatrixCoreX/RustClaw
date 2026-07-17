import { useMemo, useState } from "react";

import {
  buildMcpConfigDraft,
  buildMcpConfigUpdatePayload,
  hasUnsavedMcpChanges,
  mcpErrorLabel,
  newMcpServerDraft,
  type McpConfigDraft,
  type McpServerDraft,
} from "../lib/mcp-config";
import type {
  ApiResponse,
  McpConfigResponse,
  McpLifecycleSnapshot,
  McpProbeOutcome,
  McpToolSummary,
} from "../types/api";

type Translate = (zh: string, en: string) => string;
type ApiFetch = (path: string, init?: RequestInit) => Promise<Response>;

export interface UseMcpRuntimeParams {
  apiFetch: ApiFetch;
  t: Translate;
}

const EMPTY_DRAFT: McpConfigDraft = { enabled: false, servers: [] };

export function useMcpRuntime({ apiFetch, t }: UseMcpRuntimeParams) {
  const [mcpConfig, setMcpConfig] = useState<McpConfigResponse | null>(null);
  const [mcpDraft, setMcpDraft] = useState<McpConfigDraft>(EMPTY_DRAFT);
  const [mcpLifecycle, setMcpLifecycle] = useState<McpLifecycleSnapshot[]>([]);
  const [mcpTools, setMcpTools] = useState<McpToolSummary[]>([]);
  const [mcpLoading, setMcpLoading] = useState(false);
  const [mcpSaving, setMcpSaving] = useState(false);
  const [mcpTestingServerId, setMcpTestingServerId] = useState<string | null>(null);
  const [mcpError, setMcpError] = useState<string | null>(null);
  const [mcpSaveMessage, setMcpSaveMessage] = useState<string | null>(null);
  const [mcpProbeResults, setMcpProbeResults] = useState<Record<string, McpProbeOutcome>>({});

  const hasUnsavedMcpChangesValue = useMemo(
    () => hasUnsavedMcpChanges(mcpConfig, mcpDraft),
    [mcpConfig, mcpDraft],
  );

  const displayError = (error: unknown) => {
    const code = error instanceof Error ? error.message : "mcp_unknown_error";
    return mcpErrorLabel(code, t);
  };

  const readResponse = async <T,>(res: Response): Promise<T> => {
    const body = (await res.json()) as ApiResponse<T>;
    if (!res.ok || !body.ok || !body.data) {
      throw new Error(body.error || `mcp_http_${res.status}`);
    }
    return body.data;
  };

  const fetchMcpConfig = async () => {
    const data = await readResponse<McpConfigResponse>(await apiFetch("/v1/admin/mcp/config"));
    setMcpConfig(data);
    setMcpDraft(buildMcpConfigDraft(data));
    return data;
  };

  const fetchMcpStatus = async () => {
    const [serversResponse, toolsResponse] = await Promise.all([
      apiFetch("/v1/admin/mcp/servers"),
      apiFetch("/v1/admin/mcp/tools"),
    ]);
    const [servers, tools] = await Promise.all([
      readResponse<{ servers: McpLifecycleSnapshot[] }>(serversResponse),
      readResponse<{ tools: McpToolSummary[] }>(toolsResponse),
    ]);
    setMcpLifecycle(servers.servers);
    setMcpTools(tools.tools);
  };

  const refreshMcp = async () => {
    setMcpLoading(true);
    setMcpError(null);
    try {
      await Promise.all([fetchMcpConfig(), fetchMcpStatus()]);
    } catch (error) {
      setMcpError(displayError(error));
    } finally {
      setMcpLoading(false);
    }
  };

  const saveMcpConfig = async () => {
    setMcpSaving(true);
    setMcpError(null);
    setMcpSaveMessage(null);
    try {
      const payload = buildMcpConfigUpdatePayload(mcpDraft);
      const data = await readResponse<McpConfigResponse>(
        await apiFetch("/v1/admin/mcp/config", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify(payload),
        }),
      );
      setMcpConfig(data);
      setMcpDraft(buildMcpConfigDraft(data));
      setMcpSaveMessage(
        t(
          "MCP 设置已保存。重启 RustClaw 后会连接服务器。",
          "MCP settings saved. Restart RustClaw to connect the servers.",
        ),
      );
    } catch (error) {
      setMcpError(displayError(error));
    } finally {
      setMcpSaving(false);
    }
  };

  const testMcpServer = async (serverId: string) => {
    setMcpTestingServerId(serverId);
    setMcpError(null);
    try {
      const data = await readResponse<{ probe: McpProbeOutcome }>(
        await apiFetch(`/v1/admin/mcp/servers/${encodeURIComponent(serverId)}/test`, {
          method: "POST",
        }),
      );
      setMcpProbeResults((current) => ({ ...current, [serverId]: data.probe }));
      await fetchMcpStatus();
    } catch (error) {
      setMcpError(displayError(error));
    } finally {
      setMcpTestingServerId(null);
    }
  };

  const setMcpEnabled = (enabled: boolean) => {
    setMcpDraft((current) => ({ ...current, enabled }));
    setMcpSaveMessage(null);
  };

  const updateMcpServer = (index: number, patch: Partial<McpServerDraft>) => {
    setMcpDraft((current) => ({
      ...current,
      servers: current.servers.map((server, serverIndex) =>
        serverIndex === index ? { ...server, ...patch } : server,
      ),
    }));
    setMcpSaveMessage(null);
  };

  const addMcpServer = () => {
    setMcpDraft((current) => ({
      ...current,
      servers: [...current.servers, newMcpServerDraft(current.servers)],
    }));
    setMcpSaveMessage(null);
  };

  const removeMcpServer = (index: number) => {
    setMcpDraft((current) => ({
      ...current,
      servers: current.servers.filter((_server, serverIndex) => serverIndex !== index),
    }));
    setMcpSaveMessage(null);
  };

  return {
    mcpConfig,
    mcpDraft,
    mcpLifecycle,
    mcpTools,
    mcpLoading,
    mcpSaving,
    mcpTestingServerId,
    mcpError,
    mcpSaveMessage,
    mcpProbeResults,
    hasUnsavedMcpChanges: hasUnsavedMcpChangesValue,
    refreshMcp,
    saveMcpConfig,
    testMcpServer,
    setMcpEnabled,
    updateMcpServer,
    addMcpServer,
    removeMcpServer,
  };
}
