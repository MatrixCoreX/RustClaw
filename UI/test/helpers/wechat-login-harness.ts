import React from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import type { TestContext } from "node:test";
import { useWechatRuntime, type UseWechatRuntimeParams } from "../../src/hooks/useWechatRuntime";

type Runtime = ReturnType<typeof useWechatRuntime>;
type Props = UseWechatRuntimeParams & { requestScope: string };
export const loginStatus = (label = "current") => ({ connected: false, qr_ready: false, account_label: label, status: "unbound" });
export const qrStart = (label = "current") => ({ session_key: label, qrcode_url: `https://qr.example.test/${label}` });

export async function wechatLoginHarness(context: TestContext) {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  context.mock.timers.enable({ apis: ["setInterval", "setTimeout"] });
  const windowDescriptor = Object.getOwnPropertyDescriptor(globalThis, "window");
  const documentDescriptor = Object.getOwnPropertyDescriptor(globalThis, "document");
  const document = Object.assign(new EventTarget(), { visibilityState: "hidden" });
  Object.defineProperty(globalThis, "document", { configurable: true, value: document });
  Object.defineProperty(globalThis, "window", { configurable: true, value: {
    setInterval: (fn: () => void, ms: number) => setInterval(fn, ms), clearInterval: (id: ReturnType<typeof setInterval>) => clearInterval(id),
  } });
  const pending: { path: string; init?: RequestInit; scope: string; resolve: (response: Response) => void; reject: (error: Error) => void }[] = [];
  let props!: Props, current!: Runtime, renderer!: ReactTestRenderer, disposed = false;
  const transport = (scope: string) => (path: string, init?: RequestInit) =>
    new Promise<Response>((resolve, reject) => pending.push({ path, init, scope, resolve, reject }));
  props = { requestScope: "device-a:admin:epoch-0", apiBase: "https://device-a.example.test", uiAuthReady: true,
    enabled: true, serviceHealthy: true, statusPollingEnabled: true, apiFetch: transport("device-a:admin:epoch-0"), t: (_zh, en) => en };
  function Probe() { current = useWechatRuntime(props); return null; }
  await act(async () => { renderer = create(React.createElement(Probe)); });
  return {
    get current() { return current; }, pending,
    async update(change: Partial<Props>) {
      props = { ...props, ...(change.requestScope !== undefined ? { apiFetch: transport(change.requestScope) } : {}), ...change };
      await act(async () => { renderer.update(React.createElement(Probe)); });
    },
    async read(silent = false, runtime = current) {
      const index = pending.length;
      let completion!: Promise<void>;
      await act(async () => { completion = runtime.fetchWechatLoginStatus(silent); });
      return { index, completion };
    },
    async start(force = true, runtime = current) {
      const index = pending.length;
      let completion!: Promise<void>;
      await act(async () => { completion = runtime.startWechatQrLogin(force); });
      return { index, completion };
    },
    async reply(index: number, data: unknown, error: string | null = null) {
      await act(async () => { pending[index].resolve(new Response(JSON.stringify({ ok: !error, data, error }),
        { status: error ? 409 : 200, headers: { "Content-Type": "application/json" } })); });
    },
    async heldBody(index: number) {
      let resolve!: (body: unknown) => void;
      const body = new Promise<unknown>(done => { resolve = done; });
      await act(async () => { pending[index].resolve(Object.assign(new Response(null), { json: () => body })); });
      return async (data: unknown) => { await act(async () => { resolve({ ok: true, data, error: null }); }); };
    },
    async reject(index: number) { await act(async () => { pending[index].reject(new Error("fixture_unavailable")); }); },
    async advance(ms: number) { await act(async () => { context.mock.timers.tick(ms); }); },
    async visibility(value: "visible" | "hidden") {
      await act(async () => { document.visibilityState = value; document.dispatchEvent(new Event("visibilitychange")); });
    },
    async dispose() {
      if (disposed) return;
      disposed = true;
      await act(async () => { renderer.unmount(); });
      if (windowDescriptor) Object.defineProperty(globalThis, "window", windowDescriptor); else Reflect.deleteProperty(globalThis, "window");
      if (documentDescriptor) Object.defineProperty(globalThis, "document", documentDescriptor); else Reflect.deleteProperty(globalThis, "document");
    },
  };
}
