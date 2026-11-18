import React from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { useMemoryCapacity } from "../hooks/useMemoryCapacity";
import type { MemoryCapacity } from "./memory-capacity";
import type { MemorySettingsResult } from "../types/api";

globalThis.IS_REACT_ACT_ENVIRONMENT = true;
export type Options = Parameters<typeof useMemoryCapacity>[0];
export const t = (zh: string) => zh;
export function settings(owner = "A", revision = 1, conversation: string | null = null): MemorySettingsResult {
  return { schema_version: 1, scope: conversation ? "conversation" : "principal", target_principal_id: owner,
    conversation_id: conversation, requested: { use_mode: "inherit", generate_mode: "inherit", external_context_policy: "exclude" },
    use_memory: true, generate_memory: true, external_context_policy: "exclude", use_source: "principal", generate_source: "principal",
    external_context_source: "principal", managed_deny_reason: null, revision, policy_digest: `policy-${owner}-${revision}-${conversation}`, restart_required: false };
}
export function report(policy = settings()): MemoryCapacity {
  const adminOnly = { status: "admin_only" } as const;
  return { schema_version: 1, sampled_at_ts: 100, settings: policy, capacity_decision: "available",
    quota: { scope: "source_transcript_content_rows_and_bytes", decision: "available", counters: {
      used_rows: "0", max_rows: "100", used_bytes: "0", max_bytes: "1000", spent_and_pending_logical_microunits: "0", max_logical_microunits: "10000",
    } }, pressure: { state: "normal", measurement_scope: "main_sqlite_allocated_pages_not_memory_domain_or_host_free_space",
      observed_at_ts: 100, revision: 1, observed_bytes: adminOnly, configured_soft_limit_bytes: adminOnly },
    measurements: { own_source_transcript_payload_bytes: { status: "measured", bytes: "0" }, main_sqlite_allocated_page_bytes: adminOnly,
      memory_domain_assigned_btree_bytes: adminOnly, sqlite_freelist_page_bytes: adminOnly, database_path_filesystem_available_bytes: adminOnly } };
}
export const json = (data: unknown) => new Response(JSON.stringify({ ok: true, data }));
export function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
export async function mount(apiFetch: Options["apiFetch"], overrides: Partial<Options> = {}) {
  let runtime!: ReturnType<typeof useMemoryCapacity>;
  let renderer!: ReactTestRenderer;
  let options: Options = { requestScope: "session-A", settings: settings(), scope: "principal", conversationId: null, apiFetch, ...overrides };
  const renders: Array<{ requestScope: string; data: MemoryCapacity | null }> = [];
  function Probe(props: Options) {
    runtime = useMemoryCapacity(props);
    renders.push({ requestScope: props.requestScope, data: runtime.data });
    return null;
  }
  await act(async () => { renderer = create(<Probe {...options} />); });
  return { renders, get runtime() { return runtime; },
    async update(next: Partial<Options>) { options = { ...options, ...next }; await act(async () => { renderer.update(<Probe {...options} />); }); },
    async close() { await act(async () => { renderer.unmount(); }); } };
}
