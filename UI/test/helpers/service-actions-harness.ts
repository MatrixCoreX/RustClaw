import React from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import type { TestContext } from "node:test";
import { UiDialogProvider } from "../../src/components/UiDialogProvider";
import { useServiceActionsRuntime, type UseServiceActionsRuntimeParams } from "../../src/hooks/useServiceActionsRuntime";

type Runtime = ReturnType<typeof useServiceActionsRuntime>;
type Service = Parameters<Runtime["controlService"]>[0];
type Action = Parameters<Runtime["controlService"]>[1];
type ScopeParams = UseServiceActionsRuntimeParams;
export async function serviceActionsHarness(context: TestContext) {
  context.mock.timers.enable({ apis: ["setTimeout"] });
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  const pending: { path: string; init?: RequestInit; scope: string; resolve: (response: Response) => void; reject: (error: Error) => void }[] = [];
  const reads: string[] = [];
  let props!: ScopeParams, current!: Runtime, renderer!: ReactTestRenderer, disposed = false;
  const transport = (scope: string) => ({
    apiFetch: (path: string, init?: RequestInit) => new Promise<Response>((resolve, reject) => pending.push({ path, init, scope, resolve, reject })),
    onHealthRefresh: async () => { reads.push(`${scope}:health`); },
    onConfigRefresh: async () => { reads.push(`${scope}:config`); },
  });
  props = { requestScope: "device-a:admin:epoch-0", isAdminIdentity: true, t: (_zh, en) => en, ...transport("device-a:admin:epoch-0") };
  function Probe() { current = useServiceActionsRuntime(props); return null; }
  const element = () => React.createElement(UiDialogProvider, null, React.createElement(Probe));
  await act(async () => { renderer = create(element()); });
  return {
    get current() { return current; }, pending, reads,
    async update(change: Partial<ScopeParams>) {
      props = { ...props, ...(change.requestScope !== undefined ? transport(change.requestScope) : {}), ...change };
      await act(async () => { renderer.update(element()); });
    },
    async start(service: Service = "wechatd", action: Action = "start", runtime = current) {
      const index = pending.length;
      let completion!: Promise<boolean>;
      await act(async () => { completion = runtime.controlService(service, action); });
      return { index, completion };
    },
    async reply(index: number, status = 200, body: unknown = { ok: true, data: {}, error: null }) {
      await act(async () => { pending[index].resolve(new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })); });
    },
    async reject(index: number) {
      await act(async () => { pending[index].reject(new Error("fixture_request_failed")); });
    },
    async advance(ms = 800) { await act(async () => { context.mock.timers.tick(ms); }); },
    async dispose() {
      if (disposed) return;
      disposed = true;
      await act(async () => { renderer.unmount(); });
    },
  };
}
