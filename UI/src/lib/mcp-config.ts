import type { McpConfigResponse, McpServerConfigItem } from "../types/api";

type Translate = (zh: string, en: string) => string;

export type McpAuthMode = "none" | "bearer_env" | "oauth_client_credentials";

export interface McpServerDraft {
  originalServerId: string | null;
  serverId: string;
  enabled: boolean;
  trusted: boolean;
  transport: "stdio" | "streamable_http";
  command: string;
  argsText: string;
  envRefsText: string;
  url: string;
  authMode: McpAuthMode;
  authTokenEnv: string;
  oauthClientIdEnv: string;
  oauthClientSecretEnv: string;
  oauthScopesText: string;
  oauthResource: string;
  allowedToolsText: string;
  hasStaticEnv: boolean;
  hasAdvancedPolicy: boolean;
}

export interface McpConfigDraft {
  enabled: boolean;
  servers: McpServerDraft[];
}

export interface McpServerUpdatePayload {
  server_id: string;
  enabled: boolean;
  trusted: boolean;
  transport: "stdio" | "streamable_http";
  command?: string;
  args: string[];
  env_refs: Record<string, string>;
  url?: string;
  auth_token_env?: string;
  oauth_client_id_env?: string;
  oauth_client_secret_env?: string;
  oauth_scopes: string[];
  oauth_resource?: string;
  allowed_tools: string[];
}

export interface McpConfigUpdatePayload {
  enabled: boolean;
  servers: McpServerUpdatePayload[];
}

function optional(value: string): string | undefined {
  const trimmed = value.trim();
  return trimmed || undefined;
}

export function splitMcpLines(value: string): string[] {
  return [...new Set(value.split(/\r?\n/).map((line) => line.trim()).filter(Boolean))];
}

export function formatMcpEnvRefs(refs: Record<string, string>): string {
  return Object.entries(refs)
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([name, envRef]) => `${name}=${envRef}`)
    .join("\n");
}

export function parseMcpEnvRefs(value: string): Record<string, string> {
  const refs: Record<string, string> = {};
  for (const line of splitMcpLines(value)) {
    const separator = line.indexOf("=");
    if (separator <= 0 || separator === line.length - 1) {
      throw new Error("mcp_stdio_env_ref_line_invalid");
    }
    const name = line.slice(0, separator).trim();
    const envRef = line.slice(separator + 1).trim();
    if (!name || !envRef || Object.hasOwn(refs, name)) {
      throw new Error("mcp_stdio_env_ref_line_invalid");
    }
    refs[name] = envRef;
  }
  return refs;
}

function authModeFor(server: McpServerConfigItem): McpAuthMode {
  if (server.oauth_client_id_env || server.oauth_client_secret_env) {
    return "oauth_client_credentials";
  }
  if (server.auth_token_env) return "bearer_env";
  return "none";
}

function serverDraft(server: McpServerConfigItem): McpServerDraft {
  return {
    originalServerId: server.server_id,
    serverId: server.server_id,
    enabled: server.enabled,
    trusted: server.trusted,
    transport: server.transport === "streamable_http" ? "streamable_http" : "stdio",
    command: server.command ?? "",
    argsText: server.args.join("\n"),
    envRefsText: formatMcpEnvRefs(server.env_refs),
    url: server.url ?? "",
    authMode: authModeFor(server),
    authTokenEnv: server.auth_token_env ?? "",
    oauthClientIdEnv: server.oauth_client_id_env ?? "",
    oauthClientSecretEnv: server.oauth_client_secret_env ?? "",
    oauthScopesText: server.oauth_scopes.join("\n"),
    oauthResource: server.oauth_resource ?? "",
    allowedToolsText: server.allowed_tools.join("\n"),
    hasStaticEnv: server.has_static_env,
    hasAdvancedPolicy: server.has_advanced_policy,
  };
}

export function buildMcpConfigDraft(config: McpConfigResponse): McpConfigDraft {
  return {
    enabled: config.enabled,
    servers: config.servers.map(serverDraft),
  };
}

