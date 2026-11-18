import assert from "node:assert/strict";
import React from "react";
import { createHash } from "node:crypto";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { UiDialogProvider, useUiDialog, type UiConfirmOptions } from "../components/UiDialogProvider";
import { useMemoryRuntime, type MemoryListFilters } from "./useMemoryRuntime";

globalThis.IS_REACT_ACT_ENVIRONMENT = true;
export type ApiFetch = (path: string, init?: RequestInit) => Promise<Response>;
export type Runtime = ReturnType<typeof useMemoryRuntime>;
export const t = (zh: string) => zh;
export const filters = (search = ""): MemoryListFilters => ({
  search, scope: "", origin: "", kind: "", status: "", freshness: "",
});

export function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

export function json(data: unknown) {
  return new Response(JSON.stringify({ ok: true, data }), { status: 200 });
}

export function settings(owner: string, revision = 1, conversation: string | null = null) {
  return {
    schema_version: 1, scope: conversation ? "conversation" : "principal",
    target_principal_id: owner, conversation_id: conversation,
    requested: { use_mode: "inherit", generate_mode: "inherit", external_context_policy: "exclude" },
    use_memory: true, generate_memory: true, external_context_policy: "exclude",
    use_source: "principal", generate_source: "principal", external_context_source: "principal",
    managed_deny_reason: null, revision, policy_digest: `policy-${owner}-${revision}`, restart_required: false,
  };
}

export function page(owner: string, number = 1) {
  return { schema_version: 1, page: number, page_size: 20, total: 1, has_more: false, items: [{
    id: `memory-${owner}`, revision: 1, kind: "fact", scope_kind: "principal", origin: "user_confirmed",
    status: "active", content: owner, source: "user", evidence_available: true, trust_tier: "user_confirmed",
    updated_at_ts: 1, freshness: "fresh",
  }] };
}

function importPreview() {
  return { schema_version: 1, import_id: "fixture-import", payload_digest: `sha256:${"0".repeat(64)}`,
    accepted_items: 1, skipped_items: 0, duplicate_items: 0, trust_tier: "imported_legacy", scope_kind: "principal" };
}

export function data(path: string, owner: string, method?: string) {
  const url = new URL(path, "https://fixture.invalid");
  if (url.pathname === "/v1/memory/import/policy") return {
    schema_version: 1, protocol: "memory-import-upload-v1", max_file_bytes: "10485760", max_chunk_bytes: "1048576",
    max_value_bytes: "1048576", file_limit_scope: "memory_domain_budget",
  };
  if (url.pathname.startsWith("/v1/memory/import/uploads")) {
    if (url.pathname.endsWith("/preview")) return importPreview();
    return { schema_version: 1, upload_id: "fixture-upload", file_digest: `sha256:${createHash("sha256").update("{}").digest("hex")}`,
      total_bytes: "2", received_bytes: url.pathname.endsWith("/chunks") ? "2" : "0", chunk_bytes: "1048576", revision: "1",
      state: method === "DELETE" ? "cancelled" : "uploading" };
  }
  switch (url.pathname) {
    case "/v1/memory": return {
      long_term_enabled: true, hybrid_recall_enabled: true,
      counts: { recent: 1, preferences: 1, facts_active: 1, facts_total: 1, long_term_summaries: 1 },
    };
    case "/v1/memory/settings": return settings(owner, 1, url.searchParams.get("conversation_id"));
    case "/v1/memory/preferences": return [{ id: `preference-${owner}`, key: "fixture", value: owner,
      confidence: 1, source: "user", updated_at_ts: 1 }];
    case "/v1/memory/facts": return [{ id: `fact-${owner}`, namespace: "fixture", fact_key: "fixture",
      fact_value: owner, fact_text: owner, confidence: 1, source_kind: "user", source_ref: "fixture",
      reason: "fixture", updated_at_ts: 1, status: "active" }];
    case "/v1/memory/recent": return [{ id: `recent-${owner}`, role: "user", memory_type: "transcript",
      content: owner, created_at_ts: 1, safety_flag: "safe" }];
    case "/v1/memory/items": return page(owner, Number(url.searchParams.get("page") || 1));
    case "/v1/memory/remote-disclosure": return { schema_version: 1, consent_state: "exclude",
      extraction_provider: "fixture", extraction_model: "fixture", consolidation_provider: "fixture",
      consolidation_model: "fixture", extraction_sends: [], embedding_sends: [], withdrawal_effect: "stop" };
    case "/v1/memory/vector/status": return { schema_version: 1, provider_location: "local", state: "ready",
      policy_digest: settings(owner).policy_digest,
      active_generation: 1, queued_jobs: 0, running_jobs: 0, failed_jobs: 0, indexed_rows: 1, remote_consent: "exclude" };
    case "/v1/memory/import/preview": return importPreview();
    case "/v1/memory/import/confirm": return { schema_version: 1, import_id: "fixture-import",
      status: "confirmed", imported_items: 1, existing_items: 0 };
    case "/v1/memory/clear/preview": return { schema_version: 1, mode: "transcript",
      transcript_rows: 1, derived_rows: 0, pending_jobs: 0 };
    default: return { schema_version: 1, status: "ok", memory_id: "fixture-memory",
      revision: 2, revision_id: "fixture-undo", queued_rows: 1, generation: 2 };
  }
}

export function backend(owner: () => string): ApiFetch {
  return async (path, init) => json(data(path, owner(), init?.method));
}

export async function mount(apiFetch: ApiFetch, initialScope = "A",
  confirm: (options: UiConfirmOptions) => Promise<boolean> = async () => true,
  translate: (zh: string, en: string) => string = t) {
  let runtime!: Runtime;
  let renderer!: ReactTestRenderer;
  const renders: Array<{ scope: string; settings: Runtime["memorySettings"]; page: Runtime["memoryPage"] }> = [];
  function Probe({ scope }: { scope: string }) {
    // Only this fixture's provider service is replaced. No browser/portal or
    // production confirmation is touched, and pending approval is controllable.
    const dialog = useUiDialog();
    dialog.confirm = confirm;
    const parameters = { apiFetch, t: translate, requestScope: scope };
    runtime = useMemoryRuntime(parameters);
    renders.push({ scope, settings: runtime.memorySettings, page: runtime.memoryPage });
    return null;
  }
  const tree = (scope: string) => React.createElement(UiDialogProvider, {
    children: React.createElement(Probe, { scope }),
  });
  await act(async () => { renderer = create(tree(initialScope)); });
  return {
    renders,
    get runtime() { return runtime; },
    async scope(scope: string) { await act(async () => { renderer.update(tree(scope)); }); },
    async close() { await act(async () => { renderer.unmount(); }); },
  };
}

export async function begin(action: () => Promise<unknown>) {
  let pending!: Promise<unknown>;
  await act(async () => { pending = action(); });
  return { pending };
}

export function assertOwner(runtime: Runtime, owner: string) {
  assert.equal(runtime.memorySettings?.target_principal_id, owner);
  assert.equal(runtime.memoryPreferences[0]?.value, owner);
  assert.equal(runtime.memoryFacts[0]?.fact_text, owner);
  assert.equal(runtime.memoryRecent[0]?.content, owner);
  assert.equal(runtime.memoryPage?.items[0]?.content, owner);
}
