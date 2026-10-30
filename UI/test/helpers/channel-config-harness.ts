import React from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { useChannelConfigRuntime, type UseChannelConfigRuntimeParams } from "../../src/hooks/useChannelConfigRuntime";
import type { FeishuConfigResponse, TelegramConfigResponse, WechatConfigResponse } from "../../src/types/api";

export type Channel = "wechat" | "feishu" | "lark" | "telegram";
export const channels: Channel[] = ["wechat", "feishu", "lark", "telegram"];
type Runtime = ReturnType<typeof useChannelConfigRuntime>;
type Props = UseChannelConfigRuntimeParams & { requestScope: string; isAdminIdentity: boolean };
export const configKeys = {
  wechat: { read: "fetchWechatConfig", data: "wechatConfigData", loading: "wechatConfigLoading", error: "wechatConfigError" },
  feishu: { read: "fetchFeishuConfig", data: "feishuConfigData", loading: "feishuConfigLoading", error: "feishuConfigError" },
  lark: { read: "fetchLarkConfig", data: "larkConfigData", loading: "larkConfigLoading", error: "larkConfigError" },
  telegram: { read: "fetchTelegramConfig", data: "telegramConfigData", loading: "telegramConfigLoading", error: "telegramConfigError" },
} as const;

export function channelConfig(channel: Channel, label = "device-a"): WechatConfigResponse | FeishuConfigResponse | TelegramConfigResponse {
  if (channel === "telegram") return {
    config_path: label, restart_required: true, agents: [],
    bots: [{ name: "primary", bot_token: "", bot_token_configured: true, bot_token_masked: label,
      agent_id: "main", allowlist: [], access_mode: "public", allowed_telegram_usernames: [], is_primary: true }],
  };
  const common = { config_path: label, enabled: false, listen: "127.0.0.1:8792",
    clawd_base_url: "http://127.0.0.1:8787", api_base_url: "https://example.test", restart_required: true };
  if (channel === "wechat") return { ...common, wechat_uin_base64: "", request_timeout_seconds: 30,
    longpoll_timeout_ms: 35_000, text_chunk_chars: 1200, bot_token_configured: false, saved_session_present: false };
  return { ...common, mode: "websocket", app_id: "", app_secret: "", verification_token_configured: false,
    encrypt_key_configured: false, bind_ready: false, current_key_bound: false };
}

export async function channelConfigHarness() {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  const pending: { path: string; init?: RequestInit; scope: string; resolve: (response: Response) => void; reject: (error: Error) => void }[] = [];
  let current!: Runtime, props!: Props, renderer!: ReactTestRenderer, disposed = false;
  const transport = (scope: string) => (path: string, init?: RequestInit) =>
    new Promise<Response>((resolve, reject) => pending.push({ path, init, scope, resolve, reject }));
  props = { requestScope: "device-a:admin:epoch-0", isAdminIdentity: true,
    apiFetch: transport("device-a:admin:epoch-0"), t: (_zh, en) => en };
  function Probe() { current = useChannelConfigRuntime(props); return null; }
  await act(async () => { renderer = create(React.createElement(Probe)); });
  return {
    get current() { return current; }, pending,
    async update(change: Partial<Props>) {
      props = { ...props, ...(change.requestScope !== undefined ? { apiFetch: transport(change.requestScope) } : {}), ...change };
      await act(async () => { renderer.update(React.createElement(Probe)); });
    },
    async read(channel: Channel, runtime = current) {
      const index = pending.length;
      let completion!: Promise<void>;
      await act(async () => { completion = runtime[configKeys[channel].read](); });
      return { index, completion };
    },
    async enable(value = true, runtime = current) {
      const index = pending.length;
      let completion!: Promise<boolean>;
      await act(async () => { completion = runtime.setWechatEnabled(value); });
      return { index, completion };
    },
    async edit(value: string, runtime = current) {
      await act(async () => { runtime.setTelegramPrimaryBotDraftField("name", value); });
    },
    async save(runtime = current) {
      const index = pending.length;
      let completion!: Promise<void>;
      await act(async () => { completion = runtime.saveTelegramConfig(); });
      return { index, completion };
    },
    async reply(index: number, data: unknown, status = 200) {
      await act(async () => { pending[index].resolve(new Response(JSON.stringify({ ok: status === 200, data, error: status === 200 ? null : "fixture_request_failed" }),
        { status, headers: { "Content-Type": "application/json" } })); });
    },
    async heldBody(index: number) {
      let resolve!: (body: unknown) => void;
      const body = new Promise<unknown>(done => { resolve = done; });
      await act(async () => { pending[index].resolve(Object.assign(new Response(null), { json: () => body })); });
      return async (data: unknown) => { await act(async () => { resolve({ ok: true, data, error: null }); }); };
    },
    async reject(index: number) { await act(async () => { pending[index].reject(new Error("fixture_request_failed")); }); },
    async load(channel: Channel, label = "device-a") {
      const read = await this.read(channel);
      await this.reply(read.index, channelConfig(channel, label));
      await read.completion;
    },
    async dispose() {
      if (disposed) return;
      disposed = true;
      await act(async () => { renderer.unmount(); });
    },
  };
}
