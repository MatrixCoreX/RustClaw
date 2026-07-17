import {
  CircleAlert,
  CircleCheck,
  Loader2,
  Plus,
  RefreshCw,
  Save,
  ShieldCheck,
  Trash2,
  Unplug,
} from "lucide-react";

import { mcpErrorLabel, mcpLifecycleLabel, type McpConfigDraft, type McpServerDraft } from "../lib/mcp-config";
import type { McpConfigResponse, McpLifecycleSnapshot, McpProbeOutcome, McpToolSummary } from "../types/api";

type Translate = (zh: string, en: string) => string;

export interface McpConfigSectionProps {
  t: Translate;
  canManage: boolean;
  config: McpConfigResponse | null;
  draft: McpConfigDraft;
  lifecycle: McpLifecycleSnapshot[];
  tools: McpToolSummary[];
  loading: boolean;
  saving: boolean;
  testingServerId: string | null;
  error: string | null;
  saveMessage: string | null;
  probeResults: Record<string, McpProbeOutcome>;
  hasUnsavedChanges: boolean;
  onRefresh: () => unknown | Promise<unknown>;
  onSave: () => unknown | Promise<unknown>;
  onTestServer: (serverId: string) => unknown | Promise<unknown>;
  onEnabledChange: (enabled: boolean) => void;
  onServerChange: (index: number, patch: Partial<McpServerDraft>) => void;
  onAddServer: () => void;
  onRemoveServer: (index: number) => void;
}

function stateTone(state: string | undefined): string {
  if (state === "ready") return "border-emerald-400/30 bg-emerald-500/10 text-emerald-100";
  if (state === "degraded") return "border-amber-400/30 bg-amber-500/10 text-amber-100";
  return "border-white/10 bg-white/5 text-white/60";
}

