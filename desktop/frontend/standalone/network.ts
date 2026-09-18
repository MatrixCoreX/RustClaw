import { invoke } from "@tauri-apps/api/core";
import { initializeStandaloneRuntime } from "../runtime";
export interface AssetNode { id: string; origin: string }
export interface NodeList { version: number; nodes: AssetNode[]; selected: string }
export interface AssetConnection { id: string; node: AssetNode; response_ms: number; automatic: boolean }
export async function connectNode(id?: string): Promise<AssetConnection> {
  const connection = id
    ? await invoke<AssetConnection>("wallet_connect_node", { nodeId: id })
    : await invoke<AssetConnection>("wallet_prefer_node");
  initializeStandaloneRuntime(connection);
  return connection;
}
export function marketFetch(connection: AssetConnection) {
  return async (path: string, init?: RequestInit) => {
    if (init?.signal?.aborted) throw new DOMException("Aborted", "AbortError");
    if (init?.method && init.method !== "GET") throw new Error("wallet_intent_invalid");
    const data = await invoke<unknown>("wallet_market_read", { sessionId: connection.id, path });
    if (init?.signal?.aborted) throw new DOMException("Aborted", "AbortError");
    return new Response(JSON.stringify(data), { headers: { "Content-Type": "application/json" } });
  };
}
