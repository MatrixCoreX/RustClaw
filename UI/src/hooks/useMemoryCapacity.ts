import { useCallback, useEffect, useRef, useState } from "react";
import { capacitySelection, parseMemoryCapacity, type MemoryCapacity } from "../lib/memory-capacity";
import type { MemorySettingsResult } from "../types/api";
import { useRequestOwner } from "./useRequestOwner";

interface Options {
  requestScope: string;
  settings: MemorySettingsResult | null;
  scope: "principal" | "conversation";
  conversationId: string | null;
  apiFetch: (path: string, init?: RequestInit) => Promise<Response>;
}
type View = { owner: () => boolean; loading: boolean; failed: boolean; data: MemoryCapacity | null };

export function useMemoryCapacity({ requestScope, settings, scope, conversationId, apiFetch }: Options) {
  const selected = capacitySelection(settings, scope, conversationId);
  const key = selected?.key ?? "";
  const path = selected?.path ?? "";
  const owns = useRequestOwner(requestScope, key);
  const sequence = useRef(0);
  const pending = useRef<AbortController | null>(null);
  const [stored, setStored] = useState<View | null>(null);
  const refresh = useCallback(async () => {
    if (!owns() || !key) return;
    const ticket = ++sequence.current;
    pending.current?.abort();
    const controller = new AbortController();
    pending.current = controller;
    const current = () => owns() && sequence.current === ticket && !controller.signal.aborted;
    setStored({ owner: owns, loading: true, failed: false, data: null });
    try {
      const response = await apiFetch(path, { signal: controller.signal });
      const reply: unknown = await response.json();
      const envelope = reply as { ok?: unknown; data?: unknown } | null;
      const data = response.ok && envelope?.ok === true ? parseMemoryCapacity(envelope.data, key) : null;
      if (current()) setStored({ owner: owns, loading: false, failed: !data, data });
    } catch {
      if (current()) setStored({ owner: owns, loading: false, failed: true, data: null });
    }
  }, [apiFetch, key, owns, path]);
  useEffect(() => {
    void refresh();
    return () => { pending.current?.abort(); };
  }, [refresh]);
  // Hide old observations during render, not only after effect cleanup.
  const view = stored?.owner === owns && owns() && key ? stored : null;
  return { data: view?.data ?? null, loading: view?.loading ?? false, failed: view?.failed ?? false,
    selectable: Boolean(key) && owns(), refresh };
}
