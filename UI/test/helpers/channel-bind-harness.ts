import React from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import type { TestContext } from "node:test";
import QRCode from "qrcode";
import { UiDialogProvider } from "../../src/components/UiDialogProvider";
import { useChannelBindRuntime, type UseChannelBindRuntimeParams } from "../../src/hooks/useFeishuBindRuntime";
import type { AgentAppChannel, FeishuBindSessionResponse } from "../../src/lib/feishu-bind";

type Runtime = ReturnType<typeof useChannelBindRuntime>;
type Props = UseChannelBindRuntimeParams;
export function bindSession(platform: AgentAppChannel, id = 1, status = "pending"): FeishuBindSessionResponse {
  return { session_id: id, channel: platform, status, bind_token: "fixture-only", created_at: "1793361600",
    updated_at: "1793361600", expires_at: "1793362200", entry_url: `https://bind.example.test/${platform}/${id}`, poll_interval_seconds: 1 };
}

export async function channelBindHarness(context: TestContext, platform: AgentAppChannel = "feishu") {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  context.mock.timers.enable({ apis: ["setTimeout"] });
  const windowDescriptor = Object.getOwnPropertyDescriptor(globalThis, "window");
  Object.defineProperty(globalThis, "window", { configurable: true, value: {
    setTimeout: (fn: () => void, ms: number) => setTimeout(fn, ms), clearTimeout: (id: ReturnType<typeof setTimeout>) => clearTimeout(id),
  } });
  const pending: { path: string; init?: RequestInit; scope: string; resolve: (response: Response) => void; reject: (error: Error) => void }[] = [];
  const encoders: { input: string; resolve: (value: string) => void; reject: (error: Error) => void }[] = [];
  context.mock.method(QRCode, "toDataURL", (input: string) => new Promise<string>((resolve, reject) => encoders.push({ input, resolve, reject })));
  const reads: { scope: string; platform: AgentAppChannel; kind: string }[] = [];
  const configGates = new Map<string, Promise<void>>();
  let failConfig = false, failHealth = false;
  let props!: Props, current!: Runtime, renderer!: ReactTestRenderer, disposed = false;
  const transport = (scope: string, channel: AgentAppChannel) => ({
    apiFetch: (path: string, init?: RequestInit) => new Promise<Response>((resolve, reject) => pending.push({ path, init, scope, resolve, reject })),
    onConfigRefresh: async () => { reads.push({ scope, platform: channel, kind: "config" }); await configGates.get(scope); if (failConfig) throw new Error("fixture_config_unavailable"); },
    onHealthRefresh: async () => { reads.push({ scope, platform: channel, kind: "health" }); if (failHealth) throw new Error("fixture_health_unavailable"); },
  });
  props = { requestScope: "device-a:admin:epoch-0", isAdminIdentity: true, uiAuthReady: true, platform, t: (_zh, en) => en,
    ...transport("device-a:admin:epoch-0", platform) };
  function Probe() { current = useChannelBindRuntime(props); return null; }
  const element = () => React.createElement(UiDialogProvider, null, React.createElement(Probe));
  await act(async () => { renderer = create(element()); });
  return {
    get current() { return current; }, pending, encoders, reads,
    async update(change: Partial<Props>) {
      props = { ...props, ...transport(change.requestScope ?? props.requestScope, change.platform ?? props.platform), ...change };
      await act(async () => { renderer.update(element()); });
    },
    async begin(runtime = current) {
      const index = pending.length; let completion!: Promise<void>;
      await act(async () => { completion = runtime.beginBind(); });
      return { index, completion };
    },
    async reply(index: number, data: unknown, status = 200, error = "fixture_unavailable") {
      await act(async () => { pending[index].resolve(new Response(JSON.stringify({ ok: status === 200, data, error: status === 200 ? null : error }),
        { status, headers: { "Content-Type": "application/json" } })); });
    },
    async reject(index: number) { await act(async () => { pending[index].reject(new TypeError("fixture_network_unavailable")); }); },
    async heldBody(index: number) {
      let resolve!: (value: unknown) => void; const body = new Promise<unknown>(done => { resolve = done; });
      await act(async () => { pending[index].resolve(Object.assign(new Response(null), { json: () => body })); });
      return async (data: unknown) => { await act(async () => { resolve({ ok: true, data, error: null }); }); };
    },
    async qr(index: number, value = "data:image/png;base64,fixture-only") { await act(async () => { encoders[index].resolve(value); }); },
    async qrError(index: number) { await act(async () => { encoders[index].reject(new Error("fixture_encoder_unavailable")); }); },
    async advance(ms = 1000) { await act(async () => { context.mock.timers.tick(ms); }); },
    async load(id = 1, status = "pending") { const request = await this.begin(); await this.reply(request.index, bindSession(props.platform, id, status)); await request.completion; },
    failRefresh(config = true, health = false) { failConfig = config; failHealth = health; },
    pauseConfig() {
      let release!: () => void; configGates.set(props.requestScope, new Promise<void>(resolve => { release = resolve; }));
      return async () => { await act(async () => { release(); }); };
    },
    async dispose() {
      if (disposed) return; disposed = true;
      await act(async () => { renderer.unmount(); });
      if (windowDescriptor) Object.defineProperty(globalThis, "window", windowDescriptor); else Reflect.deleteProperty(globalThis, "window");
    },
  };
}