export function McpConfigSection({
  t,
  canManage,
  config,
  draft,
  lifecycle,
  tools,
  loading,
  saving,
  testingServerId,
  error,
  saveMessage,
  probeResults,
  hasUnsavedChanges,
  onRefresh,
  onSave,
  onTestServer,
  onEnabledChange,
  onServerChange,
  onAddServer,
  onRemoveServer,
}: McpConfigSectionProps) {
  if (!canManage) return null;

  return (
    <section className="mt-4 scroll-mt-20 border-t border-white/10 pt-5">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="max-w-2xl">
          <p className="theme-kicker text-[10px] uppercase tracking-normal">MCP</p>
          <h2 className="mt-1 text-lg font-semibold">{t("外部工具服务器", "External tool servers")}</h2>
          <p className="mt-2 text-sm leading-6 text-white/60">
            {t(
              "连接可信的本地或远程工具。密钥只填写环境变量名；RustClaw 不会在此页面读取或显示变量值。",
              "Connect trusted local or remote tools. Enter environment variable names only; RustClaw does not read or display their values here.",
            )}
          </p>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <button
            type="button"
            className="theme-secondary-btn px-3 py-2 text-xs"
            onClick={() => void onRefresh()}
            disabled={loading || saving}
          >
            {loading ? <Loader2 className="h-4 w-4 animate-spin" /> : <RefreshCw className="h-4 w-4" />}
            {t("刷新状态", "Refresh status")}
          </button>
          <button
            type="button"
            className="theme-secondary-btn px-3 py-2 text-xs"
            onClick={onAddServer}
            disabled={loading || saving}
          >
            <Plus className="h-4 w-4" />
            {t("添加服务器", "Add server")}
          </button>
          <button
            type="button"
            className="theme-accent-btn px-3 py-2 text-xs"
            onClick={() => void onSave()}
            disabled={loading || saving || !hasUnsavedChanges}
          >
            {saving ? <Loader2 className="h-4 w-4 animate-spin" /> : <Save className="h-4 w-4" />}
            {t("保存 MCP 设置", "Save MCP settings")}
          </button>
        </div>
      </div>

      <label className="mt-5 flex min-h-12 items-center justify-between gap-4 border-y border-white/10 py-3">
        <span>
          <span className="block text-sm font-medium">{t("启用 MCP 工具", "Enable MCP tools")}</span>
          <span className="mt-1 block text-xs text-white/50">
            {t("关闭时不会启动任何 MCP 服务器。", "No MCP server starts while this is off.")}
          </span>
        </span>
        <input
          type="checkbox"
          className="h-5 w-5 accent-emerald-500"
          checked={draft.enabled}
          onChange={(event) => onEnabledChange(event.target.checked)}
        />
      </label>

      {error ? (
        <p className="mt-4 flex items-start gap-2 rounded-md border border-red-500/30 bg-red-500/10 px-3 py-2 text-sm text-red-100">
          <CircleAlert className="mt-0.5 h-4 w-4 shrink-0" />
          {error}
        </p>
      ) : null}
      {saveMessage ? (
        <p className="mt-4 flex items-start gap-2 rounded-md border border-emerald-500/30 bg-emerald-500/10 px-3 py-2 text-sm text-emerald-100">
          <CircleCheck className="mt-0.5 h-4 w-4 shrink-0" />
          {saveMessage}
        </p>
      ) : null}
      {config?.restart_required ? (
        <p className="mt-4 rounded-md border border-amber-400/30 bg-amber-500/10 px-3 py-2 text-sm text-amber-100">
          {t("新设置会在重启 RustClaw 后生效。", "The new settings take effect after RustClaw restarts.")}
        </p>
      ) : null}

      <div className="mt-5 divide-y divide-white/10 border-y border-white/10">
        {draft.servers.length === 0 ? (
          <div className="flex min-h-28 flex-col items-center justify-center px-4 py-6 text-center">
            <Unplug className="h-6 w-6 text-white/35" />
            <p className="mt-2 text-sm font-medium">{t("还没有工具服务器", "No tool servers yet")}</p>
            <p className="mt-1 text-xs text-white/50">
              {t("添加后先保持关闭，检查连接信息，再确认信任并启用。", "Add one while disabled, review its connection, then confirm trust and enable it.")}
            </p>
          </div>
        ) : null}

        {draft.servers.map((server, index) => {
          const runtime = lifecycle.find((item) => item.server_id === server.originalServerId);
          const serverTools = tools.filter((tool) => tool.server_id === server.originalServerId);
          const probe = server.originalServerId ? probeResults[server.originalServerId] : undefined;
          const authMode = server.authMode;
          return (
            <fieldset key={`${server.originalServerId ?? "new"}-${index}`} className="py-5">
              <div className="flex flex-wrap items-start justify-between gap-3">
                <div className="min-w-0 flex-1">
                  <label className="block max-w-md space-y-2">
                    <span className="text-xs uppercase tracking-normal text-white/50">{t("服务器名称", "Server name")}</span>
                    <input
                      className="theme-input font-mono"
                      value={server.serverId}
                      disabled={server.originalServerId !== null}
                      onChange={(event) => onServerChange(index, { serverId: event.target.value })}
                      placeholder="local_tools"
                    />
                  </label>
                </div>
                <div className="flex items-center gap-2">
                  <span className={`rounded-md border px-2 py-1 text-xs ${stateTone(runtime?.state)}`}>
                    {hasUnsavedChanges
                      ? t("等待保存", "Pending save")
                      : config?.restart_required
                        ? t("等待重启", "Pending restart")
                        : mcpLifecycleLabel(runtime?.state ?? "unknown", t)}
                  </span>
                  <button
                    type="button"
                    className="theme-icon-btn h-9 w-9 text-red-200"
                    title={t("从草稿中移除", "Remove from draft")}
                    aria-label={t("从草稿中移除服务器", "Remove server from draft")}
                    onClick={() => onRemoveServer(index)}
                  >
                    <Trash2 className="h-4 w-4" />
                  </button>
                </div>
              </div>

              <div className="mt-4 grid gap-4 md:grid-cols-2">
                <label className="flex min-h-12 items-center justify-between gap-3 rounded-md border border-white/10 px-3 py-2">
                  <span className="text-sm">{t("启用此服务器", "Enable this server")}</span>
                  <input
                    type="checkbox"
                    className="h-5 w-5 accent-emerald-500"
                    checked={server.enabled}
                    onChange={(event) => onServerChange(index, { enabled: event.target.checked })}
                  />
                </label>
                <label className="flex min-h-12 items-center justify-between gap-3 rounded-md border border-white/10 px-3 py-2">
                  <span>
                    <span className="flex items-center gap-2 text-sm">
                      <ShieldCheck className="h-4 w-4 text-amber-200" />
                      {t("我信任此服务器", "I trust this server")}
                    </span>
                    <span className="mt-1 block text-xs text-white/45">
                      {t("仅允许下方列出的工具。", "Only the tools listed below are exposed.")}
                    </span>
                  </span>
                  <input
                    type="checkbox"
                    className="h-5 w-5 accent-amber-400"
                    checked={server.trusted}
                    onChange={(event) => onServerChange(index, { trusted: event.target.checked })}
                  />
                </label>
              </div>

              <div className="mt-4 grid gap-4 md:grid-cols-2">
                <label className="block space-y-2">
                  <span className="text-xs uppercase tracking-normal text-white/50">{t("连接方式", "Connection")}</span>
                  <select
                    className="theme-input"
                    value={server.transport}
                    onChange={(event) => onServerChange(index, { transport: event.target.value as McpServerDraft["transport"] })}
                  >
                    <option value="stdio">{t("本机命令 (stdio)", "Local command (stdio)")}</option>
                    <option value="streamable_http">HTTP</option>
                  </select>
                </label>
                {server.transport === "stdio" ? (
                  <label className="block space-y-2">
                    <span className="text-xs uppercase tracking-normal text-white/50">{t("启动命令", "Startup command")}</span>
                    <input
                      className="theme-input font-mono"
                      value={server.command}
                      onChange={(event) => onServerChange(index, { command: event.target.value })}
                      placeholder="/usr/local/bin/tool-server"
                    />
                  </label>
                ) : (
                  <label className="block space-y-2">
                    <span className="text-xs uppercase tracking-normal text-white/50">{t("服务器地址", "Server URL")}</span>
                    <input
                      className="theme-input font-mono"
                      value={server.url}
                      onChange={(event) => onServerChange(index, { url: event.target.value })}
                      placeholder="https://tools.example.com/mcp"
                    />
                  </label>
                )}
              </div>

              <div className="mt-4 grid gap-4 md:grid-cols-2">
                {server.transport === "stdio" ? (
                  <label className="block space-y-2">
                    <span className="text-xs uppercase tracking-normal text-white/50">{t("命令参数", "Command arguments")}</span>
                    <textarea
                      className="theme-input min-h-24 resize-y font-mono"
                      value={server.argsText}
                      onChange={(event) => onServerChange(index, { argsText: event.target.value })}
                      placeholder={t("每行一个参数", "One argument per line")}
                    />
                  </label>
                ) : (
                  <label className="block space-y-2">
                    <span className="text-xs uppercase tracking-normal text-white/50">{t("认证方式", "Authentication")}</span>
                    <select
                      className="theme-input"
                      value={authMode}
                      onChange={(event) => onServerChange(index, { authMode: event.target.value as McpServerDraft["authMode"] })}
                    >
                      <option value="none">{t("无需认证", "No authentication")}</option>
                      <option value="bearer_env">Bearer Token ({t("环境变量", "environment variable")})</option>
                      <option value="oauth_client_credentials">OAuth client credentials</option>
                    </select>
                  </label>
                )}
                <label className="block space-y-2">
                  <span className="text-xs uppercase tracking-normal text-white/50">{t("允许的工具", "Allowed tools")}</span>
                  <textarea
                    className="theme-input min-h-24 resize-y font-mono"
                    value={server.allowedToolsText}
                    onChange={(event) => onServerChange(index, { allowedToolsText: event.target.value })}
                    placeholder={t("每行一个工具名称", "One tool name per line")}
                  />
                </label>
              </div>

              {server.transport === "stdio" ? (
                <details className="mt-4 rounded-md border border-white/10 px-3 py-2">
                  <summary className="cursor-pointer text-sm font-medium">{t("环境变量引用（可选）", "Environment references (optional)")}</summary>
                  <p className="mt-2 text-xs text-white/50">
                    {t("格式为 CHILD_NAME=HOST_ENV_NAME，每行一项。", "Use CHILD_NAME=HOST_ENV_NAME, one mapping per line.")}
                  </p>
                  <textarea
                    className="theme-input mt-3 min-h-24 resize-y font-mono"
                    value={server.envRefsText}
                    onChange={(event) => onServerChange(index, { envRefsText: event.target.value })}
                    placeholder="API_TOKEN=RUSTCLAW_MCP_TOKEN"
                  />
                </details>
              ) : null}

              {server.transport === "streamable_http" && authMode === "bearer_env" ? (
                <label className="mt-4 block max-w-xl space-y-2">
                  <span className="text-xs uppercase tracking-normal text-white/50">{t("Token 环境变量名", "Token environment variable")}</span>
                  <input
                    className="theme-input font-mono"
                    value={server.authTokenEnv}
                    onChange={(event) => onServerChange(index, { authTokenEnv: event.target.value })}
                    placeholder="RUSTCLAW_MCP_TOKEN"
                    autoComplete="off"
                  />
                </label>
              ) : null}

              {server.transport === "streamable_http" && authMode === "oauth_client_credentials" ? (
                <div className="mt-4 grid gap-4 md:grid-cols-2">
                  <label className="block space-y-2">
                    <span className="text-xs uppercase tracking-normal text-white/50">OAuth client ID env</span>
                    <input
                      className="theme-input font-mono"
                      value={server.oauthClientIdEnv}
                      onChange={(event) => onServerChange(index, { oauthClientIdEnv: event.target.value })}
                      placeholder="RUSTCLAW_MCP_CLIENT_ID"
                      autoComplete="off"
                    />
                  </label>
                  <label className="block space-y-2">
                    <span className="text-xs uppercase tracking-normal text-white/50">OAuth client secret env</span>
                    <input
                      className="theme-input font-mono"
                      value={server.oauthClientSecretEnv}
                      onChange={(event) => onServerChange(index, { oauthClientSecretEnv: event.target.value })}
                      placeholder="RUSTCLAW_MCP_CLIENT_SECRET"
                      autoComplete="off"
                    />
                  </label>
                  <label className="block space-y-2">
                    <span className="text-xs uppercase tracking-normal text-white/50">OAuth scopes</span>
                    <textarea
                      className="theme-input min-h-20 resize-y font-mono"
                      value={server.oauthScopesText}
                      onChange={(event) => onServerChange(index, { oauthScopesText: event.target.value })}
                      placeholder={t("每行一个 scope", "One scope per line")}
                    />
                  </label>
                  <label className="block space-y-2">
                    <span className="text-xs uppercase tracking-normal text-white/50">OAuth resource</span>
                    <input
                      className="theme-input font-mono"
                      value={server.oauthResource}
                      onChange={(event) => onServerChange(index, { oauthResource: event.target.value })}
                      placeholder="https://tools.example.com/mcp"
                    />
                  </label>
                </div>
              ) : null}

              {server.hasStaticEnv || server.hasAdvancedPolicy ? (
                <p className="mt-4 rounded-md border border-sky-400/20 bg-sky-500/10 px-3 py-2 text-xs text-sky-100/80">
                  {t("此服务器还有高级配置；保存时会原样保留。", "This server has advanced settings that remain unchanged when you save.")}
                </p>
              ) : null}

              <div className="mt-4 flex flex-wrap items-center gap-2">
                <button
                  type="button"
                  className="theme-secondary-btn px-3 py-2 text-xs"
                  disabled={!server.originalServerId || runtime?.state !== "ready" || hasUnsavedChanges || testingServerId !== null}
                  onClick={() => server.originalServerId && void onTestServer(server.originalServerId)}
                >
                  {testingServerId === server.originalServerId ? <Loader2 className="h-4 w-4 animate-spin" /> : <RefreshCw className="h-4 w-4" />}
                  {t("测试协议", "Test protocol")}
                </button>
                <span className="text-xs text-white/50">
                  {t("工具数量", "Tools")}: {runtime?.tool_count ?? serverTools.length}
                </span>
                {probe ? (
                  <span className="text-xs text-emerald-200">
                    {t("连接正常", "Connection healthy")} · {probe.latency_ms} ms
                  </span>
                ) : null}
              </div>
              {runtime?.last_error_code ? (
                <p className="mt-3 text-xs text-amber-100">
                  {mcpErrorLabel(runtime.last_error_code, t)} <code className="ml-1 text-white/40">{runtime.last_error_code}</code>
                </p>
              ) : null}
              {serverTools.length > 0 ? (
                <div className="mt-3 flex flex-wrap gap-1.5">
                  {serverTools.slice(0, 12).map((tool) => (
                    <span key={tool.capability} className="rounded-md border border-white/10 bg-white/5 px-2 py-1 font-mono text-[11px] text-white/60">
                      {tool.tool_name}
                    </span>
                  ))}
                </div>
              ) : null}
            </fieldset>
          );
        })}
      </div>

      {hasUnsavedChanges ? (
        <p className="mt-4 text-xs text-amber-100">
          {t("存在未保存的 MCP 修改。协议测试会在保存并重启后可用。", "There are unsaved MCP changes. Protocol testing becomes available after saving and restarting.")}
        </p>
      ) : null}
    </section>
  );
}
