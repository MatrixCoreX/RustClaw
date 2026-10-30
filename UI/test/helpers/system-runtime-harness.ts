import React from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { UiDialogProvider } from "../../src/components/UiDialogProvider";
import { useSystemRuntime, type UseSystemRuntimeParams } from "../../src/hooks/useSystemRuntime";

type PendingRequest = {
  path: string;
  resolve: (response: Response) => void;
  reject: (error: Error) => void;
};
export type SystemRead = "summary" | "dependencies" | "workspace" | "nginx" | "webd" | "hardware";

export async function systemRuntimeHarness() {
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  const oldWindow = Object.getOwnPropertyDescriptor(globalThis, "window");
  const oldDocument = Object.getOwnPropertyDescriptor(globalThis, "document");
  const intervals = new Map<number, () => void>();
  const timeouts = new Map<number, () => void>();
  let timerId = 0, reloads = 0, healthReads = 0;
  Object.defineProperty(globalThis, "window", { configurable: true, value: {
    setInterval(callback: () => void) { intervals.set(++timerId, callback); return timerId; },
    clearInterval(id: number) { intervals.delete(id); },
    setTimeout(callback: () => void) { timeouts.set(++timerId, callback); return timerId; },
    clearTimeout(id: number) { timeouts.delete(id); },
    location: { reload() { reloads++; } },
  } });
  Object.defineProperty(globalThis, "document", { configurable: true, value: {
    visibilityState: "visible", addEventListener() {}, removeEventListener() {},
  } });
  const pending: PendingRequest[] = [];
  const noop = () => undefined;
  let props: UseSystemRuntimeParams = {
    apiFetch: (path) => new Promise<Response>((resolve, reject) => pending.push({ path, resolve, reject })),
    t: (zh) => zh, apiBase: "http://localhost:8788", uiAuthReady: true,
    isAdminIdentity: false, currentPage: "chat",
    fetchHealth: async () => { healthReads++; }, setHealth: noop, setError: noop,
    clearLlmConfigError: noop, clearSkillsConfigError: noop, fetchLlmConfig: noop,
    fetchMultimodalConfig: noop, fetchSkillsConfig: noop, fetchSkills: noop,
  };
  let current!: ReturnType<typeof useSystemRuntime>;
  function Probe() { current = useSystemRuntime(props); return null; }
  const element = () => React.createElement(UiDialogProvider, null, React.createElement(Probe));
  let renderer!: ReactTestRenderer;
  let disposed = false;
  await act(async () => { renderer = create(element()); });
  return {
    get current() { return current; },
    get reloads() { return reloads; },
    get reloadTimers() { return timeouts.size; },
    get pollTimers() { return intervals.size; },
    get healthReads() { return healthReads; },
    pending,
    async update(change: Partial<UseSystemRuntimeParams>) {
      props = { ...props, ...change };
      await act(async () => { renderer.update(element()); });
    },
    async start(kind: SystemRead, silent = false) {
      const index = pending.length;
      await act(async () => {
        switch (kind) {
          case "summary": void current.fetchHostSystemSummary(); break;
          case "dependencies": void current.fetchHostDependencies(silent); break;
          case "workspace": void current.fetchWorkspaceUpdateStatus(silent); break;
          case "nginx": void current.fetchNginxStatus(silent); break;
          case "webd": void current.fetchWebdExposureStatus(silent); break;
          case "hardware": break;
        }
      });
      if (kind === "hardware") {
        props = { ...props, currentPage: "chat" };
        await act(async () => { renderer.update(element()); });
        props = { ...props, currentPage: "dashboard" };
        await act(async () => { renderer.update(element()); });
        const hardware = pending.findIndex((request, position) => position >= index
          && request.path === "/v1/pi-app/status");
        return hardware === -1 ? index : hardware;
      }
      return index;
    },
    async respond(index: number, data: unknown, status = 200) {
      await act(async () => {
        pending[index].resolve(new Response(JSON.stringify({ ok: status === 200, data, error: null }), {
          status, headers: { "Content-Type": "application/json" },
        }));
      });
    },
    async reject(index: number) {
      await act(async () => { pending[index].reject(new Error("fixture transport failure")); });
    },
    async respondRaw(index: number, value: string, contentType: string) {
      await act(async () => {
        pending[index].resolve(new Response(value, { headers: { "Content-Type": contentType } }));
      });
    },
    async poll() {
      await act(async () => { for (const callback of [...intervals.values()]) callback(); });
    },
    async flushTimeouts() {
      await act(async () => {
        const callbacks = [...timeouts.values()]; timeouts.clear();
        for (const callback of callbacks) callback();
      });
    },
    async dispose() {
      if (disposed) return;
      disposed = true;
      await act(async () => { renderer.unmount(); });
      for (const [key, descriptor] of [["window", oldWindow], ["document", oldDocument]] as const) {
        if (descriptor) Object.defineProperty(globalThis, key, descriptor);
        else Reflect.deleteProperty(globalThis, key);
      }
    },
  };
}