export function newMcpServerDraft(existing: McpServerDraft[]): McpServerDraft {
  const used = new Set(existing.map((server) => server.serverId));
  let sequence = 1;
  while (used.has(`server_${sequence}`)) sequence += 1;
  return {
    originalServerId: null,
    serverId: `server_${sequence}`,
    enabled: false,
    trusted: false,
    transport: "stdio",
    command: "",
    argsText: "",
    envRefsText: "",
    url: "",
    authMode: "none",
    authTokenEnv: "",
    oauthClientIdEnv: "",
    oauthClientSecretEnv: "",
    oauthScopesText: "",
    oauthResource: "",
    allowedToolsText: "",
    hasStaticEnv: false,
    hasAdvancedPolicy: false,
  };
}

function serverPayload(server: McpServerDraft): McpServerUpdatePayload {
  const base = {
    server_id: server.serverId.trim(),
    enabled: server.enabled,
    trusted: server.trusted,
    transport: server.transport,
    allowed_tools: splitMcpLines(server.allowedToolsText),
  };
  if (server.transport === "stdio") {
    return {
      ...base,
      command: optional(server.command),
      args: splitMcpLines(server.argsText),
      env_refs: parseMcpEnvRefs(server.envRefsText),
      oauth_scopes: [],
    };
  }
  return {
    ...base,
    args: [],
    env_refs: {},
    url: optional(server.url),
    auth_token_env: server.authMode === "bearer_env" ? optional(server.authTokenEnv) : undefined,
    oauth_client_id_env:
      server.authMode === "oauth_client_credentials" ? optional(server.oauthClientIdEnv) : undefined,
    oauth_client_secret_env:
      server.authMode === "oauth_client_credentials" ? optional(server.oauthClientSecretEnv) : undefined,
    oauth_scopes:
      server.authMode === "oauth_client_credentials" ? splitMcpLines(server.oauthScopesText) : [],
    oauth_resource:
      server.authMode === "oauth_client_credentials" ? optional(server.oauthResource) : undefined,
  };
}

export function buildMcpConfigUpdatePayload(draft: McpConfigDraft): McpConfigUpdatePayload {
  return {
    enabled: draft.enabled,
    servers: draft.servers.map(serverPayload).sort((left, right) => left.server_id.localeCompare(right.server_id)),
  };
}

export function hasUnsavedMcpChanges(config: McpConfigResponse | null, draft: McpConfigDraft): boolean {
  if (!config) return false;
  try {
    return JSON.stringify(buildMcpConfigUpdatePayload(buildMcpConfigDraft(config))) !== JSON.stringify(buildMcpConfigUpdatePayload(draft));
  } catch {
    return true;
  }
}

export function mcpLifecycleLabel(state: string, t: Translate): string {
  switch (state) {
    case "ready":
      return t("可用", "Ready");
    case "starting":
      return t("正在连接", "Connecting");
    case "degraded":
      return t("需要处理", "Needs attention");
    case "disabled":
      return t("未启用", "Disabled");
    case "stopped":
      return t("已停止", "Stopped");
    default:
      return t("未知状态", "Unknown status");
  }
}

export function mcpErrorLabel(errorCode: string, t: Translate): string {
  switch (errorCode) {
    case "mcp_admin_required":
      return t("需要管理员权限。", "Administrator access is required.");
    case "mcp_config_read_failed":
      return t("无法读取 MCP 配置。", "Could not read MCP configuration.");
    case "mcp_config_write_failed":
      return t("无法保存 MCP 配置。", "Could not save MCP configuration.");
    case "mcp_server_untrusted":
      return t("启用服务器前需要确认信任。", "Confirm trust before enabling this server.");
    case "mcp_stdio_command_missing":
      return t("请填写启动命令。", "Enter the startup command.");
    case "mcp_http_url_missing":
      return t("请填写服务器地址。", "Enter the server URL.");
    case "mcp_auth_token_ref_invalid":
    case "mcp_oauth_secret_ref_invalid":
    case "mcp_stdio_env_ref_invalid":
    case "mcp_stdio_env_ref_line_invalid":
      return t("环境变量引用格式不正确，只填写变量名，不填写密钥值。", "The environment reference is invalid. Enter variable names, not secret values.");
    case "mcp_server_id_duplicate":
      return t("服务器名称不能重复。", "Server names must be unique.");
    case "mcp_capability_prefix_duplicate":
      return t("服务器能力命名空间冲突。", "Server capability namespaces conflict.");
    default:
      return `${t("操作未完成", "The operation did not complete")} (${errorCode})`;
  }
}
